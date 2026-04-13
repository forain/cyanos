//! Memory Manager — physical and virtual memory subsystem.
//!
//! Mirrors Linux mm/ but restricted to the microkernel nucleus:
//!   - Physical frame allocator (buddy system)
//!   - Kernel virtual address space
//!   - Per-process page table management
//!   - Slab/slub-style object allocator

#![no_std]

pub mod buddy;
pub mod paging;
pub mod slab;
pub mod vmm;

/// Initialise all memory subsystems. Called once from `kernel_main`.
pub fn init() {
    buddy::init();
    slab::init();
}
