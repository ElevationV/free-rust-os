#![allow(dead_code)]

use crate::kernel::types::*;
use core::ptr::null_mut;

#[repr(C)]
pub struct ListItem<T> {
    pub value: TickType,
    pub next: *mut ListItem<T>,
    pub prev: *mut ListItem<T>,
    pub owner: *mut T,
    pub ctner: *mut List<T>,
}

#[repr(C)]
pub struct List<T> {
    pub items_num: UBaseType,
    pub index_ptr: *mut ListItem<T>,
    pub list_end: ListItem<T>,
}

impl<T> ListItem<T> {
    pub const fn new() -> Self {
        ListItem {
            value: PORT_MAX_DELAY,
            next: null_mut(),
            prev: null_mut(),
            owner: null_mut(),
            ctner: null_mut(),
        }
    }

    pub fn remove_in_list(&mut self) -> UBaseType {
        if self.ctner == null_mut() {
            return 0;
        }
        let list = self.ctner;
        unsafe {
            if &mut (*list).list_end as *mut ListItem<T> == self {
                return PORT_MAX_DELAY;
            }
            (*self.prev).next = self.next;
            (*self.next).prev = self.prev;
            if (*list).index_ptr == self {
                (*list).index_ptr = self.prev;
            }
            self.ctner = null_mut();
            (*list).items_num -= 1;
            (*list).items_num
        }
    }

    pub fn set_owner(&mut self, owner: *mut T) { self.owner = owner; }
    pub fn get_owner(&self) -> *mut T { self.owner }
    pub fn set_value(&mut self, val: TickType) { self.value = val; }
    pub fn get_value(&self) -> TickType { self.value }
}

impl<T> List<T> {
    pub const fn new() -> Self {
        List {
            items_num: 0,
            index_ptr: null_mut(),
            list_end: ListItem::new(),
        }
    }

    pub fn init(&mut self) {
        let end_ptr = &mut self.list_end as *mut ListItem<T>;
        self.index_ptr = end_ptr;
        self.list_end.prev = end_ptr;
        self.list_end.next = end_ptr;
        self.items_num = 0;
    }

    pub fn insert_end(&mut self, new_node: *mut ListItem<T>) {
        let end_ptr = &mut self.list_end as *mut ListItem<T>;
        unsafe {
            (*new_node).next = end_ptr;
            (*new_node).prev = self.list_end.prev;
            (*self.list_end.prev).next = new_node;
            self.list_end.prev = new_node;
            (*new_node).ctner = self;
            self.items_num += 1;
        }
    }

    pub fn insert_before_index(&mut self, new_node: *mut ListItem<T>) {
        let index = self.index_ptr;
        unsafe {
            (*new_node).next = index;
            (*new_node).prev = (*index).prev;
            (*(*index).prev).next = new_node;
            (*index).prev = new_node;
            (*new_node).ctner = self;
            self.items_num += 1;
        }
    }

    pub unsafe fn insert(&mut self, new_item: *mut ListItem<T>) {
        let value = (*new_item).value;
        let iterator: *mut ListItem<T> = if value == PORT_MAX_DELAY {
            &mut self.list_end as *mut ListItem<T>
        } else {
            let mut i = self.list_end.next;
            while (*i).value <= value {
                i = (*i).next;
            }
            i
        };
        (*new_item).next = iterator;
        (*new_item).prev = (*iterator).prev;
        (*(*iterator).prev).next = new_item;
        (*iterator).prev = new_item;
        (*new_item).ctner = self as *mut List<T>;
        self.items_num += 1;
    }

    pub fn get_next_entry(&mut self) -> *mut T {
        unsafe {
            self.index_ptr = (*self.index_ptr).next;
            if self.index_ptr == &mut self.list_end as *mut ListItem<T> {
                self.index_ptr = (*self.index_ptr).next;
            }
            (*self.index_ptr).owner
        }
    }
}