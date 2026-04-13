//! Virtual Memory Manager — per-process address space descriptors.
//!
//! Analogous to Linux's `mm_struct` / `vm_area_struct`.

use crate::paging::PageFlags;

/// Represents a contiguous virtual memory region within an address space.
pub struct VmaRegion {
    pub start: usize,
    pub end: usize,
    pub flags: PageFlags,
}

/// Per-process address space.
pub struct AddressSpace {
    pub page_table_root: usize,
    pub regions: [Option<VmaRegion>; 64], // fixed-size for now; use a tree later.
}

impl AddressSpace {
    pub fn new(page_table_root: usize) -> Self {
        Self {
            page_table_root,
            regions: core::array::from_fn(|_| None),
        }
    }
}
