pub const MAX_PRIORITIES: usize = 5;

pub const USE_TIME_SLICING: bool = true;
pub const IDLE_SHOULD_YIELD: bool = true;

pub const IDLE_STACK_SIZE: usize = 256;

pub const CPU_FREQ: u32 = 72_000_000;
pub const TICK_RATE_HZ: u32 = 72_000;
pub const SYSTICK_CYCLE: u32 = CPU_FREQ / TICK_RATE_HZ;

pub const PORT_MAX_DELAY: usize = usize::MAX;
pub const MAX_SYSCALL_INTERRUPT_PRIORITY: u32 = 191;

