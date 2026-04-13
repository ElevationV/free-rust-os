mod heap_1;

pub trait Allocator {
    // allocate `size` bytes
    fn alloc(&mut self, size: usize) -> *mut u8;
 
    // allocate space for `T`, and write it into memory
    // returns null on OOM.
    fn alloc_val<T>(&mut self, value: T) -> *mut T {
        let ptr = self.alloc(core::mem::size_of::<T>()) as *mut T;
        if ptr.is_null() {
            return core::ptr::null_mut();
        }
        unsafe { 
            ptr.write(value) 
        };
        ptr
    }
 
    fn free(&mut self, ptr: *mut u8);
}
 