//! Slab allocator — fixed-size kernel object cache (à la Linux SLUB).

pub fn init() {
    // TODO: pre-warm caches for common kernel objects (Task, Port, …).
}

/// Allocate a slab object of `size` bytes. Returns a raw pointer or panics.
pub fn alloc(_size: usize) -> *mut u8 {
    todo!("slab allocator")
}

/// Return a slab object to its cache.
///
/// # Safety
/// `ptr` must have been returned by `alloc` with the same `size`.
pub unsafe fn free(_ptr: *mut u8, _size: usize) {}
