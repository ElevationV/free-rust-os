#![allow(dead_code)]
#![allow(static_mut_refs)]

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

    pub unsafe fn take(&mut self, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            if self.owner.is_null() {
                self.owner = CURRENT_TCB;
                self.recursive_count = 1;
                port::exit_critical();
                return true;
            }

            if self.owner == CURRENT_TCB {
                self.recursive_count += 1;
                port::exit_critical();
                return true;
            }
            
            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            self.priority_inherit();

            self.place_on_event_list(remaining);

            port::exit_critical();

            port::task_yield();
            port::instruction_sync();

            let timed_out = !(*CURRENT_TCB).event_list_item.ctner.is_null();
            if timed_out {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout {
                    return false;
                }
                remaining = timeout - elapsed;
            }
        }
    }

    pub unsafe fn give(&mut self) -> bool {
        port::enter_critical();

        if self.owner != CURRENT_TCB {
            port::exit_critical();
            return false;
        }

        self.recursive_count -= 1;
        if self.recursive_count > 0 {
            port::exit_critical();
            return true;
        }

        self.priority_disinherit();
        self.owner = core::ptr::null_mut();

        if self.wait_list.items_num > 0 {
            self.remove_from_event_list();
        }

        port::exit_critical();
        true
    }

    unsafe fn priority_inherit(&mut self) {
        let caller_priority = (*CURRENT_TCB).priority;
        let owner = self.owner;
        let owner_priority = (*owner).priority;

        if caller_priority <= owner_priority {
            return;
        }

        if (*owner).state == TaskState::Ready {
            (*owner).state_list_item.remove_in_list();
            if READY_LISTS[owner_priority as usize].items_num == 0 {
                clear_ready_priority(owner_priority);
            }

            (*owner).priority = caller_priority;
            (*owner).state_list_item.value = caller_priority as TickType;
            READY_LISTS[caller_priority as usize]
                .insert_end(&raw mut (*owner).state_list_item);
            record_ready_priority(caller_priority);
        } else {
            (*owner).priority = caller_priority;
        }
    }

    unsafe fn priority_disinherit(&mut self) {
        let owner = self.owner; 
        let base = (*owner).base_priority;
        let current_pri = (*owner).priority;

        if current_pri == base {
            return;
        }

        (*owner).priority = base;
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
        
        (*tcb).event_list_item.value = PORT_MAX_DELAY - priority;
        self.wait_list.insert(&raw mut (*tcb).event_list_item);

        let (wake_time, overflowed) = TICK_COUNT.overflowing_add(timeout);
        (*tcb).ticks_to_delay = wake_time;
        (*tcb).state_list_item.value = wake_time;

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
        let end_ptr = &raw mut self.wait_list.list_end;
        let item = (*end_ptr).next;
        let tcb = (*item).owner as *mut TCB;

        // Remove from wait_list.
        (*item).remove_in_list();

        if (*tcb).state == TaskState::Delayed {
            (*tcb).state_list_item.remove_in_list();
        }

        self.owner = tcb;
        self.recursive_count = 1;

        let priority = (*tcb).priority;
        (*tcb).state_list_item.value = priority as TickType;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
        (*tcb).state = TaskState::Ready;

        if priority >= (*CURRENT_TCB).priority {
            port::task_yield();
        }
    }
}