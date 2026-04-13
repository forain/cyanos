//! Virtual memory / page table management.
//!
//! Architecture-agnostic interface; arch crates provide the concrete
//! page-table walk (x86-64 4-level PT, AArch64 TTBR0/TTBR1, etc.).

use bitflags::bitflags;

bitflags! {
    /// Page mapping flags (architecture-agnostic).
    #[derive(Clone, Copy, Debug)]
    pub struct PageFlags: u64 {
        const PRESENT   = 1 << 0;
        const WRITABLE  = 1 << 1;
        const USER      = 1 << 2;
        const EXECUTE   = 1 << 3;
        const NOCACHE   = 1 << 4;
    }
}

/// Map a single virtual page to a physical frame in the given address space.
///
/// # Safety
/// `page_table_root` must point to a valid, writable page table root.
pub unsafe fn map_page(
    page_table_root: usize,
    virt: usize,
    phys: usize,
    flags: PageFlags,
) {
    // Architecture-specific implementation will live in arch/*/src/paging.rs
    let _ = (page_table_root, virt, phys, flags);
    todo!("arch-specific page table walk")
}

/// Unmap a virtual page.
pub unsafe fn unmap_page(page_table_root: usize, virt: usize) {
    let _ = (page_table_root, virt);
    todo!("arch-specific page table walk")
}
