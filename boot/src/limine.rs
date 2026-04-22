//! Limine boot protocol — request/response structures and parser.
//!
//! Ref: Limine Boot Protocol Specification
//!      https://github.com/limine-bootloader/limine/blob/stable/PROTOCOL.md

use super::{BootInfo, MemoryRegion, MemoryType};

// ── Magic numbers ─────────────────────────────────────────────────────────────

/// First two words common to every Limine request ID.
const COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

// ── Request structure ─────────────────────────────────────────────────────────

/// Generic Limine request header.
#[repr(C)]
struct Request {
    id:       [u64; 4],
    revision: u64,
    response: *mut u8,
}

/// Specialized request for the entry point.
#[repr(C)]
struct EntryPointRequest {
    id:          [u64; 4],
    revision:    u64,
    response:    *mut u8,
    entry_point: unsafe extern "C" fn() -> !,
}

// SAFETY: response is written once by the bootloader.
unsafe impl Sync for Request {}
unsafe impl Sync for EntryPointRequest {}

impl Request {
    /// Return the response pointer if Limine satisfied this request.
    unsafe fn response<T>(&self) -> Option<&T> {
        if self.response.is_null() {
            None
        } else {
            Some(&*(self.response as *const T))
        }
    }
}

// ── Requests ──────────────────────────────────────────────────────────────────

extern "C" {
    fn _start() -> !;
}

/// Base revision — tells Limine the maximum protocol revision this kernel
/// understands.
#[link_section = ".limine_requests"]
#[used]
static BASE_REVISION: [u64; 3] = [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, 6];

#[link_section = ".limine_requests"]
#[used]
static ENTRY_POINT_REQUEST: EntryPointRequest = EntryPointRequest {
    id:       [COMMON_MAGIC[0], COMMON_MAGIC[1], 0x13d86c035a1cd3e1, 0x2b0caa89d8f3026a],
    revision: 0,
    response: core::ptr::null_mut(),
    entry_point: _start,
};

#[link_section = ".limine_requests"]
#[used]
static MEMMAP_REQUEST: Request = Request {
    id:       [COMMON_MAGIC[0], COMMON_MAGIC[1], 0x67cf3d9d378a806f, 0xe304acdfc50c3c62],
    revision: 0,
    response: core::ptr::null_mut(),
};

#[link_section = ".limine_requests"]
#[used]
static FRAMEBUFFER_REQUEST: Request = Request {
    id:       [COMMON_MAGIC[0], COMMON_MAGIC[1], 0x9d5827dcd881dd75, 0xa3148604f6fab11b],
    revision: 0,
    response: core::ptr::null_mut(),
};

#[link_section = ".limine_requests"]
#[used]
static RSDP_REQUEST: Request = Request {
    id:       [COMMON_MAGIC[0], COMMON_MAGIC[1], 0xc5e77b6b397e7b43, 0x27637845accdcf3c],
    revision: 0,
    response: core::ptr::null_mut(),
};

#[link_section = ".limine_requests"]
#[used]
static MODULE_REQUEST: Request = Request {
    id:       [COMMON_MAGIC[0], COMMON_MAGIC[1], 0xad97e90e83f1ed83, 0xa7a3e59b2c5d9f5a],
    revision: 0,
    response: core::ptr::null_mut(),
};

// ── Response layouts ──────────────────────────────────────────────────────────

#[repr(C)]
struct MemMapResponse {
    revision:    u64,
    entry_count: u64,
    entries:     *const *const MemMapEntry,
}

#[repr(C)]
struct MemMapEntry {
    base:   u64,
    length: u64,
    typ:    u64,
}

const USABLE:                u64 = 0;
const ACPI_RECLAIMABLE:      u64 = 2;
const ACPI_NVS:              u64 = 3;
const BAD_MEMORY:            u64 = 4;

#[repr(C)]
struct FramebufferResponse {
    revision:        u64,
    framebuffer_count: u64,
    framebuffers:    *const *const LimineFramebuffer,
}

#[repr(C)]
struct LimineFramebuffer {
    address:         *mut u8,
    width:           u64,
    height:          u64,
    pitch:           u64,
    bpp:             u16,
    memory_model:    u8,
    red_mask_size:   u8,
    red_mask_shift:  u8,
    green_mask_size: u8,
    green_mask_shift: u8,
    blue_mask_size:  u8,
    blue_mask_shift: u8,
    _unused:         [u8; 7],
    edid_size:       u64,
    edid:            *const u8,
}

#[repr(C)]
struct RsdpResponse {
    revision: u64,
    address:  *const u8,
}

#[repr(C)]
struct ModuleResponse {
    revision:     u64,
    module_count: u64,
    modules:      *const *const Module,
}

#[repr(C)]
struct Module {
    address:  *const u8,
    size:     u64,
    path:     *const u8,
    cmdline:  *const u8,
    media_type: u64,
    unused:   [u64; 4],
}

// ── Static memory-map storage ─────────────────────────────────────────────────

static mut MM: [MemoryRegion; 128] = [MemoryRegion {
    base: 0, length: 0, kind: MemoryType::Reserved,
}; 128];

// ── Public API ────────────────────────────────────────────────────────────────

pub unsafe fn parse() -> BootInfo {
    let mut info = BootInfo {
        memory_map:         core::ptr::null(),
        memory_map_len:     0,
        framebuffer_base:   0,
        framebuffer_width:  0,
        framebuffer_height: 0,
        framebuffer_pitch:  0,
        rsdp_addr:          0,
        uart_base:          0,
        initrd_base:        0,
        initrd_size:        0,
    };

    if let Some(resp) = MEMMAP_REQUEST.response::<MemMapResponse>() {
        let mut idx = 0usize;
        let n = (resp.entry_count as usize).min(512);
        for i in 0..n {
            if idx >= 128 { break; }
            let e = &**resp.entries.add(i);
            let kind = match e.typ {
                USABLE           => MemoryType::Available,
                ACPI_RECLAIMABLE => MemoryType::AcpiReclaimable,
                ACPI_NVS         => MemoryType::AcpiNvs,
                BAD_MEMORY       => MemoryType::BadMemory,
                _                => MemoryType::Reserved,
            };
            MM[idx] = MemoryRegion { base: e.base, length: e.length, kind };
            idx += 1;
        }
        info.memory_map     = core::ptr::addr_of!(MM) as *const MemoryRegion;
        info.memory_map_len = idx;
    }

    if let Some(resp) = FRAMEBUFFER_REQUEST.response::<FramebufferResponse>() {
        if resp.framebuffer_count > 0 {
            let fb = &**resp.framebuffers;
            info.framebuffer_base   = fb.address as u64;
            info.framebuffer_width  = fb.width  as u32;
            info.framebuffer_height = fb.height as u32;
            info.framebuffer_pitch  = fb.pitch  as u32;
        }
    }

    if let Some(resp) = RSDP_REQUEST.response::<RsdpResponse>() {
        info.rsdp_addr = resp.address as u64;
    }

    if let Some(resp) = MODULE_REQUEST.response::<ModuleResponse>() {
        if resp.module_count > 0 {
            let module = &**resp.modules;
            info.initrd_base = module.address as u64;
            info.initrd_size = module.size;
        }
    }

    info
}
