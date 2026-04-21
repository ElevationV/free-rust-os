#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::task::{TCB, TaskState};
use super::list::{List, ListItem};
use super::types::{UBaseType, StackType, TickType};
use super::config::USE_TIME_SLICING;

use crate::rtos::{list, port};

pub static mut CURRENT_TCB: *mut TCB = core::ptr::null_mut();
pub static mut TOP_READY_PRIORITY: UBaseType = 0;
pub static mut TICK_COUNT: TickType = 0;
pub static mut READY_LISTS: [List<TCB>; 5] = [
    List::new(), List::new(), List::new(), List::new(), List::new()
];

// Two delay lists to handle TickType(u32) overflow
//
// CURRENT_DELAY_LIST points to the active list for the current tick epoch. 
// A task is inserted here when its wake_time does not overflow
//
// OVERFLOW_DELAY_LIST points to the list for the next tick epoch. 
// A task is inserted here when its wake_time wraps around
//
// when TICK_COUNT overflows back to 0, the two pointers are swapped
static mut DELAY_LIST_1: List<TCB> = List::new();
static mut DELAY_LIST_2: List<TCB> = List::new();
pub(crate) static mut CURRENT_DELAY_LIST: *mut List<TCB> = core::ptr::null_mut();
pub(crate) static mut OVERFLOW_DELAY_LIST: *mut List<TCB> = core::ptr::null_mut();

// Suspend list
static mut SUSPENDED_LIST: List<TCB> = List::new();

// IDLE task
pub static mut IDLE_TCB: TCB = TCB::new();
pub static mut IDLE_STACK: [StackType; 256] = [0; 256];

unsafe extern "C" fn idle_task(_param: *mut ()) {
    loop {}
}

pub unsafe fn init() {
    for i in 0..5 {
        READY_LISTS[i].init();
    }

    DELAY_LIST_1.init();
    DELAY_LIST_2.init();
    CURRENT_DELAY_LIST  = &raw mut DELAY_LIST_1;
    OVERFLOW_DELAY_LIST = &raw mut DELAY_LIST_2;

    create_task(
        idle_task, "IDLE", 0,
        IDLE_STACK.as_mut_ptr(),
        IDLE_STACK.len(),
        &raw mut IDLE_TCB,
    );
}

pub unsafe fn start() {
    select_highest_priority_task();
    port::start_scheduler();
}

// Task state transitions:
//
//   create_task          : None      -> Ready
//   scheduler selects    : Ready     -> Running
//   task_delay           : Running   -> Delayed
//   tick / abort_delay   : Delayed   -> Ready
//   task_suspend         : Running   -> Suspended
//   task_suspend         : Ready     -> Suspended
//   task_suspend         : Delayed   -> Suspended  (discards remaining delay)
//   task_resume          : Suspended -> Ready
// 

pub unsafe fn create_task(
    task_fn: unsafe extern "C" fn(*mut ()),
    name: &str,
    priority: UBaseType,
    stack: *mut StackType,
    stack_depth: usize,
    tcb: *mut TCB,
) {
    let prio = if priority as usize >= 5 { 4 } else { priority };

    (*tcb).init(task_fn, core::ptr::null_mut(), stack, stack_depth, prio, name);
    record_ready_priority(prio);
    READY_LISTS[prio as usize].insert_before_index(&raw mut (*tcb).state_list_item);
    (*tcb).state = TaskState::Ready;
}


pub unsafe fn task_delay(ticks: TickType) {
    port::disable_interrupts();

    let (wake_time, overflowed) = TICK_COUNT.overflowing_add(ticks);
    (*CURRENT_TCB).ticks_to_delay = wake_time;
    // remove current task from ready list
    let priority = (*CURRENT_TCB).priority;
    (*CURRENT_TCB).state_list_item.remove_in_list();
    if READY_LISTS[priority as usize].items_num == 0 {
        clear_ready_priority(priority);
    }
    // put current task into delay list
    (*CURRENT_TCB).state_list_item.value = wake_time;

    // if wake time overflowed, put it into overflow delay list(where OVERFLOW_DELAY_LIST pointed at)
    if overflowed {
        (*OVERFLOW_DELAY_LIST).insert(&raw mut (*CURRENT_TCB).state_list_item);
    } else {
        (*CURRENT_DELAY_LIST).insert(&raw mut (*CURRENT_TCB).state_list_item);
    }
    (*CURRENT_TCB).state = TaskState::Delayed;

    port::enable_interrupts();
    port::task_yield();
    port::instruction_sync();
}

pub unsafe fn abort_delay(tcb: *mut TCB) {
    if (*tcb).state != TaskState::Delayed {
        return;
    }
    
    let list_item = &raw mut (*tcb).state_list_item;
    // orphan node
    if (*list_item).ctner.is_null() {
        return;
    }
    // remove it from delay list
    (*list_item).remove_in_list();
    
    // put it into ready list
    let priority = (*tcb).priority;
    (*list_item).value = priority;
    READY_LISTS[priority as usize].insert_before_index(list_item);
    record_ready_priority(priority);
    (*tcb).state = TaskState::Ready;
    
    if priority >= (*CURRENT_TCB).priority {
        port::task_yield();
    }
}

pub unsafe fn task_suspend(tcb: *mut TCB) {
    if matches!((*tcb).state, TaskState::None | TaskState::Suspended) {
        return; 
    }
    
    let list_item = &raw mut (*tcb).state_list_item;
    
    match (*tcb).state {
        TaskState::Ready | TaskState::Running => {
            // remove from ready list and reset priority(if necessary)
            (*list_item).remove_in_list();
            let priority = (*tcb).priority;
            if READY_LISTS[priority as usize].items_num == 0 {
                clear_ready_priority(priority);
            }
        }
        TaskState::Delayed => {
            // only remove it from delay list
            (*list_item).remove_in_list();
        }
        _ => unreachable!()
    }
    // put it into suspend list
    (*tcb).state = TaskState::Suspended;
    SUSPENDED_LIST.insert_before_index(list_item);
    
    if tcb == CURRENT_TCB {
        port::task_yield();
        port::instruction_sync();
    }
}

pub unsafe fn task_resume(tcb: *mut TCB) {
    if (*tcb).state != TaskState::Suspended {
        return;
    }
    // remove it from suspended list
    let list_item = &raw mut (*tcb).state_list_item;
    (*list_item).remove_in_list();
    // put it into ready list
    let priority = (*tcb).priority;
    (*list_item).value = priority as TickType;
    READY_LISTS[priority as usize].insert_before_index(list_item);
    record_ready_priority(priority);
    (*tcb).state = TaskState::Ready;
 
    if priority >= (*CURRENT_TCB).priority {
        port::task_yield();
    }
}

pub(crate) unsafe fn record_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY |= 1 << priority;
}

pub(crate) unsafe fn clear_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY &= !(1 << priority);
}

pub(crate) unsafe fn select_highest_priority_task() {
    if TOP_READY_PRIORITY == 0 {
        CURRENT_TCB = &raw mut IDLE_TCB;
        return;
    }
    let top_priority = 31 - TOP_READY_PRIORITY.leading_zeros();
    CURRENT_TCB = READY_LISTS[top_priority as usize].get_next_entry();
}

pub(crate) unsafe fn switch_context() {
    port::check_stack_overflow((*CURRENT_TCB).stack, &(*CURRENT_TCB).name);
    // CURRENT_TCB may be changed in `task_suspend` or `task_delay`
    // their state are not Ready, and shouldn't be Ready
    if (*CURRENT_TCB).state == TaskState::Running {
        (*CURRENT_TCB).state = TaskState::Ready;
    }
    select_highest_priority_task();
    (*CURRENT_TCB).state = TaskState::Running;
}

unsafe fn switch_delay_lists() {
    core::ptr::swap(
        &raw mut CURRENT_DELAY_LIST,
        &raw mut OVERFLOW_DELAY_LIST,
    );
}

pub(crate) unsafe fn tick() {
    let (next_tick, overflowed) = TICK_COUNT.overflowing_add(1);
    TICK_COUNT = next_tick;

    // TICK_COUNT overflowed, swap two pointers
    if overflowed {
        switch_delay_lists();
    }

    let mut need_switch = false;

    // wake every task in the current delay list whose wake_time has beenreached
    loop {
        let end_ptr = &raw mut (*CURRENT_DELAY_LIST).list_end as *mut ListItem<TCB>;
        let head = (*end_ptr).next;

        // List is empty, or the earliest wake_time is still in the future.
        if head == end_ptr || (*head).value > TICK_COUNT {
            break;
        }

        // remove head node from delay list
        let tcb = (*head).owner as *mut TCB;
        (*head).remove_in_list();

        // put it into ready list
        (*head).value = (*tcb).priority as TickType;
        let priority = (*tcb).priority;
        READY_LISTS[priority as usize].insert_before_index(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
        (*tcb).state = TaskState::Ready;

        if priority >= (*CURRENT_TCB).priority {
            need_switch = true;
        }
    }
    
    // timeslice is 1 tick
    // used to switch between tasks with same priority
    if USE_TIME_SLICING {
        let priority = (*CURRENT_TCB).priority;
        if READY_LISTS[priority as usize].items_num > 1 {
            need_switch = true;
        }
    }

    if need_switch {
        port::task_yield();
    }
}