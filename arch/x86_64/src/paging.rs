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
/// Returns `true` on success, `false` if an intermediate page-table node
/// could not be allocated (OOM).
///
/// # Safety
/// `pml4` must point to a valid, 4-KiB-aligned PML4 table within the kernel's
/// identity-mapped physical region.
pub unsafe fn map_4k(pml4: *mut u64, virt: usize, phys: usize, flags: PageTableFlags) -> bool {
    let pml4_idx = (virt >> 39) & 0x1FF;
    let pdpt_idx = (virt >> 30) & 0x1FF;
    let pd_idx   = (virt >> 21) & 0x1FF;
    let pt_idx   = (virt >> 12) & 0x1FF;

    let pdpt = match ensure_table(pml4, pml4_idx, flags) { Some(p) => p, None => return false };
    let pd   = match ensure_table(pdpt, pdpt_idx, flags) { Some(p) => p, None => return false };
    let pt   = match ensure_table(pd,   pd_idx,   flags) { Some(p) => p, None => return false };

    pt.add(pt_idx).write(phys as u64 | flags.bits());
    true
}

/// Unmap a single 4 KiB page and flush the TLB entry.
///
/// # Safety
/// `pml4` must point to a valid PML4 and `virt` must be 4-KiB aligned.
pub unsafe fn unmap_4k(pml4: *mut u64, virt: usize) {
    let pml4_idx = (virt >> 39) & 0x1FF;
    let pdpt_idx = (virt >> 30) & 0x1FF;
    let pd_idx   = (virt >> 21) & 0x1FF;
    let pt_idx   = (virt >> 12) & 0x1FF;

    let pdpt_entry = pml4.add(pml4_idx).read();
    if pdpt_entry & PageTableFlags::PRESENT.bits() == 0 { return; }
    let pdpt = (pdpt_entry & !0xFFF) as *mut u64;

    let pd_entry = pdpt.add(pdpt_idx).read();
    if pd_entry & PageTableFlags::PRESENT.bits() == 0 { return; }
    let pd = (pd_entry & !0xFFF) as *mut u64;

    let pt_entry = pd.add(pd_idx).read();
    if pt_entry & PageTableFlags::PRESENT.bits() == 0 { return; }
    let pt = (pt_entry & !0xFFF) as *mut u64;

    pt.add(pt_idx).write(0);

    // Flush the TLB entry for this virtual address.
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!("invlpg [{addr}]", addr = in(reg) virt, options(nostack));
}

/// Ensure an intermediate page-table node exists at `parent[idx]`, creating
/// it with a zeroed page if absent.  Returns `None` on OOM.  NX is stripped
/// from intermediate entries because it applies at the level it is set and
/// would block execution in the entire region covered by that entry.
unsafe fn ensure_table(parent: *mut u64, idx: usize, flags: PageTableFlags) -> Option<*mut u64> {
    let entry = parent.add(idx).read();
    if entry & PageTableFlags::PRESENT.bits() != 0 {
        return Some((entry & !0xFFF) as *mut u64);
    }
    let table = alloc_zeroed_page()?;
    // Strip NO_EXECUTE from intermediate entries; keep only P/W/U for the walk.
    let intermediate_flags = flags & (PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER);
    parent.add(idx).write(table as u64 | intermediate_flags.bits());
    Some(table)
}

/// Allocate and zero a 4 KiB page for an intermediate page-table node.
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
/// `arch_set_page_table` only writes CR3 on the **current** CPU.  On SMP,
/// unmapping a page on CPU A while other CPUs may have cached translations for
/// the same virtual address requires a TLB shootdown IPI.
///
/// **Current implementation**: single-CPU stub that reloads CR3 to flush the
/// local TLB only.
/// On a production SMP system this must:
///   1. Collect the set of CPUs running threads that share the affected page table.
///   2. Send an IPI (e.g. APIC vector 0xFE) to those CPUs.
///   3. Each receiving CPU executes `invlpg` or reloads CR3.
///   4. Wait for all CPUs to acknowledge before returning.
#[no_mangle]
pub unsafe extern "C" fn arch_tlb_shootdown_all() {
    // Reload CR3 to flush local TLB; on SMP an IPI to other CPUs is also needed.
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "mov {tmp}, cr3",
        "mov cr3, {tmp}",
        tmp = out(reg) _,
        options(nostack)
    );
}

// ── arch_set_page_table ───────────────────────────────────────────────────────

/// Load `root` into CR3.
///
/// If `root` is 0 we leave CR3 unchanged — the kernel identity map stays
/// active and there is no user-space mapping to switch away from.
/// Called by the scheduler immediately before every `cpu_switch_to` into a
/// user task, and with 0 on return to the scheduler idle loop.
#[no_mangle]
pub unsafe extern "C" fn arch_set_page_table(root: usize) {
    if root != 0 {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!(
            "mov cr3, {r}",
            r = in(reg) root as u64,
            options(nostack)
        );
    }
}

// ── arch_alloc_page_table_root ────────────────────────────────────────────────

/// Allocate a zeroed 4 KiB page to serve as a process's PML4 root.
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
// Resolved at link time by mm::paging — no circular crate dependency.

/// Translate mm::PageFlags bits to x86-64 page-table flags.
fn translate_flags(bits: u64) -> PageTableFlags {
    use mm::paging::PageFlags;
    let src = PageFlags::from_bits_truncate(bits);
    let mut f = PageTableFlags::empty();
    if src.contains(PageFlags::PRESENT)  { f |= PageTableFlags::PRESENT; }
    if src.contains(PageFlags::WRITABLE) { f |= PageTableFlags::WRITABLE; }
    if src.contains(PageFlags::USER)     { f |= PageTableFlags::USER; }
    if src.contains(PageFlags::NOCACHE)  { f |= PageTableFlags::NO_CACHE; }
    // NO_EXECUTE if EXECUTE is NOT requested.
    if !src.contains(PageFlags::EXECUTE) { f |= PageTableFlags::NO_EXECUTE; }
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
