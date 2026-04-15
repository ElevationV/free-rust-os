#![allow(dead_code)]
#![allow(static_mut_refs)]

use crate::rtos::kernel::{
    list::List,
    task::{TCB, TaskState},
    types::{TickType, UBaseType},
    types::PORT_MAX_DELAY,
};
use crate::rtos::kernel::scheduler::{
    CURRENT_TCB, TICK_COUNT,
    CURRENT_DELAY_LIST, OVERFLOW_DELAY_LIST,
    READY_LISTS,
    abort_delay, record_ready_priority, clear_ready_priority,
};
use crate::rtos::port;

pub struct Semaphore {
    count: UBaseType,
    max_count: UBaseType,
    // Tasks waiting to take this semaphore, sorted by priority descending.
    // event_list_item.value = PORT_MAX_DELAY - priority
    // so list.insert() keeps highest-priority task at the head. 
    wait_list: List<TCB>,
}

impl Semaphore {
    // binary semaphore: max = 1, initial count = 1 (available).
    pub const fn new_binary() -> Self {
        Semaphore {
            count: 1,
            max_count: 1,
            wait_list: List::new(),
        }
    }

    // Counting semaphore.
    // `initial` is clamped to `max` if it exceeds it.
    pub const fn new_counting(max: UBaseType, initial: UBaseType) -> Self {
        let count = if initial > max { max } else { initial };
        Semaphore {
            count,
            max_count: max,
            wait_list: List::new(),
        }
    }

    pub fn init(&mut self) {
        self.wait_list.init();
    }


    // FreeRTOS xSemaphoreTake flow:
    //
    //  loop {
    //    enter critical
    //    if count > 0 {
    //      count -= 1
    //      exit critical
    //      return true
    //    }
    //    if timeout == 0 {
    //      exit critical
    //      return false
    //    }
    //    // place current task on wait_list AND delay list
    //    place_on_event_list(timeout)
    //    exit critical
    //    trigger PendSV  ← yields here, resumes below after being woken
    //    // woken by give() or by tick timeout
    //    if timed out { return false }
    //    // else loop and try to take again
    //  }

    /// Try to take the semaphore.
    /// Blocks up to `timeout` ticks. Use `PORT_MAX_DELAY` to wait forever.
    /// Returns `true` if taken, `false` if timed out.
    pub unsafe fn take(&mut self, timeout: TickType) -> bool {
        // record the entry tick so we can track elapsed time across spurious wakeups.
        let entry_tick = TICK_COUNT;
        // remaining ticks we are still willing to wait.
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            // fast path: semaphore is available.
            if self.count > 0 {
                self.count -= 1;
                port::exit_critical();
                return true;
            }

            // no count available and caller does not want to wait.
            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            // place current task on both the semaphore wait_list and delay list.
            self.place_on_event_list(remaining);

            port::exit_critical();

            // execution resumes here after give() or timeout.
            port::trigger_pendsv();
            port::instruction_sync();

            // check whether we were woken by a timeout or by give().
            // give() clears event_list_item.ctner before moving the task to
            // the ready list, so a non-null ctner means we timed out.
            let timed_out = !(*CURRENT_TCB).event_list_item.ctner.is_null();
            if timed_out {
                // Remove ourselves from the wait_list (we are no longer waiting).
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            // woken by give() — loop back and re-try taking the count.
            // recalculate remaining time to handle spurious wakeups correctly.
            if timeout == PORT_MAX_DELAY {
                // keep remaining as PORT_MAX_DELAY.
            } else {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout {
                    return false; // ran out of time between give() and re-take
                }
                remaining = timeout - elapsed;
            }
        }
    }


    // FreeRTOS xSemaphoreGive flow:
    //
    //  enter critical
    //  if wait_list non-empty {
    //    remove highest-priority waiter from wait_list AND delay list
    //    move waiter to ready list
    //    if waiter.priority >= current.priority { trigger PendSV }
    //  } else {
    //    if count < max_count { count += 1 }
    //    else { return false }   // overflow
    //  }
    //  exit critical
    //  return true

    /// release the semaphore.
    /// wakes the highest-priority waiting task, or increments count if none.
    /// returns `false` if count would exceed `max_count` (only relevant for
    /// counting semaphores with no waiters).
    pub unsafe fn give(&mut self) -> bool {
        port::enter_critical();

        let result = if self.wait_list.items_num > 0 {
            // wake the highest-priority waiter
            self.remove_from_event_list();
            true
        } else if self.count < self.max_count {
            self.count += 1;
            true
        } else {
            // count would overflow max_count
            false
        };

        port::exit_critical();
        result
    }

    unsafe fn place_on_event_list(&mut self, timeout: TickType) {
        let tcb = CURRENT_TCB;

        // insert into wait_list by priority
        // Lower value = higher priority in list.insert() ordering
        let priority = (*tcb).priority;
        (*tcb).event_list_item.value = PORT_MAX_DELAY - priority;
        self.wait_list.insert(&raw mut (*tcb).event_list_item);

        // insert into delay_list by wake_time
        let (wake_time, overflowed) = TICK_COUNT.overflowing_add(timeout);
        (*tcb).ticks_to_delay = wake_time;
        (*tcb).state_list_item.value = wake_time;

        // remove from ready list first
        (*tcb).state_list_item.remove_in_list();
        if READY_LISTS[priority as usize].items_num == 0 {
            clear_ready_priority(priority);
        }

        if overflowed {
            (*OVERFLOW_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        } else {
            (*CURRENT_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        }

        (*tcb).state = TaskState::Delayed;
    }

    unsafe fn remove_from_event_list(&mut self) {
        // head of wait_list = highest priority waiter
        let end_ptr = &raw mut self.wait_list.list_end;
        let item = (*end_ptr).next;
        let tcb = (*item).owner as *mut TCB;

        // remove from wait_list
        (*item).remove_in_list();

        // remove from delay list
        if (*tcb).state == TaskState::Delayed {
            (*tcb).state_list_item.remove_in_list();
        }

        // move to ready list
        let priority = (*tcb).priority;
        (*tcb).state_list_item.value = priority as TickType;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
        (*tcb).state = TaskState::Ready;

        // preempt if the woken task has higher or equal priority
        if priority >= (*CURRENT_TCB).priority {
            port::trigger_pendsv();
        }
    }
}