use super::Allocator;

const ALIGN: usize = 4;

pub struct Heap1<const N: usize> {
    memory: [u8; N],
    next: usize,
}

impl<const N: usize> Heap1<N> {
    pub const fn new() -> Self {
        Heap1 {
            memory: [0u8; N],
            next: 0,
        }
    }

    pub fn used(&self) -> usize {
        self.next
    }

    pub fn free_space(&self) -> usize {
        N.saturating_sub(self.next)
    }
}

impl<const N: usize> Allocator for Heap1<N> {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if size == 0 {
            return core::ptr::null_mut();
        }

        // align the current cursor, then reserve aligned_size bytes.
        let start = (self.next + ALIGN - 1) & !(ALIGN - 1);
        let aligned_size = (size + ALIGN - 1) & !(ALIGN - 1);

        let end = match start.checked_add(aligned_size) {
            Some(e) if e <= N => e,
            _ => return core::ptr::null_mut(), // oom
        };

        self.next = end;
        unsafe { self.memory.as_mut_ptr().add(start) }
    }

    // heap_1 never frees
    fn free(&mut self, _ptr: *mut u8) {}
}


// Wraps Heap1 in an UnsafeCell so it can live in a `static`
// Every method is unsafe: caller must hold a critical section.
//
// Example:
// ``` 
//   static HEAP: GlobalHeap1<4096> = GlobalHeap1::new();
//
//   unsafe {
//       port::enter_critical();
//       let p = HEAP.alloc(64);
//       port::exit_critical();
//   }
// ```

use core::cell::UnsafeCell;

pub struct GlobalHeap1<const N: usize> {
    inner: UnsafeCell<Heap1<N>>,
}

// SAFETY: single-core Cortex-M3; caller guarantees critical section.
unsafe impl<const N: usize> Sync for GlobalHeap1<N> {}

impl<const N: usize> GlobalHeap1<N> {
    pub const fn new() -> Self {
        GlobalHeap1 {
            inner: UnsafeCell::new(Heap1::new()),
        }
    }

    pub unsafe fn alloc(&self, size: usize) -> *mut u8 {
        (*self.inner.get()).alloc(size)
    }

    pub unsafe fn alloc_val<T>(&self, value: T) -> *mut T {
        (*self.inner.get()).alloc_val(value)
    }

    pub unsafe fn used(&self) -> usize {
        (*self.inner.get()).used()
    }

    pub unsafe fn free_space(&self) -> usize {
        (*self.inner.get()).free_space()
    }
}
