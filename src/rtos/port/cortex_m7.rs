#![allow(dead_code)]

use crate::rtos::kernel::types::{StackType, MAX_SYSCALL_INTERRUPT_PRIORITY};
use core::arch::{naked_asm};

const INITIAL_XPSR: StackType = 0x01000000; // xPSR: Process State Register
const START_ADDRESS_MASK: StackType = 0xFFFFFFFE;

const NVIC_SYSPRI2: *mut u32 = 0xE000ED20 as *mut u32; // System Handler Priority Register 2
const NVIC_PENDSV_PRI: u32 = (255u32) << 16;  // PendSV Priority
const NVIC_SYSTICK_PRI: u32 = (255u32) << 24; // SysTick Priority

const NVIC_INT_CTRL: *mut u32 = 0xE000ED04 as *mut u32; // ICSR: Interrupt Control and State Register
const PENDSVSET_BIT: u32 = 1 << 28;

const SYSTICK_CTRL: *mut u32 = 0xE000E010 as *mut u32; // CSR: Comtrol and Status Register
const SYSTICK_LOAD: *mut u32 = 0xE000E014 as *mut u32; // RR: Reload Register
const SYSTICK_ENABLE_BIT: u32 = 1 << 0; // enable sys tick
const SYSTICK_INT_BIT: u32 = 1 << 1;    // allow interrupt
const SYSTICK_CLK_BIT: u32 = 1 << 2;    // clock source

static mut CRITICAL_NESTING: u32 = 0xaaaaaaaa; 

// Overflow detection
// 
// Verify the lowest `STACK_CHECK_WORDS` words of the stack buffer stillc ontain `STACK_FILL_BYTE`
// If any word has been overwritten, the stack has grown past its allocated space and an overflow is reported.  
pub const STACK_FILL_BYTE: StackType = 0xA5A5A5A5;
const STACK_CHECK_WORDS: usize = 4;

unsafe fn task_exit_error() -> ! {
    loop {}
}

pub unsafe fn initialise_stack(
    mut top: *mut StackType,
    task_fn: unsafe extern "C" fn(*mut ()),
    param: *mut (),
) -> *mut StackType {
    // xPSR
    top = top.sub(1);
    top.write(INITIAL_XPSR);

    // PC
    top = top.sub(1);
    top.write((task_fn as StackType) & START_ADDRESS_MASK);

    // LR
    top = top.sub(1);
    top.write(task_exit_error as StackType);

    // clear r12, r3, r2, r1
    top = top.sub(5);
    // r0 = param
    top.write(param as StackType);

    // clear r4-r11
    top = top.sub(8);

    top
}

pub unsafe fn start_scheduler() {
    CRITICAL_NESTING = 0;
    // set PendSV and SysTick lowest priority
    NVIC_SYSPRI2.write_volatile(NVIC_SYSPRI2.read_volatile() | NVIC_PENDSV_PRI);
    NVIC_SYSPRI2.write_volatile(NVIC_SYSPRI2.read_volatile() | NVIC_SYSTICK_PRI);
    
    // set systick cycle
    SYSTICK_LOAD.write_volatile(1000 - 1);
    SYSTICK_CTRL.write_volatile(SYSTICK_CLK_BIT | SYSTICK_INT_BIT | SYSTICK_ENABLE_BIT);

    start_first_task();
}


// construct critical section
pub unsafe fn enter_critical() {
    disable_interrupts();
    CRITICAL_NESTING += 1;
}

pub unsafe fn exit_critical() {
    CRITICAL_NESTING -= 1;
    if CRITICAL_NESTING == 0 {
        enable_interrupts();
    }
}

// tirgger pendsv
pub unsafe fn task_yield() {
    NVIC_INT_CTRL.write_volatile(PENDSVSET_BIT);
}

// check and handle stackoverflow
pub unsafe fn check_stack_overflow(
    stack_base: *mut StackType, 
    task_name: &[u8; 16]) {
    for i in 0..STACK_CHECK_WORDS {
        if stack_base.add(i).read() != STACK_FILL_BYTE {
            handle_stack_overflow();
        }
    }
}

// this function could be customized
fn handle_stack_overflow() {
    loop{}
}

pub unsafe fn disable_interrupts() {
    core::arch::asm!(
        "mov r0, {max_pri}",
        "msr basepri, r0",
        "dsb",
        "isb",
        max_pri = const MAX_SYSCALL_INTERRUPT_PRIORITY,
        out("r0") _,
    );
}

pub unsafe fn enable_interrupts() {
    core::arch::asm!(
        "mov r0, #0",
        "msr basepri, r0",
        out("r0") _,
    );
}

// isb
pub unsafe fn instruction_sync() {
    core::arch::asm!("isb", options(nostack));
}

// set up
#[unsafe(naked)]
unsafe extern "C" fn start_first_task() {
    naked_asm!(
        "ldr r0, = 0xE000ED08", // r0 = &VTOR
        "ldr r0, [r0]",         // r0 = VTOR
        "ldr r0, [r0]",         // r0 = vector_table[0]
        "msr msp, r0",          // set MSP

        "cpsie i",              // enable IRQ interrupt
        "cpsie f",              // enable Fault interrupt
        "dsb",
        "isb",
        "svc 0",                // trigger SVC interrupt
        "nop",
        "nop"
    )
}

// handle SVC interrupt
#[unsafe(naked)]
#[export_name = "SVCall"]
unsafe extern "C" fn svc_handler() {
    naked_asm!(
        "ldr r3, ={current_tcb}", // r3 = &CURRENT_TCB
        "ldr r1, [r3]",           // r1 = CURRENT_TCB
        "ldr r0, [r1]",           // r0 = (*CURRENT_TCB).top_of_stack
        "ldmia r0!, {{r4-r11}}",  // pop r4-r11 out of stack
        "msr psp, r0",            // psp = r0: psp pointed to the hardware stack frame
        "isb",
        "mov r0, #0",             // r0 = #0
        "msr basepri, r0",        // enable all interrupts
        "orr r14, r14, #0xD",     // set Thread, PSP
        "bx r14",                 // automatically pop r0-r3, r12, PC, LR, xPSR
        current_tcb = sym crate::rtos::kernel::scheduler::CURRENT_TCB,
    )
}   // (then run current task)


// function need swich because of sys_tick or schedule..
#[unsafe(naked)]
#[export_name = "PendSV"]
unsafe extern "C" fn pend_sv_handler() { 
    naked_asm!(
        "mrs r0, psp",            // r0 = psp
        "isb",
        "ldr r3, ={current_tcb}", // get &CURRENT_TCB
        "ldr r2, [r3]",           // get CURRENT_TCB
        "stmdb r0!, {{r4-r11}}",  // push r4-r11 into stack
        "str r0, [r2]",           // save new stack top back to (*CURRENT_TCB).top_of_stack
        
        "stmdb sp!, {{r3, r14}}", // save r3, r14(`bl` will change r14)
        "mov r0, #{max_pri}",     // r0 = MAX_SYSCALL_INTERRUPT_PRIORITY
        "msr basepri, r0",        // disable all interrupts
        "dsb",
        "isb",
        "bl {switch}",            // call switch function: select highest priority(mainly)
        "mov r0, #0",             // r0 = #0
        "msr basepri, r0",        // enable all interrupts
        "ldmia sp!, {{r3, r14}}", // recover r3, r14
        
        "ldr r1, [r3]",           // r1 = new CURRENT_TCB(because r3 stores address)
        "ldr r0, [r1]",           // r0 = new (*CURRENT_TCB).top_of_stack
        "ldmia r0!, {{r4-r11}}",  // pop r4-r11
        "msr psp, r0",            // psp = r0
        "isb",
        "bx r14",                 // automically pop the rest of registers
        "nop",
        current_tcb = sym crate::rtos::kernel::scheduler::CURRENT_TCB,
        max_pri = const crate::rtos::kernel::types::MAX_SYSCALL_INTERRUPT_PRIORITY,
        switch = sym crate::rtos::kernel::scheduler::switch_context,
    )  
}   // (then run current task)

#[export_name = "SysTick"]
pub unsafe extern "C" fn sys_tick_handler() {
    crate::rtos::kernel::scheduler::tick();
}
