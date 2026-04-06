#![allow(dead_code)]

pub type TickType = u32;
pub type BaseType = i32;
pub type UBaseType = u32;
pub type StackType = u32;

pub const PORT_MAX_DELAY: TickType = u32::MAX;
pub const MAX_SYSCALL_INTERRUPT_PRIORITY: u32 = 191;
pub const MAX_PRIORITIES: usize = 5;
