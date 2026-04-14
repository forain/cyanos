//! AArch64 page table management (4 KiB granule, 4-level translation).
//!
//! Implements the ARMv8-A VMSAv8-64 translation table format.
//! TTBR0_EL1 addresses user space; TTBR1_EL1 addresses the kernel.
//! We use a 48-bit VA space (4 levels, IA = 48 bits).

use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct PageDescFlags: u64 {
        const VALID     = 1 << 0;
        const TABLE     = 1 << 1;  // 1 = table/page descriptor, 0 = block
        const USER      = 1 << 6;  // AP[1]: EL0 accessible
        const RDONLY    = 1 << 7;  // AP[2]: read-only
        const INNER_SHR = 3 << 8;  // SH[1:0] = inner-shareable
        const AF        = 1 << 10; // Access Flag (must be set; else fault on first access)
        const NO_EXEC   = 1 << 54; // UXN / PXN
    }
}

/// Map a single 4 KiB page into the 4-level page table rooted at `pgd`.
///
/// # Safety
/// `pgd` must point to a valid, 4-KiB-aligned Level-0 (PGD) page table that
/// lies within a region addressable without MMU (identity-mapped or physical
/// address space).
pub unsafe fn map_4k(pgd: *mut u64, virt: usize, phys: usize, flags: PageDescFlags) {
    let l0 = (virt >> 39) & 0x1FF;
    let l1 = (virt >> 30) & 0x1FF;
    let l2 = (virt >> 21) & 0x1FF;
    let l3 = (virt >> 12) & 0x1FF;

    let p1 = ensure_table(pgd, l0, flags);
    let p2 = ensure_table(p1,  l1, flags);
    let p3 = ensure_table(p2,  l2, flags);

    // L3 entry: page descriptor (bit 1 = 1, bit 0 = 1).
    p3.add(l3).write(phys as u64 | flags.bits() | 0b11);
}

unsafe fn ensure_table(parent: *mut u64, idx: usize, _flags: PageDescFlags) -> *mut u64 {
    let entry = parent.add(idx).read();
    if entry & PageDescFlags::VALID.bits() != 0 {
        // Table is already present; extract the physical address.
        return (entry & 0x0000_FFFF_FFFF_F000) as *mut u64;
    }
    let table = alloc_zeroed_page();
    parent.add(idx).write(
        table as u64 | PageDescFlags::TABLE.bits() | PageDescFlags::VALID.bits()
    );
    table
}

/// Allocate and zero a 4 KiB physical page for an intermediate page-table node.
unsafe fn alloc_zeroed_page() -> *mut u64 {
    let phys = mm::buddy::alloc(0)
        .expect("aarch64::paging: OOM allocating page table page");
    let ptr = phys as *mut u8;
    ptr.write_bytes(0, mm::buddy::PAGE_SIZE);
    phys as *mut u64
}
