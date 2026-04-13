//! AArch64 page table management (4 KiB granule, 4-level translation).

use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct PageDescFlags: u64 {
        const VALID     = 1 << 0;
        const TABLE     = 1 << 1;  // 1 = table descriptor, 0 = block
        const USER      = 1 << 6;
        const RDONLY    = 1 << 7;
        const INNER_SHR = 3 << 8;
        const AF        = 1 << 10; // Access flag — must be set or fault on first access
        const NO_EXEC   = 1 << 54;
    }
}

/// Map a 4 KiB page (TTBR0 address space — user/kernel depending on VA range).
///
/// # Safety
/// `pgd` must point to a valid 4-KiB-aligned Level-0 page table.
pub unsafe fn map_4k(pgd: *mut u64, virt: usize, phys: usize, flags: PageDescFlags) {
    let l0 = (virt >> 39) & 0x1FF;
    let l1 = (virt >> 30) & 0x1FF;
    let l2 = (virt >> 21) & 0x1FF;
    let l3 = (virt >> 12) & 0x1FF;

    let p1 = ensure_table(pgd, l0, flags);
    let p2 = ensure_table(p1, l1, flags);
    let p3 = ensure_table(p2, l2, flags);

    // L3 entry: page descriptor (bit 1 = 1, bit 0 = 1).
    p3.add(l3).write(phys as u64 | flags.bits() | 0b11);
}

unsafe fn ensure_table(parent: *mut u64, idx: usize, _flags: PageDescFlags) -> *mut u64 {
    let entry = parent.add(idx).read();
    if entry & PageDescFlags::VALID.bits() != 0 {
        return (entry & 0x0000_FFFF_FFFF_F000) as *mut u64;
    }
    let table = alloc_zeroed_page();
    parent.add(idx).write(table as u64 | PageDescFlags::TABLE.bits() | PageDescFlags::VALID.bits());
    table
}

unsafe fn alloc_zeroed_page() -> *mut u64 {
    static mut TEMP: [u64; 512] = [0u64; 512];
    core::ptr::addr_of_mut!(TEMP) as *mut u64
}
