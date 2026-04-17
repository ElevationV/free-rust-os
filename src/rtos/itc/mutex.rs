#![allow(dead_code)]
#![allow(static_mut_refs)]

//  Mutex — binary semaphore + priority inheritance
//
//  Differences from a plain binary semaphore:
//
//  1. Ownership tracking
//     `owner` records which TCB currently holds the mutex.
//     Only the owner may call give()
//
//  2. Priority inheritance
//     When task B (low priority) holds the mutex and task A (high priority) blocks on take(), 
//     B's runtime priority is raised to A's priority so that B is not unfairly preempted by medium-priority tasks 
//     while it holds the resource A is waiting for.  
//     When B releases the mutex, its priority is restored to base_priority.
//
//     pi flow:
//       take() [by high-pri A, mutex held by low-pri B]:
//         if A.priority > B.priority {
//           raise B to A.priority in READY_LISTS
//           B.priority = A.priority          // runtime boost
//           (base_priority is untouched)
//         }
//         place A on wait_list + delay list
//         yield
//
//       give() [by B]:
//         restore B.priority = B.base_priority
//         re-insert B in READY_LISTS at base priority
//         wake highest-priority waiter (same as semaphore)
//
//  3. Recursive locking (xSemaphoreTakeRecursive)
//     If the owner calls take() again, recursive_count is incremented and the call returns immediately (no deadlock).  
//     give() decrements the counter
//     only when it reaches 0 is the mutex actually released
//
//  Usage example:
//
//    static mut MTX: Mutex = Mutex::new();
//
//    unsafe {
//        MTX.init();
//
//        // acquire (blocks up to PORT_MAX_DELAY ticks)
//        if MTX.take(PORT_MAX_DELAY) {
//            // critical section
//            MTX.give();
//        }
//    }

use crate::rtos::kernel::{
    list::List,
    task::{TCB, TaskState},
    types::{TickType, UBaseType, PORT_MAX_DELAY},
};
use crate::rtos::kernel::scheduler::{
    CURRENT_TCB, TICK_COUNT,
    CURRENT_DELAY_LIST, OVERFLOW_DELAY_LIST,
    READY_LISTS,
    record_ready_priority, clear_ready_priority,
};
use crate::rtos::port;

pub struct Mutex {
    wait_list: List<TCB>,
    owner: *mut TCB,

    // 0 means the mutex is free (or held once)
    // Incremented each time the owner calls take() again
    recursive_count: UBaseType,
}

unsafe impl Sync for Mutex {}
unsafe impl Send for Mutex {}

impl Mutex {
    pub const fn new() -> Self {
        Mutex {
            wait_list: List::new(),
            owner: core::ptr::null_mut(),
            recursive_count: 0,
        }
    }

    pub fn init(&mut self) {
        self.wait_list.init();
    }

    //  loop {
    //    enter critical
    //    if owner == null {                  // mutex is free
    //      owner = CURRENT_TCB
    //      recursive_count = 1
    //      exit critical; return true
    //    }
    //    if owner == CURRENT_TCB {           // recursive re-lock by owner
    //      recursive_count += 1
    //      exit critical; return true
    //    }
    //    if timeout == 0 { exit critical; return false }
    //    priority_inherit()                  // boost owner if needed
    //    place_on_event_list(timeout)        // block caller
    //    exit critical
    //    trigger PendSV                      // yield here
    //    // resumed after give() or timeout
    //    if timed_out { return false }
    //    // else retry
    //  }

    /// Acquire the mutex.  
    /// Blocks up to `timeout` ticks.
    /// Use `PORT_MAX_DELAY` to wait forever.
    /// Returns `true` on success, `false` on timeout
    pub unsafe fn take(&mut self, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            // mutex is free
            if self.owner.is_null() {
                self.owner = CURRENT_TCB;
                self.recursive_count = 1;
                port::exit_critical();
                return true;
            }

            // recursive re-lock by the same owner
            if self.owner == CURRENT_TCB {
                self.recursive_count += 1;
                port::exit_critical();
                return true;
            }

            // mutex is held by someone else
            // give up if timeout exhausted
            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            // priority inheritance
            self.priority_inherit();

            // block the current task
            self.place_on_event_list(remaining);

            port::exit_critical();

            port::trigger_pendsv();
            port::instruction_sync();

            // A non-null ctner on event_list_item means give() did NOT remove
            // us from the wait_list, i.e. we timed out
            let timed_out = !(*CURRENT_TCB).event_list_item.ctner.is_null();
            if timed_out {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            // Woken by give() — recalculate remaining and retry.
            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout {
                    return false;
                }
                remaining = timeout - elapsed;
            }
        }
    }

    //
    //  enter critical
    //  assert owner == CURRENT_TCB          // only the owner may release
    //  recursive_count -= 1
    //  if recursive_count > 0 { exit critical; return true }  // still locked
    //  priority_disinherit()                // restore owner's base priority
    //  owner = null
    //  if wait_list non-empty {
    //    remove highest-priority waiter; make it the new owner
    //    move waiter to ready list
    //    trigger PendSV if waiter.priority >= current.priority
    //  }
    //  exit critical; return true

    /// Release the mutex.  
    /// Must be called by the current owner.
    /// Returns `false` if the caller does not own the mutex (programming error)
    pub unsafe fn give(&mut self) -> bool {
        port::enter_critical();

        // only the holder may release
        if self.owner != CURRENT_TCB {
            port::exit_critical();
            return false;
        }

        // release only when it reaches 0
        self.recursive_count -= 1;
        if self.recursive_count > 0 {
            port::exit_critical();
            return true;
        }

        // restore the owner's priority before we clear ownership
        self.priority_disinherit();
        self.owner = core::ptr::null_mut();

        // wake the highest-priority waiter and hand ownership to it
        if self.wait_list.items_num > 0 {
            self.remove_from_event_list();
        }

        port::exit_critical();
        true
    }

    /// If the caller has a higher priority than the current mutex owner,
    /// raise the owner to the caller's priority so it can run and release the mutex sooner
    unsafe fn priority_inherit(&mut self) {
        let caller_priority = (*CURRENT_TCB).priority;
        let owner = self.owner;
        let owner_priority = (*owner).priority;

        if caller_priority <= owner_priority {
            // no boost needed
            return;
        }

        // The owner may currently be in the ready list.
        // We need to move it to the correct (higher-priority) bucket
        if (*owner).state == TaskState::Ready {
            // Remove from the old priority bucket.
            (*owner).state_list_item.remove_in_list();
            if READY_LISTS[owner_priority as usize].items_num == 0 {
                clear_ready_priority(owner_priority);
            }

            // insert at the caller's (higher) priority.
            (*owner).priority = caller_priority;
            (*owner).state_list_item.value = caller_priority as TickType;
            READY_LISTS[caller_priority as usize]
                .insert_end(&raw mut (*owner).state_list_item);
            record_ready_priority(caller_priority);
        } else {
            // if owner is Running or Delayed, just update the priority field.
            // The scheduler will pick up the new value on the next context switch
            (*owner).priority = caller_priority;
        }
    }

    /// Called by give(): restore the owner's runtime priority to its base.
    unsafe fn priority_disinherit(&mut self) {
        let owner = self.owner; // still CURRENT_TCB at this point
        let base = (*owner).base_priority;
        let current_pri = (*owner).priority;

        if current_pri == base {
            // Priority was never boosted; nothing to do.
            return;
        }

        // Owner is currently Running (it is calling give()), so we only need
        // to update the priority field.  The scheduler will use base priority
        // for future ready-list insertions.
        (*owner).priority = base;

        // If the owner is still in a ready list (unlikely while Running, but possible), 
        // move it to the correct bucket
        if (*owner).state == TaskState::Ready {
            (*owner).state_list_item.remove_in_list();
            if READY_LISTS[current_pri as usize].items_num == 0 {
                clear_ready_priority(current_pri);
            }
            (*owner).state_list_item.value = base as TickType;
            READY_LISTS[base as usize].insert_end(&raw mut (*owner).state_list_item);
            record_ready_priority(base);
        }
    }

    unsafe fn place_on_event_list(&mut self, timeout: TickType) {
        let tcb = CURRENT_TCB;
        let priority = (*tcb).priority;

        // Insert into wait_list ordered by priority (highest first)
        (*tcb).event_list_item.value = PORT_MAX_DELAY - priority;
        self.wait_list.insert(&raw mut (*tcb).event_list_item);

        // Insert into delay list ordered by absolute wake time
        let (wake_time, overflowed) = TICK_COUNT.overflowing_add(timeout);
        (*tcb).ticks_to_delay = wake_time;
        (*tcb).state_list_item.value = wake_time;

        // Remove from the ready list.
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

    /// Wake the highest-priority waiter and make it the new mutex owner
    unsafe fn remove_from_event_list(&mut self) {
        // The wait_list sentinel's `next` is the highest-priority item
        // because list.insert() sorts by ascending value and we store
        // PORT_MAX_DELAY - priority (so higher priority → smaller value → head).
        let end_ptr = &raw mut self.wait_list.list_end;
        let item = (*end_ptr).next;
        let tcb = (*item).owner as *mut TCB;

        // Remove from wait_list.
        (*item).remove_in_list();

        // Remove from delay list (task was blocked, so it is in a delay list).
        if (*tcb).state == TaskState::Delayed {
            (*tcb).state_list_item.remove_in_list();
        }

        // Hand ownership to the waiter before it runs.
        self.owner = tcb;
        self.recursive_count = 1;

        // Move to the ready list.
        let priority = (*tcb).priority;
        (*tcb).state_list_item.value = priority as TickType;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
        (*tcb).state = TaskState::Ready;

        // Preempt if the newly ready task outranks the current task.
        if priority >= (*CURRENT_TCB).priority {
            port::trigger_pendsv();
        }
    }
}