//! Virtual Memory Manager — per-process address space descriptors.
//!
//! Analogous to Linux's `mm_struct` / `vm_area_struct`.
//!
//! Demand paging
//! -------------
//! `map_lazy()` records a VMA without allocating or installing any page-table
//! entries.  On the first access the CPU takes a page fault; the fault handler
//! calls `handle_user_page_fault(fault_va)` which allocates exactly one 4 KiB
//! page, zeroes it, and maps it into the page table.  Subsequent accesses to
//! other pages in the same VMA each trigger their own fault.  A lazy VMA may
//! span at most `MAX_LAZY_PAGES` pages; faults beyond that limit return `false`
//! (treated as a segfault) to prevent untracked allocations that cannot be freed.

use crate::paging::{PageFlags, map_page, unmap_page, tlb_shootdown_all};
use crate::buddy::{PAGE_SIZE, alloc as buddy_alloc, free as buddy_free};

/// Maximum number of individual pages tracked per lazy VMA.
///
/// A lazy VMA may span at most `MAX_LAZY_PAGES` pages.  Attempting to fault in
/// a page beyond this index is rejected with OOM (returns `false` from
/// `handle_user_page_fault`), which the kernel treats as a segmentation fault.
/// This prevents the previous silent leak where pages beyond this limit were
/// mapped but could never be freed.
///
/// To support larger anonymous regions increase this constant; it trades stack
/// space in each `VmaRegion` (8 bytes × MAX_LAZY_PAGES) for tracking capacity.
pub const MAX_LAZY_PAGES: usize = 64;

/// Represents a contiguous virtual memory region within an address space.
#[derive(Clone, Copy)]
pub struct VmaRegion {
    pub start: usize,
    pub end:   usize,   // exclusive
    /// For eager VMAs: physical base of the contiguous buddy allocation.
    /// For lazy VMAs: unused (see `lazy_pages`).
    pub phys:  usize,
    pub flags: PageFlags,
    /// True if physical pages are allocated lazily on first access.
    pub lazy:  bool,
    /// Per-page physical addresses for lazy VMAs (0 = not yet faulted in).
    /// Indexed by `(fault_va - start) / PAGE_SIZE`.
    pub lazy_pages: [usize; MAX_LAZY_PAGES],
    /// Number of entries in `lazy_pages` that have been filled.
    pub lazy_count: usize,
}

/// Per-process address space.
pub struct AddressSpace {
    pub page_table_root: usize,
    pub regions: [Option<VmaRegion>; 64],
}

impl Drop for AddressSpace {
    /// Unmap and free all VMAs, then release the page-table root page.
    ///
    /// Called automatically when the owning `Task` is dropped by the
    /// zombie-reaping path in `sched::run()`.  This is the authoritative
    /// cleanup path for per-process physical memory.
    fn drop(&mut self) {
        // Free all VMA backing pages.
        for slot in self.regions.iter_mut() {
            if let Some(region) = slot.take() {
                if region.lazy {
                    let max_idx = ((region.end - region.start) / PAGE_SIZE)
                        .min(MAX_LAZY_PAGES);
                    for i in 0..max_idx {
                        if region.lazy_pages[i] != 0 {
                            buddy_free(region.lazy_pages[i], 0);
                        }
                    }
                } else if region.phys != 0 {
                    let pages = (region.end - region.start) / PAGE_SIZE;
                    buddy_free(region.phys, pages_to_order(pages));
                }
            }
        }
        // Free the page-table root (PGD on AArch64, PML4 on x86-64).
        if self.page_table_root != 0 {
            buddy_free(self.page_table_root, 0);
        }
        // Flush stale TLB entries on all CPUs now that all mappings are gone.
        tlb_shootdown_all();
    }
}

impl AddressSpace {
    pub fn new(page_table_root: usize) -> Self {
        Self {
            page_table_root,
            regions: [None; 64],
        }
    }

    /// Map `size` bytes (rounded up to pages) at virtual address `virt`,
    /// backed by freshly allocated physical pages.
    ///
    /// Returns `true` on success, `false` if OOM or the VMA table is full.
    pub fn map(&mut self, virt: usize, size: usize, flags: PageFlags) -> bool {
        if size == 0 { return false; }

        // Find a free VMA slot.
        let slot = match self.regions.iter().position(|r| r.is_none()) {
            Some(i) => i,
            None    => return false,
        };

        // Align virt down and size up to page granularity.
        let virt  = virt & !(PAGE_SIZE - 1);
        let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        let end   = match virt.checked_add(pages * PAGE_SIZE) {
            Some(e) => e,
            None    => return false, // overflow → reject
        };

        // Reject if the new range overlaps any existing VMA.
        for r in self.regions.iter().filter_map(|r| r.as_ref()) {
            if virt < r.end && end > r.start { return false; }
        }
        let order = pages_to_order(pages);

        let phys = match buddy_alloc(order) {
            Some(p) => p,
            None    => return false,
        };

        // Zero the backing memory.
        unsafe { (phys as *mut u8).write_bytes(0, pages * PAGE_SIZE); }

        // Map each page.  If any individual mapping fails (OOM in page-table
        // node allocation), unmap the pages already installed, free the buddy
        // allocation, and report failure.
        for i in 0..pages {
            let ok = unsafe {
                map_page(
                    self.page_table_root,
                    virt + i * PAGE_SIZE,
                    phys + i * PAGE_SIZE,
                    flags,
                )
            };
            if !ok {
                // Roll back already-mapped pages.
                for j in 0..i {
                    unsafe { unmap_page(self.page_table_root, virt + j * PAGE_SIZE); }
                }
                buddy_free(phys, order);
                return false;
            }
        }

        self.regions[slot] = Some(VmaRegion {
            start: virt,
            end:   virt + pages * PAGE_SIZE,
            phys,
            flags,
            lazy: false,
            lazy_pages: [0; MAX_LAZY_PAGES],
            lazy_count: 0,
        });
        true
    }

    /// Reserve a virtual address range without allocating physical pages.
    ///
    /// Each page is allocated and mapped on the first access that faults into
    /// it.  Mirrors `mmap(PROT_…, MAP_ANONYMOUS | MAP_PRIVATE, …)` with no
    /// `MAP_POPULATE` flag.
    ///
    /// Returns `true` on success, `false` if the VMA table is full or the range
    /// overlaps an existing VMA.
    pub fn map_lazy(&mut self, virt: usize, size: usize, flags: PageFlags) -> bool {
        if size == 0 { return false; }

        let slot = match self.regions.iter().position(|r| r.is_none()) {
            Some(i) => i,
            None    => return false,
        };

        let virt  = virt & !(PAGE_SIZE - 1);
        let pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;
        let end   = match virt.checked_add(pages * PAGE_SIZE) {
            Some(e) => e,
            None    => return false, // overflow → reject
        };

        for r in self.regions.iter().filter_map(|r| r.as_ref()) {
            if virt < r.end && end > r.start { return false; }
        }

        self.regions[slot] = Some(VmaRegion {
            start: virt,
            end,
            phys: 0,
            flags,
            lazy: true,
            lazy_pages: [0; MAX_LAZY_PAGES],
            lazy_count: 0,
        });
        true
    }

    /// Handle a user-mode page fault at `fault_va`.
    ///
    /// Looks up the VMA that contains `fault_va`.  If the VMA is lazy and the
    /// faulting page has not been backed yet, allocates one 4 KiB physical page,
    /// zeroes it, and maps it into `self.page_table_root`.
    ///
    /// Returns `true` if the fault was handled (execution can resume), or `false`
    /// if `fault_va` is not within any VMA (segmentation fault).
    pub fn handle_user_page_fault(&mut self, fault_va: usize) -> bool {
        let page_va = fault_va & !(PAGE_SIZE - 1);

        // Find the VMA that covers the faulting address.
        let region = match self.regions.iter_mut().filter_map(|r| r.as_mut()).find(
            |r| fault_va >= r.start && fault_va < r.end
        ) {
            Some(r) => r,
            None    => return false, // not mapped at all → segfault
        };

        if !region.lazy {
            // The page should already be present; this is not a demand-paging
            // fault — likely a protection fault.  Signal as unhandled.
            return false;
        }

        // Compute the page index within this VMA.
        let page_idx = (page_va - region.start) / PAGE_SIZE;

        // If this page was already faulted in, it is a protection fault.
        if page_idx < MAX_LAZY_PAGES && region.lazy_pages[page_idx] != 0 {
            return false;
        }

        // Allocate one physical page for this fault.
        let phys = match buddy_alloc(0) {
            Some(p) => p,
            None    => return false, // OOM
        };
        unsafe { (phys as *mut u8).write_bytes(0, PAGE_SIZE); }

        // Map just the faulting page.  If the page-table walk itself runs out
        // of memory, free the backing page and return false (segfault).
        let mapped = unsafe {
            map_page(self.page_table_root, page_va, phys, region.flags)
        };
        if !mapped {
            buddy_free(phys, 0);
            return false;
        }

        // Reject pages beyond the tracking table capacity.  This branch is only
        // reached when `page_idx >= MAX_LAZY_PAGES` AND the map_page call above
        // succeeded (the `!mapped` early-return already handled map failure).
        // Mapping them would succeed but they could never be freed (silent leak),
        // so we unmap the freshly-installed PTE, free the physical page, and
        // return false to trigger a segfault.
        if page_idx >= MAX_LAZY_PAGES {
            unsafe { unmap_page(self.page_table_root, page_va); }
            buddy_free(phys, 0);
            return false;
        }

        // Track the physical address so unmap() can free it later.
        region.lazy_pages[page_idx] = phys;
        region.lazy_count += 1;

        true
    }

    /// Unmap `size` bytes starting at `virt` and free the backing pages.
    ///
    /// The `virt` address must match a VmaRegion start exactly.
    pub fn unmap(&mut self, virt: usize, size: usize) {
        if size == 0 { return; }
        let virt  = virt & !(PAGE_SIZE - 1);
        let _pages = (size + PAGE_SIZE - 1) / PAGE_SIZE;

        for slot in self.regions.iter_mut() {
            let region = match slot {
                Some(r) if r.start == virt => r,
                _ => continue,
            };

            // Unmap each page and free backing memory.
            if region.lazy {
                // Lazy VMA: free each individually tracked physical page.
                // Iterate by page count (not lazy_count) since pages may
                // fault in out of order, leaving gaps in lazy_pages.
                let max_idx = ((region.end - region.start) / PAGE_SIZE).min(MAX_LAZY_PAGES);
                for i in 0..max_idx {
                    if region.lazy_pages[i] != 0 {
                        unsafe { unmap_page(self.page_table_root, region.start + i * PAGE_SIZE); }
                        buddy_free(region.lazy_pages[i], 0);
                    }
                }
            } else {
                // Eager VMA: pages form a single contiguous buddy allocation.
                let region_pages = (region.end - region.start) / PAGE_SIZE;
                for i in 0..region_pages {
                    unsafe { unmap_page(self.page_table_root, region.start + i * PAGE_SIZE); }
                }
                let order = pages_to_order(region_pages);
                buddy_free(region.phys, order);
            }

            *slot = None;
            // Flush stale TLB entries on all CPUs after clearing PTEs.
            tlb_shootdown_all();
            return;
        }
    }

    /// Look up the VmaRegion that contains `virt`, if any.
    pub fn find(&self, virt: usize) -> Option<&VmaRegion> {
        self.regions.iter()
            .filter_map(|r| r.as_ref())
            .find(|r| virt >= r.start && virt < r.end)
    }
}

fn pages_to_order(pages: usize) -> usize {
    let mut order = 0;
    let mut cap   = 1usize;
    while cap < pages { cap <<= 1; order += 1; }
    order
}
