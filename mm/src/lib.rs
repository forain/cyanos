//! Memory Manager — physical and virtual memory subsystem.
//!
//! Mirrors Linux mm/ but restricted to the microkernel nucleus:
//!   - Physical frame allocator (buddy system)
//!   - Kernel virtual address space
//!   - Per-process page table management
//!   - Slab/slub-style object allocator

#![no_std]

pub mod buddy;
pub mod cow;
pub mod paging;
pub mod slab;
pub mod vmm;

/// Initialise all memory subsystems with a physical memory map.
/// Called once from `kernel_main` after boot info is parsed.
pub fn init_with_map(regions: &[boot::MemoryRegion]) {
    buddy::init_from_map(regions);
    slab::init();
}

/// Fallback init with no memory map (used in unit tests).
pub fn init() {
    buddy::init_from_map(&[]);
    slab::init();
}
