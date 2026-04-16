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

extern "C" {
    /// Arch-provided: map `phys` at `virt` with the given flags in the page
    /// table rooted at `page_table_root`.  Returns `true` on success, `false`
    /// if an intermediate page-table node could not be allocated (OOM).
    /// Implemented by each arch crate and resolved at link time.
    fn arch_map_page(page_table_root: usize, virt: usize, phys: usize, flags: u64) -> bool;
    /// Arch-provided: remove the mapping for `virt` and flush the TLB entry.
    fn arch_unmap_page(page_table_root: usize, virt: usize);
    /// Arch-provided: broadcast TLB invalidation for all user-space entries to
    /// all CPUs (inner-shareable TLBI on AArch64; CR3 reload on x86-64).
    ///
    /// # SMP note
    /// The current stub only flushes the **local** CPU's TLB.  A full SMP
    /// implementation must also send an IPI to all other CPUs sharing the
    /// address space and wait for their acknowledgement before returning.
    fn arch_tlb_shootdown_all();
}

/// Map a single virtual page to a physical frame in the given address space.
///
/// Returns `true` on success, `false` if an intermediate page-table node
/// could not be allocated (OOM).
///
/// # Safety
/// `page_table_root` must point to a valid, writable page table root.
pub unsafe fn map_page(
    page_table_root: usize,
    virt: usize,
    phys: usize,
    flags: PageFlags,
) -> bool {
    arch_map_page(page_table_root, virt, phys, flags.bits())
}

/// Unmap a virtual page and flush the TLB entry on the current CPU.
///
/// After the last `unmap_page` call in a batch, callers **must** call
/// `tlb_shootdown_all()` to ensure no other CPU retains a stale translation.
pub unsafe fn unmap_page(page_table_root: usize, virt: usize) {
    arch_unmap_page(page_table_root, virt);
}

/// Invalidate all user-space TLB entries across all CPUs.
///
/// Call this after removing one or more page-table entries (via `unmap_page`)
/// to prevent other CPUs from using cached translations for the removed pages.
///
/// # SMP completeness
/// The underlying `arch_tlb_shootdown_all` currently flushes the local CPU
/// only.  On a multi-core system, additional IPI-based coordination is required
/// — see the arch implementation for details.
pub fn tlb_shootdown_all() {
    unsafe { arch_tlb_shootdown_all(); }
}
