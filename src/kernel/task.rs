#![allow(dead_code)]

use crate::kernel::{list::ListItem, types::{StackType, TickType, UBaseType}};

#[repr(C)]
pub struct TCB {
    pub top_of_stack: *mut StackType,
    pub state_list_item: ListItem<TCB>,
    pub stack: *mut StackType,
    pub name: [u8; 16],
    pub ticks_to_delay: TickType,
    pub priority: UBaseType
}

impl TCB {
    pub const fn new() -> Self {
        TCB {
            top_of_stack: core::ptr::null_mut(),
            state_list_item: ListItem::new(),
            stack: core::ptr::null_mut(),
            name: [0u8; 16],
            ticks_to_delay: 0,
            priority: 0,
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
    
        // initialize stack
        let top = stack.add(stack_depth - 1);
        self.top_of_stack = crate::port::cortex_m3::initialise_stack(
            top, task_fn, param
        );
    }
}