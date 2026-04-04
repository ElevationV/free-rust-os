#![allow(dead_code)]

use crate::kernel::task::TCB;
use crate::kernel::list::List;
use crate::kernel::types::{UBaseType, MAX_PRIORITIES};
use crate::port::cortex_m3;

pub static mut CURRENT_TCB: *mut TCB = core::ptr::null_mut();
pub static mut TOP_READY_PRIORITY: UBaseType = 0;
pub static mut READY_LISTS: [List<TCB>; MAX_PRIORITIES] = [
    List::new(),
    List::new(),
    List::new(),
    List::new(),
    List::new(),
];

pub unsafe fn init() {
    for i in 0..MAX_PRIORITIES {
        READY_LISTS[i].init();
    }
}

pub unsafe fn record_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY |= 1 << priority;
}

pub unsafe fn clear_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY &= !(1 << priority);
}

pub unsafe fn select_highest_priority_task() {
    let top_priority = 31 - TOP_READY_PRIORITY.leading_zeros();
    CURRENT_TCB = READY_LISTS[top_priority as usize].get_next_entry();
}

pub unsafe fn switch_context() {
    select_highest_priority_task();
}


pub unsafe fn tick() {
    // no tick currently
    cortex_m3::trigger_pendsv();
}



pub unsafe fn start() {
    cortex_m3::start_scheduler();
}