//! Capability table — lightweight per-process token → kernel-object mapping.
//!
//! Capability tokens are opaque `u32` values handed to user-space programs.
//! The kernel resolves them back to kernel objects (ports, VMOs, file descriptors)
//! via the `CapTable` associated with the current process.
//!
//! Used by `mmap(fd_cap, ...)` and VFS IPC messages to reference kernel objects
//! without exposing raw kernel pointers to user space.

#![no_std]

use spin::Mutex;

/// Maximum number of capabilities per process.
pub const MAX_CAPS: usize = 1024;

/// Kind of kernel object a capability refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapKind {
    /// An IPC port ID.
    Port(u32),
    /// A virtual memory object (physical address range for file-backed mmap).
    Vmo(u64),
    /// A file descriptor number (used inside the VFS server).
    FileDesc(u32),
}

/// One capability slot.
#[derive(Clone, Copy)]
struct CapSlot {
    /// Token value (`u32::MAX` = free).
    token: u32,
    kind:  CapKind,
}

impl CapSlot {
    const fn free() -> Self {
        Self { token: u32::MAX, kind: CapKind::Port(0) }
    }
    fn is_free(&self) -> bool { self.token == u32::MAX }
}

/// Per-process capability table.
pub struct CapTable {
    slots:    [CapSlot; MAX_CAPS],
    next_tok: u32,
}

impl CapTable {
    pub const fn new() -> Self {
        Self {
            slots:    [const { CapSlot::free() }; MAX_CAPS],
            next_tok: 0,
        }
    }

    /// Allocate a new capability for `kind`.  Returns the token on success.
    pub fn alloc_cap(&mut self, kind: CapKind) -> Option<u32> {
        // Find a free slot.
        let slot_idx = self.slots.iter().position(|s| s.is_free())?;
        let tok = self.next_tok;
        self.next_tok = self.next_tok.wrapping_add(1);
        if self.next_tok == u32::MAX { self.next_tok = 0; } // skip sentinel
        self.slots[slot_idx] = CapSlot { token: tok, kind };
        Some(tok)
    }

    /// Look up a capability token.  Returns the `CapKind` on success.
    pub fn lookup_cap(&self, token: u32) -> Option<CapKind> {
        self.slots.iter()
            .find(|s| !s.is_free() && s.token == token)
            .map(|s| s.kind)
    }

    /// Revoke and free a capability.
    pub fn free_cap(&mut self, token: u32) {
        if let Some(slot) = self.slots.iter_mut()
            .find(|s| !s.is_free() && s.token == token)
        {
            *slot = CapSlot::free();
        }
    }
}

// ── Global per-CPU/per-process table (simplified: one global table for now) ──
// Phase 3 will associate one CapTable per Task.

static GLOBAL_CAP_TABLE: Mutex<CapTable> = Mutex::new(CapTable::new());

pub fn alloc_cap(kind: CapKind) -> Option<u32> {
    GLOBAL_CAP_TABLE.lock().alloc_cap(kind)
}

pub fn lookup_cap(token: u32) -> Option<CapKind> {
    GLOBAL_CAP_TABLE.lock().lookup_cap(token)
}

pub fn free_cap(token: u32) {
    GLOBAL_CAP_TABLE.lock().free_cap(token);
}
