#![allow(dead_code)]

use crate::rtos::kernel::types::StackType;
use core::arch::{naked_asm};

const INITIAL_XPSR: StackType = 0x01000000;
const START_ADDRESS_MASK: StackType = 0xfffffffe;

const NVIC_SYSPRI2: *mut u32 = 0xe000ed20 as *mut u32;
const NVIC_PENDSV_PRI: u32 = (255u32) << 16;
const NVIC_SYSTICK_PRI: u32 = (255u32) << 24;

const NVIC_INT_CTRL: *mut u32 = 0xe000ed04 as *mut u32;
const PENDSVSET_BIT: u32 = 1 << 28;

const SYSTICK_CTRL: *mut u32 = 0xe000e010 as *mut u32;
const SYSTICK_LOAD: *mut u32 = 0xe000e014 as *mut u32;
const SYSTICK_ENABLE_BIT: u32 = 1 << 0;
const SYSTICK_INT_BIT: u32 = 1 << 1;
const SYSTICK_CLK_BIT: u32 = 1 << 2;

static mut CRITICAL_NESTING: u32 = 0xaaaaaaaa;


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
    // 设置 PendSV 和 SysTick 为最低优先级
    NVIC_SYSPRI2.write_volatile(NVIC_SYSPRI2.read_volatile() | NVIC_PENDSV_PRI);
    NVIC_SYSPRI2.write_volatile(NVIC_SYSPRI2.read_volatile() | NVIC_SYSTICK_PRI);

    // 配置 SysTick 每 1000 个时钟周期触发一次
    SYSTICK_LOAD.write_volatile(1000 - 1);
    SYSTICK_CTRL.write_volatile(SYSTICK_CLK_BIT | SYSTICK_INT_BIT | SYSTICK_ENABLE_BIT);

    start_first_task();
}


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

pub unsafe fn trigger_pendsv() {
    NVIC_INT_CTRL.write_volatile(PENDSVSET_BIT);
}

pub unsafe fn disable_interrupts() {
    core::arch::asm!(
        "mov r0, {max_pri}",
        "msr basepri, r0",
        "dsb",
        "isb",
        max_pri = const 191u32,
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

pub unsafe fn instruction_sync() {
    core::arch::asm!("isb", options(nostack));
}

#[unsafe(naked)]
pub unsafe extern "C" fn start_first_task() {
    naked_asm!(
        "ldr r0, = 0xE000ED08",
        "ldr r0, [r0]",
        "ldr r0, [r0]",
        "msr msp, r0",

        "cpsie i",
        "cpsie f",
        "dsb",
        "isb",
        "svc 0",
        "nop",
        "nop"
    )
}

#[unsafe(naked)]
#[export_name = "SVCall"]
pub unsafe extern "C" fn svc_handler() {
    naked_asm!(
        "ldr r3, ={current_tcb}",
        "ldr r1, [r3]",
        "ldr r0, [r1]",
        "ldmia r0!, {{r4-r11}}",
        "msr psp, r0",
        "isb",
        "mov r0, #0",
        "msr basepri, r0",
        "orr r14, r14, #0xD",
        "bx r14",
        current_tcb = sym crate::rtos::kernel::scheduler::CURRENT_TCB,
    )
}

#[unsafe(naked)]
#[export_name = "PendSV"]
pub unsafe extern "C" fn pend_sv_handler() {
    naked_asm!(
        "mrs r0, psp",
        "isb",
        "ldr r3, ={current_tcb}",
        "ldr r2, [r3]",
        "stmdb r0!, {{r4-r11}}",
        "str r0, [r2]",
        
        "stmdb sp!, {{r3, r14}}",
        "mov r0, #{max_pri}",
        "msr basepri, r0",
        "dsb",
        "isb",
        "bl {switch}",
        "mov r0, #0",
        "msr basepri, r0",
        "ldmia sp!, {{r3, r14}}",
        
        "ldr r1, [r3]",
        "ldr r0, [r1]",
        "ldmia r0!, {{r4-r11}}",
        "msr psp, r0",
        "isb",
        "bx r14",
        "nop",
        current_tcb = sym crate::rtos::kernel::scheduler::CURRENT_TCB,
        max_pri = const crate::rtos::kernel::types::MAX_SYSCALL_INTERRUPT_PRIORITY,
        switch = sym crate::rtos::kernel::scheduler::switch_context,
    )
}

#[export_name = "SysTick"]
pub unsafe extern "C" fn sys_tick_handler() {
    crate::rtos::kernel::scheduler::tick();
}
