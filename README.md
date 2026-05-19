# FreeRusTOS

A lightweight Real-Time Operating System (RTOS) kernel written in Rust, targeting ARM Cortex-M3 and Cortex-M7 microcontrollers. The design is inspired by FreeRTOS and demonstrates how core RTOS primitives — scheduling, synchronization, and memory management — can be implemented safely and efficiently in `no_std` Rust.

---

## Features

- **Preemptive priority-based scheduler** with up to 5 configurable priority levels
- **Time-slicing** for equal-priority tasks (1-tick quantum)
- **Task management**: create, delay, suspend, resume, and abort-delay
- **Inter-Task Communication (ITC)**
  - Recursive mutex with **priority inheritance**
  - Binary and counting **semaphores**
  - Type-safe **message queues** with send-to-front support
- **Dual delay lists** to handle tick-count overflow correctly
- **Bump allocator** (`Heap1`) for deterministic, no-fragmentation allocation
- **Stack overflow detection** via fill-byte sentinel checks
- **Cortex-M7 FPU support**: full context save/restore of `s0–s31` and FPSCR
- **Critical sections** via `BASEPRI` (not `PRIMASK`), leaving high-priority IRQs unmasked


---

## Configuration

All tunable parameters live in `src/rtos/kernel/config.rs`:

| Constant | Default | Description |
|---|---|---|
| `MAX_PRIORITIES` | `5` | Number of priority levels (0 = lowest) |
| `USE_TIME_SLICING` | `true` | Round-robin among equal-priority tasks |
| `IDLE_SHOULD_YIELD` | `true` | Idle task yields immediately |
| `IDLE_STACK_SIZE` | `256` | Idle task stack depth (words) |
| `CPU_FREQ` | `72_000_000` | CPU frequency in Hz |
| `TICK_RATE_HZ` | `72_000` | Scheduler tick rate in Hz |
| `MAX_SYSCALL_INTERRUPT_PRIORITY` | `191` | IRQs above this priority are never masked |
| `PORT_MAX_DELAY` | `usize::MAX` | Block indefinitely |

Select the target chip via Cargo features:

```toml
[features]
cortex-m3 = []
cortex-m7 = []
```

---

## Getting Started

### Prerequisites

- Rust nightly toolchain with the appropriate target:
  ```sh
  rustup target add thumbv7m-none-eabi    # Cortex-M3
  rustup target add thumbv7em-none-eabihf # Cortex-M7 with FPU
  ```
- A `memory.x` linker script matching your chip's flash/RAM layout
- `cargo-binutils` or a suitable runner configured in `.cargo/config.toml`

### Build

```sh
# Cortex-M3
cargo build --release --features cortex-m3 --target thumbv7m-none-eabi

# Cortex-M7
cargo build --release --features cortex-m7 --target thumbv7em-none-eabihf
```

### Minimal example

```rust
#![no_std]
#![no_main]

use rtos::{kernel::scheduler, port};

static mut TASK_A_STACK: [usize; 256] = [0; 256];
static mut TASK_A_TCB: scheduler::TCB = scheduler::TCB::new();

unsafe extern "C" fn task_a(_param: *mut ()) {
    loop {
        // do work …
        scheduler::task_delay(100); // delay 100 ticks
    }
}

#[no_mangle]
pub unsafe extern "C" fn main() -> ! {
    scheduler::init();

    scheduler::create_task(
        task_a, "TaskA", 2,
        TASK_A_STACK.as_mut_ptr(),
        TASK_A_STACK.len(),
        &raw mut TASK_A_TCB,
    );

    scheduler::start(); // never returns
    loop {}
}
```

---

## Key Design Decisions

### Priority bitmap scheduler

`TOP_READY_PRIORITY` is a bitmask where each bit represents a non-empty ready queue. The highest ready priority is found in O(1) with `31 - leading_zeros()`, matching the CLZ-based approach used in FreeRTOS on ARM.

### Dual delay lists

Two `List<TCB>` instances — `CURRENT_DELAY_LIST` and `OVERFLOW_DELAY_LIST` — are swapped when `TICK_COUNT` wraps around zero. Tasks whose wake time crosses the overflow boundary are placed in the overflow list; after the swap they become the current list automatically, with no need to re-sort or re-insert any task.

### Priority inheritance in Mutex

When a high-priority task blocks on a mutex owned by a lower-priority task, `priority_inherit` raises the owner's priority and repositions it in the correct ready queue. When the mutex is released, `priority_disinherit` restores `base_priority`. This prevents priority inversion without a full inheritance chain.

### Intrusive linked list

`ListItem<T>` is embedded directly inside `TCB` (one for state, one for events). No heap allocation is needed for list membership. The `ctner` back-pointer lets any item remove itself from its owning list in O(1).

### BASEPRI-based critical sections

`enter_critical` / `exit_critical` use `BASEPRI` to mask only interrupts with a numeric priority ≥ `MAX_SYSCALL_INTERRUPT_PRIORITY`. Interrupts with a lower numeric value (higher hardware priority) remain active, enabling hard real-time ISRs alongside the RTOS.

### Cortex-M7 FPU context

With lazy stacking disabled (`LSPEN = 0`), every context switch unconditionally saves and restores `s16–s31` via `vstmdb` / `vldmia` in the PendSV handler. The hardware automatically saves `s0–s15` and FPSCR on exception entry, giving a complete and deterministic FPU context switch.

---

## ITC API

All ITC operations are `unsafe` because they access shared kernel state. Callers must not invoke them from within a critical section.

### Mutex

```rust
static mut MTX: Mutex = Mutex::new();

unsafe {
    MTX.init();

    if MTX.take(PORT_MAX_DELAY) {   // block until acquired
        // critical section …
        MTX.give();
    }
}
```

### Semaphore

```rust
static mut SEM: Semaphore = Semaphore::new_counting(4, 0);

unsafe {
    SEM.init();
    SEM.give();                     // signal from ISR or another task
    SEM.take(PORT_MAX_DELAY);       // wait for signal
}
```

### Queue

```rust
// Declare a queue that holds up to 8 u32 values
static mut Q: queue_of!(u32, 8) = Queue::new();

unsafe {
    Q.init();
    Q.send(&42u32 as *const u32 as *const u8, PORT_MAX_DELAY);

    let mut val: u32 = 0;
    Q.receive(&mut val as *mut u32 as *mut u8, PORT_MAX_DELAY);
}

// Type-safe wrapper
static mut QH: queue_handle_of!(u32, 8) = QueueHandle::new();

unsafe {
    QH.init();
    QH.send_t(42u32, PORT_MAX_DELAY);

    let mut val: u32 = 0;
    QH.receive_t(&mut val, PORT_MAX_DELAY);
}
```

---

## Heap Allocator

`Heap1` is a bump (linear) allocator — memory is never freed. It is suitable for one-shot startup allocation of stacks, TCBs, and other fixed-size objects.

```rust
use rtos::heap::heap_1::GlobalHeap1;

static HEAP: GlobalHeap1<4096> = GlobalHeap1::new();

unsafe {
    port::enter_critical();
    let ptr: *mut u32 = HEAP.alloc_val(0u32);
    port::exit_critical();
}
```

---

## Limitations

- **No dynamic memory freeing**: `Heap1::free` is a no-op by design.
- **Fixed priority count**: `MAX_PRIORITIES` is a compile-time constant; changing it requires a rebuild.
- **Single-core only**: no SMP support.
- **No MPU integration**: stack overflow detection relies on fill-byte sentinels, not hardware memory protection.
- **`unsafe` throughout**: the kernel internals use raw pointers extensively; incorrect use from application code can cause undefined behaviour.

---

## License

This project is provided for educational purposes. See individual source files for any licence notices.