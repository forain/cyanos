//! AArch64 page table management (4 KiB granule, 4-level translation).
//!
//! Implements the ARMv8-A VMSAv8-64 translation table format.
//! TTBR0_EL1 addresses user space; TTBR1_EL1 addresses the kernel.
//! We use a 48-bit VA space (4 levels, IA = 48 bits).
//!
//! MAIR_EL1 index mapping (set up in arch::init):
//!   0 = Normal memory (0xFF)  — used for all RAM mappings
//!   1 = Device nGnRnE (0x00)  — used for MMIO (NOCACHE flag)

use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct PageDescFlags: u64 {
        const VALID     = 1 << 0;
        const TABLE     = 1 << 1;  // 1 = table/page descriptor, 0 = block
        const ATTR_DEV  = 1 << 2;  // AttrIndx[0] — selects MAIR index 1 (device)
        const USER      = 1 << 6;  // AP[1]: EL0 accessible
        const RDONLY    = 1 << 7;  // AP[2]: read-only
        const INNER_SHR = 3 << 8;  // SH[1:0] = inner-shareable
        const AF        = 1 << 10; // Access Flag (must be set; else fault on first access)
        const NO_EXEC   = 1 << 54; // UXN / PXN
    }
}

/// Map a single 4 KiB page into the 4-level page table rooted at `pgd`.
///
/// Returns `true` on success, `false` if an intermediate page-table node
/// could not be allocated (OOM).  The caller must handle `false` gracefully
/// — in particular the page-fault handler must return `false` so the faulting
/// task receives a segfault rather than a kernel panic.
///
/// # Safety
/// `pgd` must point to a valid, 4-KiB-aligned Level-0 (PGD) page table that
/// lies within a region addressable without MMU (identity-mapped or physical
/// address space).
pub unsafe fn map_4k(pgd: *mut u64, virt: usize, phys: usize, flags: PageDescFlags) -> bool {
    let l0 = (virt >> 39) & 0x1FF;
    let l1 = (virt >> 30) & 0x1FF;
    let l2 = (virt >> 21) & 0x1FF;
    let l3 = (virt >> 12) & 0x1FF;

    let p1 = match ensure_table(pgd, l0) { Some(p) => p, None => return false };
    let p2 = match ensure_table(p1,  l1) { Some(p) => p, None => return false };
    let p3 = match ensure_table(p2,  l2) { Some(p) => p, None => return false };

    // L3 entry: page descriptor (bit 1 = 1, bit 0 = 1).
    p3.add(l3).write(phys as u64 | flags.bits() | 0b11);

    // Flush TLB for this address (inner-shareable broadcast).
    core::arch::asm!(
        "dsb ishst",
        "tlbi vaae1is, {va}",
        "dsb ish",
        "isb",
        va = in(reg) (virt >> 12) as u64,
        options(nostack)
    );
    true
}

/// Unmap a single 4 KiB page and invalidate its TLB entry.
///
/// # Safety
/// `pgd` must point to a valid PGD and `virt` must be 4-KiB aligned.
pub unsafe fn unmap_4k(pgd: *mut u64, virt: usize) {
    let l0 = (virt >> 39) & 0x1FF;
    let l1 = (virt >> 30) & 0x1FF;
    let l2 = (virt >> 21) & 0x1FF;
    let l3 = (virt >> 12) & 0x1FF;

    let e0 = pgd.add(l0).read();
    if e0 & PageDescFlags::VALID.bits() == 0 { return; }
    let p1 = (e0 & 0x0000_FFFF_FFFF_F000) as *mut u64;

    let e1 = p1.add(l1).read();
    if e1 & PageDescFlags::VALID.bits() == 0 { return; }
    let p2 = (e1 & 0x0000_FFFF_FFFF_F000) as *mut u64;

    let e2 = p2.add(l2).read();
    if e2 & PageDescFlags::VALID.bits() == 0 { return; }
    let p3 = (e2 & 0x0000_FFFF_FFFF_F000) as *mut u64;

    p3.add(l3).write(0);

    core::arch::asm!(
        "dsb ishst",
        "tlbi vaae1is, {va}",
        "dsb ish",
        "isb",
        va = in(reg) (virt >> 12) as u64,
        options(nostack)
    );
}

/// Returns `Some(ptr)` if the intermediate table at `parent[idx]` exists or
/// was just allocated, or `None` on OOM.
unsafe fn ensure_table(parent: *mut u64, idx: usize) -> Option<*mut u64> {
    let entry = parent.add(idx).read();
    if entry & PageDescFlags::VALID.bits() != 0 {
        // Table is already present; extract the physical address.
        return Some((entry & 0x0000_FFFF_FFFF_F000) as *mut u64);
    }
    let table = alloc_zeroed_page()?;
    parent.add(idx).write(
        table as u64 | PageDescFlags::TABLE.bits() | PageDescFlags::VALID.bits()
    );
    Some(table)
}

/// Allocate and zero a 4 KiB physical page for an intermediate page-table node.
/// Returns `None` on OOM instead of panicking.
unsafe fn alloc_zeroed_page() -> Option<*mut u64> {
    let phys = mm::buddy::alloc(0)?;
    let ptr = phys as *mut u8;
    ptr.write_bytes(0, mm::buddy::PAGE_SIZE);
    Some(phys as *mut u64)
}

// ── arch_tlb_shootdown_all ────────────────────────────────────────────────────

/// Broadcast a TLB invalidation for all user-space entries to all CPUs.
///
/// # SMP correctness requirement
///
/// `arch_set_page_table` only writes TTBR0_EL1 on the **current** CPU.  When a
/// page is unmapped on CPU A while other CPUs may have TLB entries for the same
/// virtual address (e.g. a task that ran on CPU B and is now scheduled on CPU A),
/// those stale entries must be flushed via an IPI-triggered shootdown.
///
/// **Current implementation**: single-CPU stub that flushes the local TLB only.
/// On a production SMP system this must:
///   1. Pause all CPUs sharing the same address space (IPI with type TLBI).
///   2. Execute `TLBI VMALLE1IS` on each CPU to flush ASID-tagged entries.
///   3. Resume the paused CPUs.
///
/// The `TLBI VAAE1IS` instruction used in `unmap_4k` already broadcasts via the
/// inner-shareable domain on properly configured systems (SH bits set), but
/// relying on ISH broadcast requires all CPUs to be in the same IS domain, which
/// must be verified at bringup.
#[no_mangle]
pub unsafe extern "C" fn arch_tlb_shootdown_all() {
    // SAFETY (single-CPU path): invalidate all ASID-tagged EL0 entries on this CPU.
    core::arch::asm!(
        "dsb ishst",
        "tlbi vmalle1is",   // broadcast across inner-shareable domain
        "dsb ish",
        "isb",
        options(nostack)
    );
}

// ── arch_set_page_table ───────────────────────────────────────────────────────

/// Load `root` into TTBR0_EL1.
///
/// If `root` is 0 the TTBR0 is cleared (safe — kernel code uses TTBR1_EL1).
/// Called by the scheduler immediately before every `cpu_switch_to` into a
/// user task, and with 0 on return to the scheduler idle loop.
#[no_mangle]
pub unsafe extern "C" fn arch_set_page_table(root: usize) {
    core::arch::asm!(
        "msr ttbr0_el1, {r}",
        "isb",
        r = in(reg) root as u64,
        options(nostack)
    );
}

// ── arch_alloc_page_table_root ────────────────────────────────────────────────

/// Allocate a zeroed 4 KiB page to serve as a process's TTBR0_EL1 root (PGD).
///
/// Returns the physical address of the page, or 0 on OOM.
/// Called by `sched::spawn_user` via an `extern "C"` declaration.
#[no_mangle]
pub unsafe extern "C" fn arch_alloc_page_table_root() -> usize {
    match mm::buddy::alloc(0) {
        Some(phys) => {
            (phys as *mut u8).write_bytes(0, mm::buddy::PAGE_SIZE);
            phys
        }
        None => 0,
    }
}

// ── arch_map_page / arch_unmap_page ──────────────────────────────────────────

/// Translate mm::PageFlags bits to AArch64 page-descriptor flags.
fn translate_flags(bits: u64) -> PageDescFlags {
    use mm::paging::PageFlags;
    let src = PageFlags::from_bits_truncate(bits);
    // Always-required bits for a valid page descriptor with access flag.
    let mut f = PageDescFlags::VALID | PageDescFlags::AF | PageDescFlags::INNER_SHR;
    if src.contains(PageFlags::USER)     { f |= PageDescFlags::USER; }
    if !src.contains(PageFlags::WRITABLE){ f |= PageDescFlags::RDONLY; }
    if !src.contains(PageFlags::EXECUTE) { f |= PageDescFlags::NO_EXEC; }
    if src.contains(PageFlags::NOCACHE)  { f |= PageDescFlags::ATTR_DEV; } // MAIR index 1
    f
}

#[no_mangle]
pub unsafe extern "C" fn arch_map_page(
    page_table_root: usize,
    virt: usize,
    phys: usize,
    flags: u64,
) -> bool {
    map_4k(page_table_root as *mut u64, virt, phys, translate_flags(flags))
}

#[no_mangle]
pub unsafe extern "C" fn arch_unmap_page(page_table_root: usize, virt: usize) {
    unmap_4k(page_table_root as *mut u64, virt);
}
