//! Flattened Device Tree (DTB) parser — for AArch64 QEMU and real hardware.
//!
//! QEMU -machine virt passes the DTB physical address in x0 on entry.
//! We parse just enough to extract memory regions and the UART base address.
//!
//! Spec: DeviceTree Specification v0.4 §5 (FDT format)

use super::{BootInfo, MemoryRegion, MemoryType};

/// FDT magic number.
pub const FDT_MAGIC: u32 = 0xD00DFEED;

/// FDT header (big-endian on the wire).
#[repr(C)]
struct FdtHeader {
    magic:            u32, // 0xD00DFEED
    totalsize:        u32,
    off_dt_struct:    u32,
    off_dt_strings:   u32,
    off_mem_rsvmap:   u32,
    version:          u32,
    last_comp_version: u32,
    boot_cpuid_phys:  u32,
    size_dt_strings:  u32,
    size_dt_struct:   u32,
}

/// FDT token types.
const FDT_BEGIN_NODE: u32 = 0x00000001;
const FDT_END_NODE:   u32 = 0x00000002;
const FDT_PROP:       u32 = 0x00000003;
const FDT_NOP:        u32 = 0x00000004;
const FDT_END:        u32 = 0x00000009;

fn be32(p: *const u8) -> u32 {
    unsafe {
        u32::from_be_bytes([*p, *p.add(1), *p.add(2), *p.add(3)])
    }
}

fn be64(p: *const u8) -> u64 {
    unsafe {
        u64::from_be_bytes([
            *p, *p.add(1), *p.add(2), *p.add(3),
            *p.add(4), *p.add(5), *p.add(6), *p.add(7),
        ])
    }
}

/// Validate a DTB pointer and return true if it looks like a valid FDT.
///
/// # Safety
/// `dtb_phys` must be a readable physical address.
pub unsafe fn is_valid_dtb(dtb_phys: usize) -> bool {
    if dtb_phys == 0 || dtb_phys & 7 != 0 { return false; }
    be32(dtb_phys as *const u8) == FDT_MAGIC
}

/// Parse the DTB at `dtb_phys` and populate a `BootInfo`.
///
/// # Safety
/// `dtb_phys` must be the physical address of a valid, complete FDT blob.
pub unsafe fn parse(dtb_phys: usize) -> BootInfo {
    let mut info = BootInfo {
        memory_map:          core::ptr::null(),
        memory_map_len:      0,
        framebuffer_base:    0,
        framebuffer_width:   0,
        framebuffer_height:  0,
        framebuffer_pitch:   0,
        rsdp_addr:           0,
    };

    static mut MM: [MemoryRegion; 32] = [MemoryRegion {
        base: 0, length: 0, kind: MemoryType::Reserved
    }; 32];
    let mut mm_idx = 0usize;

    let hdr = dtb_phys as *const FdtHeader;
    if be32(dtb_phys as *const u8) != FDT_MAGIC { return info; }

    let struct_off  = be32(core::ptr::addr_of!((*hdr).off_dt_struct) as *const u8) as usize;
    let strings_off = be32(core::ptr::addr_of!((*hdr).off_dt_strings) as *const u8) as usize;

    let struct_base  = dtb_phys + struct_off;
    let strings_base = dtb_phys + strings_off;

    let mut pos = struct_base;
    let mut depth = 0i32;
    let mut in_memory = false;
    let mut address_cells = 2u8;  // default: 2 × u32 = u64
    let mut size_cells = 2u8;     // default: 2 × u32 = u64

    loop {
        let token = be32(pos as *const u8);
        pos += 4;

        match token {
            FDT_BEGIN_NODE => {
                // Node name is a null-terminated string.
                let name_ptr = pos as *const u8;
                let mut name_len = 0;
                while *name_ptr.add(name_len) != 0 { name_len += 1; }
                let name = core::str::from_utf8(
                    core::slice::from_raw_parts(name_ptr, name_len)
                ).unwrap_or("");

                // Detect "memory" nodes (may be "memory@<addr>").
                in_memory = name == "memory" || name.starts_with("memory@");

                // Advance past the name (aligned to 4 bytes).
                pos += (name_len + 1 + 3) & !3;
                depth += 1;
            }

            FDT_END_NODE => {
                depth -= 1;
                if depth == 1 { in_memory = false; }
            }

            FDT_PROP => {
                let data_len  = be32(pos as *const u8) as usize;
                let name_off  = be32((pos + 4) as *const u8) as usize;
                let data_ptr  = (pos + 8) as *const u8;
                pos += 8 + (data_len + 3) & !3;

                // Property name string from strings block.
                let prop_name_ptr = (strings_base + name_off) as *const u8;
                let mut pn_len = 0;
                while *prop_name_ptr.add(pn_len) != 0 { pn_len += 1; }
                let prop_name = core::str::from_utf8(
                    core::slice::from_raw_parts(prop_name_ptr, pn_len)
                ).unwrap_or("");

                match prop_name {
                    "#address-cells" if data_len >= 4 => {
                        address_cells = be32(data_ptr) as u8;
                    }
                    "#size-cells" if data_len >= 4 => {
                        size_cells = be32(data_ptr) as u8;
                    }
                    "reg" if in_memory && mm_idx < 32 => {
                        // "reg" = list of (address, size) pairs.
                        // Each address is address_cells × u32, size is size_cells × u32.
                        let entry_bytes = (address_cells + size_cells) as usize * 4;
                        let mut off = 0;
                        while off + entry_bytes <= data_len {
                            let base = if address_cells == 2 {
                                be64(data_ptr.add(off))
                            } else {
                                be32(data_ptr.add(off)) as u64
                            };
                            let size_off = off + address_cells as usize * 4;
                            let size = if size_cells == 2 {
                                be64(data_ptr.add(size_off))
                            } else {
                                be32(data_ptr.add(size_off)) as u64
                            };
                            MM[mm_idx] = MemoryRegion {
                                base, length: size, kind: MemoryType::Available
                            };
                            mm_idx += 1;
                            off += entry_bytes;
                        }
                    }
                    _ => {}
                }
            }

            FDT_NOP  => {}
            FDT_END  => break,
            _        => break, // malformed
        }
    }

    info.memory_map     = core::ptr::addr_of!(MM) as *const MemoryRegion;
    info.memory_map_len = mm_idx;
    info
}
