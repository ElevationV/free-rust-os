#![no_std]
#![no_main]

mod rtos;

use cortex_m_rt::entry;
use cortex_m_semihosting::hprintln;
use rtos::task::TCB;
use rtos::types::StackType;


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
    unsafe {
        rtos::scheduler::init();

        rtos::scheduler::create_task(
            task1, "Task1", 1,
            &raw mut TASK1_STACK[0] as *mut StackType, 256,
            &raw mut TASK1_TCB,
        );
        rtos::scheduler::create_task(
            task2, "Task2", 1,
            &raw mut TASK2_STACK[0] as *mut StackType, 256,
            &raw mut TASK2_TCB,
        );
        
        rtos::scheduler::start();
    }
    loop {}
}

unsafe extern "C" fn task1(_param: *mut ()) {
    loop {
        hprintln!("Task1 running").ok();
        rtos::scheduler::task_delay(100000);
    }
}

unsafe extern "C" fn task2(_param: *mut ()) {
    loop {
        hprintln!("Task2 running").ok();
        rtos::scheduler::task_delay(10000);
    }
}
