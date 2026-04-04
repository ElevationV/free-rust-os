#![no_std]
#![no_main]

mod kernel;
mod port;

use cortex_m_rt::entry;
use cortex_m_semihosting::hprintln;
use kernel::task::TCB;
use kernel::types::StackType;


#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

static mut TASK1_STACK: [StackType; 256] = [0; 256];
static mut TASK2_STACK: [StackType; 256] = [0; 256];
static mut TASK1_TCB: TCB = TCB::new();
static mut TASK2_TCB: TCB = TCB::new();

#[entry]
fn main() -> ! {
    hprintln!("Hello from Rust!").ok();
    hprintln!("initializing tasks").ok();

    unsafe {
        hprintln!("1: before task1 init").ok();
        kernel::scheduler::init();
        hprintln!("1.5: lists initialized").ok();
        
        TASK1_TCB.init(
            task1, core::ptr::null_mut(),
            TASK1_STACK.as_mut_ptr(), 256,
            1, "Task1"
        );
        hprintln!("2: after task1 init").ok();

        TASK2_TCB.init(
            task2, core::ptr::null_mut(),
            TASK2_STACK.as_mut_ptr(), 256,
            1, "Task2"
        );
        hprintln!("3: after task2 init").ok();

        kernel::scheduler::record_ready_priority(1);
        hprintln!("4: priority recorded").ok();

        kernel::scheduler::READY_LISTS[1]
            .insert_end(&raw mut TASK1_TCB.state_list_item);
        hprintln!("5: task1 inserted").ok();

        kernel::scheduler::READY_LISTS[1]
            .insert_end(&raw mut TASK2_TCB.state_list_item);
        hprintln!("6: task2 inserted").ok();

        hprintln!("7: starting scheduler").ok();
        kernel::scheduler::select_highest_priority_task();
        hprintln!("current_tcb: {:x}", kernel::scheduler::CURRENT_TCB as usize).ok();
        hprintln!("task1_tcb addr: {:x}", &raw const TASK1_TCB as usize).ok();
        hprintln!("top_of_stack: {:x}", (*kernel::scheduler::CURRENT_TCB).top_of_stack as usize).ok();

        let top = (*kernel::scheduler::CURRENT_TCB).top_of_stack;
        hprintln!("top_of_stack: {:x}", top as usize).ok();
        // 打印栈帧内容，应该能看到 xPSR=0x01000000 和任务函数地址
        hprintln!("stack[0] (r4): {:x}", *top).ok();
        hprintln!("stack[8] (r0): {:x}", *top.add(8)).ok();
        hprintln!("stack[14] (pc): {:x}", *top.add(14)).ok();
        hprintln!("stack[15] (xpsr): {:x}", *top.add(15)).ok();
        kernel::scheduler::start();
        
        hprintln!("8: should not reach here").ok();
    }

    loop {}
}

unsafe extern "C" fn task1(_param: *mut ()) {
    loop {
        hprintln!("Task1 running").ok();
    }
}

unsafe extern "C" fn task2(_param: *mut ()) {
    loop {
        hprintln!("Task2 running").ok();
    }
}