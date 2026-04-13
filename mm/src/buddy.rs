//! Buddy allocator — Linux-style power-of-two physical page allocator.
//!
//! See: linux/mm/page_alloc.c

use spin::Mutex;

pub const PAGE_SIZE: usize = 4096;
pub const MAX_ORDER: usize = 11; // 2^10 pages = 4 MiB max contiguous block.

/// A free list for one order level.
struct FreeList {
    head: Option<usize>, // physical address of first free block
}

impl FreeList {
    const fn empty() -> Self { Self { head: None } }
}

static FREE_LISTS: Mutex<[FreeList; MAX_ORDER]> = Mutex::new([const { FreeList::empty() }; MAX_ORDER]);

/// Initialise the buddy allocator with the available physical memory map.
/// Must be called before any allocation.
pub fn init() {
    // TODO: parse memory map from bootloader (e.g. multiboot2 / UEFI).
}

/// Allocate 2^order contiguous physical pages. Returns physical address or None.
pub fn alloc(order: usize) -> Option<usize> {
    assert!(order < MAX_ORDER);
    let mut lists = FREE_LISTS.lock();
    // Walk up from requested order looking for a free block.
    for o in order..MAX_ORDER {
        if let Some(addr) = lists[o].head.take() {
            // Split excess blocks back down.
            for split in (order..o).rev() {
                let buddy = addr + (PAGE_SIZE << split);
                lists[split].head = Some(buddy);
            }
            return Some(addr);
        }
    }
    None
}

/// Free 2^order contiguous pages starting at `addr`.
pub fn free(addr: usize, order: usize) {
    assert!(order < MAX_ORDER);
    let mut lists = FREE_LISTS.lock();
    let mut current = addr;
    let mut current_order = order;
    // Merge with buddy while possible.
    while current_order < MAX_ORDER - 1 {
        let buddy = current ^ (PAGE_SIZE << current_order);
        if lists[current_order].head == Some(buddy) {
            lists[current_order].head = None;
            current = current.min(buddy);
            current_order += 1;
        } else {
            break;
        }
    }
    lists[current_order].head = Some(current);
}
