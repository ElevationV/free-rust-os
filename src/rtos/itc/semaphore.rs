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
    record_ready_priority, clear_ready_priority,
};
use crate::rtos::port;

pub struct Semaphore {
    count: UBaseType,
    max_count: UBaseType,
    wait_list: List<TCB>,
}

impl Semaphore {
    pub const fn new_binary() -> Self {
        Semaphore {
            count: 1,
            max_count: 1,
            wait_list: List::new(),
        }
    }

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


    pub unsafe fn take(&mut self, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;
        
        loop {
            port::enter_critical();
            
            // semaphore is available
            if self.count > 0 {
                self.count -= 1;
                port::exit_critical();
                return true;
            }
            // no count available and caller does not want to wait
            if remaining == 0 {
                port::exit_critical();
                return false;
            }
            
            // place current task on both the semaphore wait_list and delay list
            self.place_on_event_list(remaining);

            port::exit_critical();

            port::task_yield();
            port::instruction_sync();

            // check whether we were woken by a timeout or by give()
            // give() clears event_list_item.ctner before moving the task to the ready list, 
            // so a non-null ctner means we timed out
            let timed_out = !(*CURRENT_TCB).event_list_item.ctner.is_null();
            if timed_out {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout == PORT_MAX_DELAY {
                // keep remaining as PORT_MAX_DELAY
            } else {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout {
                    return false; // ran out of time between give() and re-take
                }
                remaining = timeout - elapsed;
            }
        }
    }

    pub unsafe fn give(&mut self) -> bool {
        port::enter_critical();
        
        let result = if self.wait_list.items_num > 0 {
            // wake the highest-priority waiter
            self.count += 1;
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
        let priority = (*tcb).priority;
        (*tcb).event_list_item.value = PORT_MAX_DELAY - priority;
        self.wait_list.insert(&raw mut (*tcb).event_list_item);
        
        // insert into delay_list by wake_time
        let (wake_time, overflowed) = TICK_COUNT.overflowing_add(timeout);
        (*tcb).ticks_to_delay = wake_time;
        (*tcb).state_list_item.value = wake_time;
        
        // remove in redy list
        (*tcb).state_list_item.remove_in_list();
        if READY_LISTS[priority as usize].items_num == 0 {
            clear_ready_priority(priority);
        }
        
        // TICK_COUNT Overflow
        if overflowed {
            (*OVERFLOW_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        } else {
            (*CURRENT_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        }
        
        // set task state as Delayed
        (*tcb).state = TaskState::Delayed;
    }

    unsafe fn remove_from_event_list(&mut self) {
        // get the last blocked task
        let end_ptr = &raw mut self.wait_list.list_end;
        let item = (*end_ptr).next;
        let tcb = (*item).owner as *mut TCB;
        
        // remove in wait list
        (*item).remove_in_list();
        
        // remove in DELAY_LIST
        if (*tcb).state == TaskState::Delayed {
            (*tcb).state_list_item.remove_in_list();
        }
        
        // insert back into READY_LISTS
        let priority = (*tcb).priority;
        (*tcb).state_list_item.value = priority as TickType;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority); 
        (*tcb).state = TaskState::Ready;

        // higher priority task exists
        if priority >= (*CURRENT_TCB).priority {
            port::task_yield();
        }
    }
}