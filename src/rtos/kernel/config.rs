use crate::rtos::types::TickType;


pub const MAX_PRIORITIES: usize = 5;
pub const TIME_SLICE: TickType = 10;
pub const USE_TIME_SLICING: bool = true;
pub const IDLE_SHOULD_YIELD: bool = true;
pub const IDLE_STACK_SIZE: usize = 256;
