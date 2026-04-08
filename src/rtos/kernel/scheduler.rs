#![allow(dead_code)]
#![allow(static_mut_refs)]
use crate::rtos::kernel::task::TCB;
use crate::rtos::kernel::list::{List, ListItem};
use crate::rtos::kernel::types::{UBaseType, StackType, TickType};
use crate::rtos::config::{TIME_SLICE, MAX_PRIORITIES, USE_TIME_SLICING, IDLE_STACK_SIZE};
use crate::rtos::port;

pub static mut SLICE_COUNTER: TickType = 0;
pub static mut CURRENT_TCB: *mut TCB = core::ptr::null_mut();
pub static mut TOP_READY_PRIORITY: UBaseType = 0;
pub static mut DELAY_LIST: List<TCB> = List::new();
pub static mut TICK_COUNT: TickType = 0;
pub static mut READY_LISTS: [List<TCB>; MAX_PRIORITIES] = [
    List::new(),
    List::new(),
    List::new(),
    List::new(),
    List::new(),
];
pub static mut IDLE_TCB:TCB = TCB::new();
pub static mut IDLE_STACK: [StackType; IDLE_STACK_SIZE] = [0; IDLE_STACK_SIZE];

pub unsafe fn init() {
    for i in 0..MAX_PRIORITIES {
        READY_LISTS[i].init();
    }
    DELAY_LIST.init();
}

pub unsafe fn create_task(
    task_fn: unsafe extern "C" fn(*mut ()),
    name: &str,
    priority: UBaseType,
    stack: *mut StackType,
    stack_depth: usize,
    tcb: *mut TCB,
) {
    let priority = if priority as usize >= MAX_PRIORITIES {
        MAX_PRIORITIES as UBaseType - 1
    } else {
        priority
    };
    
    (*tcb).init(task_fn, core::ptr::null_mut(), stack, stack_depth, priority, name);
    record_ready_priority(priority);
    READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
}

pub unsafe fn start() {
    create_task(idle_task, "IDLE", 0, 
        &raw mut IDLE_STACK[0] as *mut StackType, 
        128,
        &raw mut IDLE_TCB,
    );
    
    select_highest_priority_task();
    port::start_scheduler();
}

pub unsafe fn task_delay(ticks: TickType) {
    SLICE_COUNTER = 0;
    port::disable_interrupts();

    let wake_time = TICK_COUNT + ticks;
    (*CURRENT_TCB).ticks_to_delay = wake_time;

    let priority = (*CURRENT_TCB).priority;
    // remove in `READY_LISTS`
    (*CURRENT_TCB).state_list_item.remove_in_list();
    // update bitmap
    if READY_LISTS[priority as usize].items_num == 0 {
        clear_ready_priority(priority);
    }

    (*CURRENT_TCB).state_list_item.value = wake_time;
    DELAY_LIST.insert(&raw mut (*CURRENT_TCB).state_list_item);

    port::enable_interrupts();
    port::trigger_pendsv();
    port::instruction_sync();
}

// Task Schedule Helper
pub(crate) unsafe fn record_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY |= 1 << priority;
}

pub(crate) unsafe fn clear_ready_priority(priority: UBaseType) {
    TOP_READY_PRIORITY &= !(1 << priority);
}

pub(crate) unsafe fn select_highest_priority_task() {
    if TOP_READY_PRIORITY == 0 {
        // fallback: idle task
        CURRENT_TCB = &raw mut IDLE_TCB;
        return;
    }

    let top_priority = 31 - TOP_READY_PRIORITY.leading_zeros();
    CURRENT_TCB = READY_LISTS[top_priority as usize].get_next_entry();
}

pub(crate) unsafe fn switch_context() {
    select_highest_priority_task();
}

unsafe extern "C" fn idle_task(_param: *mut ()) {
    loop {
        core::arch::asm!("wfi");
    }
}

pub(crate) unsafe fn tick() {
    TICK_COUNT += 1;
    
    // check `DELAY_LIST` and move ready tasks back to `READY_LISTS`
    loop {
        let end_ptr = &raw mut DELAY_LIST.list_end as *mut ListItem<TCB>;
        let head = (*end_ptr).next;
        // delay list is empty or head task not ready
        if head == end_ptr || (*head).value > TICK_COUNT{
            break;
        }
        
        let tcb = (*head).owner as *mut TCB;
        (*head).remove_in_list(); // remove in `DELAY_LIST`
        
        (*head).value = (*tcb).priority as TickType;
        let priority = (*tcb).priority;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
    }
    
    if USE_TIME_SLICING {
        let priority = (*CURRENT_TCB).priority;
        if READY_LISTS[priority as usize].items_num > 1 {
            SLICE_COUNTER += 1;
            if SLICE_COUNTER >= TIME_SLICE {
                SLICE_COUNTER = 0;
                port::trigger_pendsv();
            }
        } else {
            SLICE_COUNTER = 0;
        }
    }
}
