//! x86-64 four-level page table (PML4 → PDPT → PD → PT, 4 KiB pages).
//!
//! Implements the IA-32e paging structures described in Intel SDM Vol 3A §4.5.

use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct PageTableFlags: u64 {
        const PRESENT       = 1 << 0;
        const WRITABLE      = 1 << 1;
        const USER          = 1 << 2;
        const WRITE_THROUGH = 1 << 3;
        const NO_CACHE      = 1 << 4;
        const ACCESSED      = 1 << 5;
        const DIRTY         = 1 << 6;
        const HUGE          = 1 << 7;
        const NO_EXECUTE    = 1 << 63;
    }
}

pub const PAGE_SIZE: usize = 4096;

/// Map a single 4 KiB page.
///
/// # Safety
/// `pml4` must point to a valid, 4-KiB-aligned PML4 table within the kernel's
/// identity-mapped physical region.
pub unsafe fn map_4k(pml4: *mut u64, virt: usize, phys: usize, flags: PageTableFlags) {
    let pml4_idx = (virt >> 39) & 0x1FF;
    let pdpt_idx = (virt >> 30) & 0x1FF;
    let pd_idx   = (virt >> 21) & 0x1FF;
    let pt_idx   = (virt >> 12) & 0x1FF;

    let pdpt = ensure_table(pml4, pml4_idx, flags);
    let pd   = ensure_table(pdpt, pdpt_idx, flags);
    let pt   = ensure_table(pd,   pd_idx,   flags);

    pt.add(pt_idx).write(phys as u64 | flags.bits());
}

unsafe fn ensure_table(parent: *mut u64, idx: usize, flags: PageTableFlags) -> *mut u64 {
    let entry = parent.add(idx).read();
    if entry & PageTableFlags::PRESENT.bits() != 0 {
        return (entry & !0xFFF) as *mut u64;
    }
    let table = alloc_zeroed_page();
    parent.add(idx).write(table as u64 | flags.bits());
    table
}

/// Allocate and zero a 4 KiB page for an intermediate page-table node.
unsafe fn alloc_zeroed_page() -> *mut u64 {
    let phys = mm::buddy::alloc(0)
        .expect("x86_64::paging: OOM allocating page table page");
    let ptr = phys as *mut u8;
    ptr.write_bytes(0, mm::buddy::PAGE_SIZE);
    phys as *mut u64
}
