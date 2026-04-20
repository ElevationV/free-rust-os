#![allow(dead_code)]
#![allow(static_mut_refs)]

use crate::rtos::kernel::{
    list::List,
    task::{TCB, TaskState},
    types::{TickType, PORT_MAX_DELAY},
};
use crate::rtos::kernel::scheduler::{
    CURRENT_TCB, TICK_COUNT,
    CURRENT_DELAY_LIST, OVERFLOW_DELAY_LIST,
    READY_LISTS,
    record_ready_priority, clear_ready_priority,
};
use crate::rtos::port;

#[macro_export]
macro_rules! queue_of {
    ($T:ty, $CAP:expr) => {
        $crate::rtos::itc::queue::Queue<
            { core::mem::size_of::<$T>() },
            $CAP,
            { core::mem::size_of::<$T>() * $CAP },
        >
    };
}

#[macro_export]
macro_rules! queue_handle_of {
    ($T:ty, $CAP:expr) => {
        $crate::rtos::itc::queue::QueueHandle<
            $T,
            { core::mem::size_of::<$T>() },
            $CAP,
            { core::mem::size_of::<$T>() * $CAP },
        >
    };
}

pub struct Queue<
    const ITEM_SIZE: usize,
    const CAPACITY: usize,
    const STORAGE_SIZE: usize,
> {
    storage: [u8; STORAGE_SIZE],
    head: usize,
    tail: usize,
    count: usize,
    send_wait_list: List<TCB>,
    recv_wait_list: List<TCB>,
}

unsafe impl<const IS: usize, const CAP: usize, const SS: usize> Sync
    for Queue<IS, CAP, SS> {}
unsafe impl<const IS: usize, const CAP: usize, const SS: usize> Send
    for Queue<IS, CAP, SS> {}

impl<const ITEM_SIZE: usize, const CAPACITY: usize, const STORAGE_SIZE: usize>
    Queue<ITEM_SIZE, CAPACITY, STORAGE_SIZE>
{
    const CHECK: () = assert!(
        STORAGE_SIZE == ITEM_SIZE * CAPACITY,
        "Queue: STORAGE_SIZE must equal ITEM_SIZE * CAPACITY"
    );

    pub const fn new() -> Self {
        let _ = Self::CHECK;
        Queue {
            storage: [0u8; STORAGE_SIZE],
            head: 0,
            tail: 0,
            count: 0,
            send_wait_list: List::new(),
            recv_wait_list: List::new(),
        }
    }

    pub fn init(&mut self) {
        self.send_wait_list.init();
        self.recv_wait_list.init();
    }

    pub fn len(&self)      -> usize { self.count }
    pub fn is_empty(&self) -> bool  { self.count == 0 }
    pub fn is_full(&self)  -> bool  { self.count == CAPACITY }

    pub unsafe fn send(&mut self, item_ptr: *const u8, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            if self.count < CAPACITY {
                let dst = self.storage.as_mut_ptr().add(self.tail * ITEM_SIZE);
                core::ptr::copy_nonoverlapping(item_ptr, dst, ITEM_SIZE);
                self.tail = (self.tail + 1) % CAPACITY;
                self.count += 1;

                if self.recv_wait_list.items_num > 0 {
                    Self::wake_waiter(&raw mut self.recv_wait_list);
                }
                port::exit_critical();
                return true;
            }

            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            Self::place_on_event_list(&raw mut self.send_wait_list, remaining);
            port::exit_critical();
            port::task_yield();
            port::instruction_sync();

            // Non-null ctner means give() did NOT remove us -> timed out
            if !(*CURRENT_TCB).event_list_item.ctner.is_null() {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout { return false; }
                remaining = timeout - elapsed;
            }
        }
    }

    pub unsafe fn send_to_front(
        &mut self,
        item_ptr: *const u8,
        timeout: TickType,
    ) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            if self.count < CAPACITY {
                // Step head back one slot (wrapping) and write there
                self.head = (self.head + CAPACITY - 1) % CAPACITY;
                let dst = self.storage.as_mut_ptr().add(self.head * ITEM_SIZE);
                core::ptr::copy_nonoverlapping(item_ptr, dst, ITEM_SIZE);
                self.count += 1;

                if self.recv_wait_list.items_num > 0 {
                    Self::wake_waiter(&raw mut self.recv_wait_list);
                }
                port::exit_critical();
                return true;
            }

            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            Self::place_on_event_list(&raw mut self.send_wait_list, remaining);
            port::exit_critical();
            port::task_yield();
            port::instruction_sync();

            if !(*CURRENT_TCB).event_list_item.ctner.is_null() {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout { return false; }
                remaining = timeout - elapsed;
            }
        }
    }

    pub unsafe fn receive(&mut self, out_ptr: *mut u8, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            if self.count > 0 {
                let src = self.storage.as_ptr().add(self.head * ITEM_SIZE);
                core::ptr::copy_nonoverlapping(src, out_ptr, ITEM_SIZE);
                self.head = (self.head + 1) % CAPACITY;
                self.count -= 1;

                if self.send_wait_list.items_num > 0 {
                    Self::wake_waiter(&raw mut self.send_wait_list);
                }
                port::exit_critical();
                return true;
            }

            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            Self::place_on_event_list(&raw mut self.recv_wait_list, remaining);
            port::exit_critical();
            port::task_yield();
            port::instruction_sync();

            if !(*CURRENT_TCB).event_list_item.ctner.is_null() {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout { return false; }
                remaining = timeout - elapsed;
            }
        }
    }

    pub unsafe fn peek(&mut self, out_ptr: *mut u8, timeout: TickType) -> bool {
        let entry_tick = TICK_COUNT;
        let mut remaining = timeout;

        loop {
            port::enter_critical();

            if self.count > 0 {
                let src = self.storage.as_ptr().add(self.head * ITEM_SIZE);
                core::ptr::copy_nonoverlapping(src, out_ptr, ITEM_SIZE);
                // head / count unchanged — item stays in queue.
                port::exit_critical();
                return true;
            }

            if remaining == 0 {
                port::exit_critical();
                return false;
            }

            Self::place_on_event_list(&raw mut self.recv_wait_list, remaining);
            port::exit_critical();
            port::task_yield();
            port::instruction_sync();

            if !(*CURRENT_TCB).event_list_item.ctner.is_null() {
                (*CURRENT_TCB).event_list_item.remove_in_list();
                return false;
            }

            if timeout != PORT_MAX_DELAY {
                let elapsed = TICK_COUNT.wrapping_sub(entry_tick);
                if elapsed >= timeout { return false; }
                remaining = timeout - elapsed;
            }
        }
    }

    unsafe fn place_on_event_list(wait_list: *mut List<TCB>, timeout: TickType) {
        let tcb = CURRENT_TCB;
        let priority = (*tcb).priority;

        // Insert into the event list, highest priority first
        (*tcb).event_list_item.value = PORT_MAX_DELAY - priority;
        (*wait_list).insert(&raw mut (*tcb).event_list_item);

        // Insert into the delay list by absolute wake time
        let (wake_time, overflowed) = TICK_COUNT.overflowing_add(timeout);
        (*tcb).ticks_to_delay = wake_time;
        (*tcb).state_list_item.value = wake_time;

        // Remove from the ready list
        (*tcb).state_list_item.remove_in_list();
        if READY_LISTS[priority as usize].items_num == 0 {
            clear_ready_priority(priority);
        }

        if overflowed {
            (*OVERFLOW_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        } else {
            (*CURRENT_DELAY_LIST).insert(&raw mut (*tcb).state_list_item);
        }

        (*tcb).state = TaskState::Delayed;
    }

    unsafe fn wake_waiter(wait_list: *mut List<TCB>) {
        let end_ptr = &raw mut (*wait_list).list_end;
        let item    = (*end_ptr).next;
        let tcb     = (*item).owner as *mut TCB;

        (*item).remove_in_list();

        if (*tcb).state == TaskState::Delayed {
            (*tcb).state_list_item.remove_in_list();
        }

        let priority = (*tcb).priority;
        (*tcb).state_list_item.value = priority as TickType;
        READY_LISTS[priority as usize].insert_end(&raw mut (*tcb).state_list_item);
        record_ready_priority(priority);
        (*tcb).state = TaskState::Ready;

        if priority >= (*CURRENT_TCB).priority {
            port::task_yield();
        }
    }
}

pub struct QueueHandle<
    T: Copy,
    const ITEM_SIZE: usize,
    const CAPACITY: usize,
    const STORAGE_SIZE: usize,
> {
    inner: Queue<ITEM_SIZE, CAPACITY, STORAGE_SIZE>,
    _marker: core::marker::PhantomData<T>,
}

impl<T: Copy, const ITEM_SIZE: usize, const CAPACITY: usize, const STORAGE_SIZE: usize>
    QueueHandle<T, ITEM_SIZE, CAPACITY, STORAGE_SIZE>
{
    const CHECK: () = assert!(
        ITEM_SIZE == core::mem::size_of::<T>(),
        "QueueHandle: ITEM_SIZE must equal core::mem::size_of::<T>()"
    );

    pub const fn new() -> Self {
        let _ = Self::CHECK;
        QueueHandle {
            inner: Queue::new(),
            _marker: core::marker::PhantomData,
        }
    }

    pub fn init(&mut self) { self.inner.init(); }

    pub unsafe fn send_t(&mut self, item: T, timeout: TickType) -> bool {
        self.inner.send(&item as *const T as *const u8, timeout)
    }

    pub unsafe fn receive_t(&mut self, out: &mut T, timeout: TickType) -> bool {
        self.inner.receive(out as *mut T as *mut u8, timeout)
    }

    pub unsafe fn peek_t(&mut self, out: &mut T, timeout: TickType) -> bool {
        self.inner.peek(out as *mut T as *mut u8, timeout)
    }

    pub unsafe fn send_to_front_t(&mut self, item: T, timeout: TickType) -> bool {
        self.inner.send_to_front(&item as *const T as *const u8, timeout)
    }

    pub fn len(&self)      -> usize { self.inner.len() }
    pub fn is_empty(&self) -> bool  { self.inner.is_empty() }
    pub fn is_full(&self)  -> bool  { self.inner.is_full() }
}