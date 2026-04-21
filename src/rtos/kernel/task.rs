#![allow(dead_code)]

use super::list::ListItem;
use super::types::{StackType, TickType, UBaseType};
use crate::rtos::port;

#[derive(Clone, Copy, PartialEq)]
pub enum TaskState{
    Ready,
    Running,
    Delayed,
    Suspended, 
    None
}

#[repr(C)]
pub struct TCB {
    pub top_of_stack: *mut StackType,
    // state list, such as READY_LISTS[], *_DELAY_LIST, SUSPENDED_LIST ..
    pub state_list_item: ListItem<TCB>,
    // event list, such as pended by mutex, seamphore ..
    pub event_list_item: ListItem<TCB>,
    pub stack: *mut StackType,
    pub name: [u8; 16],
    pub ticks_to_delay: TickType,
    // current priority, may change because of inherit
    pub priority: UBaseType,
    // base priority, not change unless necessary
    pub base_priority: UBaseType,
    pub state: TaskState
}

impl TCB {
    pub const fn new() -> Self {
        TCB {
            top_of_stack: core::ptr::null_mut(),
            state_list_item: ListItem::new(),
            event_list_item: ListItem::new(),
            stack: core::ptr::null_mut(),
            name: [0u8; 16],
            ticks_to_delay: 0,
            priority: 0, 
            base_priority: 0,
            state: TaskState::None 
        }
    }
    
    pub unsafe fn init(
        &mut self,
        task_fn: unsafe extern "C" fn(*mut ()),
        param: *mut (),
        stack: *mut StackType,
        stack_depth: usize,
        priority: UBaseType,
        name: &str,
    ) {
        let self_ptr = self as *mut TCB;
        
        self.stack = stack;
        self.priority = priority;
        self.ticks_to_delay = 0;
    
        // set name
        let name_bytes = name.as_bytes();
        let len = name_bytes.len().min(15);
        self.name[..len].copy_from_slice(&name_bytes[..len]);
        self.name[len] = 0;
    
        // initialize list node
        self.state_list_item = ListItem::new();
        self.state_list_item.owner = self_ptr;
        
        // 
        self.base_priority = priority;
        self.event_list_item = ListItem::new();
        self.event_list_item.owner = self_ptr;
    
        // initialize stack
        for i in 0..stack_depth {
            stack.add(i).write(port::STACK_FILL_BYTE);
        }
        let top = stack.add(stack_depth - 1);
        self.top_of_stack = port::initialise_stack(
            top, task_fn, param
        );
    }
}