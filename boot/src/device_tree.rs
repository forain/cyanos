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
        uart_base:           0,
        initrd_base:         0,
        initrd_size:         0,
        hhdm_offset:         0,
    };

    // 64 slots: enough for all available regions + firmware memreserve entries.
    static mut MM: [MemoryRegion; 64] = [MemoryRegion {
        base: 0, length: 0, kind: MemoryType::Reserved
    }; 64];
    let mut mm_idx = 0usize;

    let hdr = dtb_phys as *const FdtHeader;
    if be32(dtb_phys as *const u8) != FDT_MAGIC { return info; }

    // totalsize is at offset 4 in the FDT header (big-endian u32).
    let total_size  = be32((dtb_phys + 4) as *const u8) as usize;
    // dtb_end marks the exclusive upper bound for all reads from this DTB.
    // Any read at or beyond this address is out of range and must be rejected.
    let dtb_end = dtb_phys + total_size;

    let struct_off   = be32(core::ptr::addr_of!((*hdr).off_dt_struct)   as *const u8) as usize;
    let strings_off  = be32(core::ptr::addr_of!((*hdr).off_dt_strings)  as *const u8) as usize;
    let memrsv_off   = be32(core::ptr::addr_of!((*hdr).off_mem_rsvmap)  as *const u8) as usize;

    // Validate that all block offsets lie within the DTB.
    if struct_off  >= total_size { return info; }
    if strings_off >= total_size { return info; }

    let struct_base  = dtb_phys + struct_off;
    let strings_base = dtb_phys + strings_off;
    // Upper bound of the strings block (conservative: extends to dtb_end).
    let strings_end  = dtb_end;

    let mut pos = struct_base;
    let mut depth = 0i32;
    let mut in_memory     = false;
    let mut in_framebuf   = false;
    let mut in_pl011      = false;
    let mut in_chosen     = false;
    let mut address_cells = 2u8;  // default: 2 × u32 = u64
    let mut size_cells    = 2u8;  // default: 2 × u32 = u64
    // Scratch: accumulate framebuffer fields from the /framebuffer node.
    let mut fb_base:   u64 = 0;
    let mut fb_width:  u32 = 0;
    let mut fb_height: u32 = 0;
    let mut fb_pitch:  u32 = 0;

    loop {
        // Bounds check: need at least 4 bytes for the token.
        if pos + 4 > dtb_end { break; }
        let token = be32(pos as *const u8);
        pos += 4;

        match token {
            FDT_BEGIN_NODE => {
                // Node name is a null-terminated string.
                let name_ptr = pos as *const u8;
                let mut name_len = 0;
                // Scan only within the DTB bounds to avoid unbounded reads.
                while pos + name_len < dtb_end && *name_ptr.add(name_len) != 0 {
                    name_len += 1;
                }
                let name = core::str::from_utf8(
                    core::slice::from_raw_parts(name_ptr, name_len)
                ).unwrap_or("");

                // Detect interesting node types by their base name.
                in_memory   = name == "memory"      || name.starts_with("memory@");
                in_framebuf = name == "framebuffer"  || name.starts_with("framebuffer@")
                           || name.starts_with("simple-framebuffer@");
                in_pl011    = name.starts_with("pl011@") || name.starts_with("uart@");
                in_chosen   = name == "chosen";

                // Debug: Print when we enter the chosen node
                if in_chosen {
                    extern "C" {
                        fn serial_print_bytes(ptr: *const u8, len: usize);
                    }
                    let msg = "[DTB] Entering /chosen node\n";
                    serial_print_bytes(msg.as_ptr(), msg.len());
                }

                // Advance past the name (aligned to 4 bytes).
                pos += (name_len + 1 + 3) & !3;
                depth += 1;
            }

            FDT_END_NODE => {
                depth -= 1;
                if depth == 1 {
                    in_memory   = false;
                    in_framebuf = false;
                    in_pl011    = false;
                    in_chosen   = false;
                }
            }

            FDT_PROP => {
                // Need 8 bytes for the prop header (data_len + name_off).
                if pos + 8 > dtb_end { break; }
                let data_len  = be32(pos as *const u8) as usize;
                let name_off  = be32((pos + 4) as *const u8) as usize;
                // Validate that the property data fits within the DTB.
                // Use checked_add to guard against overflow from a malformed data_len.
                let data_start = pos + 8;
                let data_end   = match data_start.checked_add(data_len) {
                    Some(e) => e,
                    None    => break,
                };
                if data_end > dtb_end { break; }
                let data_ptr  = data_start as *const u8;
                pos += 8 + ((data_len + 3) & !3);

                // Property name string from strings block: validate name_off.
                let name_ptr_addr = strings_base + name_off;
                if name_ptr_addr >= strings_end { continue; }
                let prop_name_ptr = name_ptr_addr as *const u8;
                let mut pn_len = 0;
                // Scan only within the strings block bounds.
                while name_ptr_addr + pn_len < strings_end
                    && *prop_name_ptr.add(pn_len) != 0
                {
                    pn_len += 1;
                }
                let prop_name = core::str::from_utf8(
                    core::slice::from_raw_parts(prop_name_ptr, pn_len)
                ).unwrap_or("");

                // Debug: Print all properties in /chosen node
                if in_chosen {
                    extern "C" {
                        fn serial_print_bytes(ptr: *const u8, len: usize);
                    }
                    let prefix = "[DTB] /chosen property: ";
                    serial_print_bytes(prefix.as_ptr(), prefix.len());
                    serial_print_bytes(prop_name_ptr, pn_len);
                    let newline = "\n";
                    serial_print_bytes(newline.as_ptr(), newline.len());
                }

                match prop_name {
                    // #address-cells / #size-cells are inheritable: every node
                    // can define them for *its children*.  We only care about the
                    // root node's values (depth == 1) because that controls how
                    // /memory@, /pl011@, and /framebuffer@ children encode their
                    // "reg" properties.  Sub-nodes like /cpus (which sets
                    // #address-cells = <1> for cpu@ children) must not overwrite
                    // the root values we rely on for memory-map parsing.
                    "#address-cells" if data_len >= 4 && depth == 1 => {
                        address_cells = be32(data_ptr) as u8;
                    }
                    "#size-cells" if data_len >= 4 && depth == 1 => {
                        size_cells = be32(data_ptr) as u8;
                    }

                    // ── Framebuffer node ──────────────────────────────────────
                    // QEMU virt exposes a simple-framebuffer node with:
                    //   reg       = <base size>
                    //   width     = <u32>
                    //   height    = <u32>
                    //   stride    = <u32>   (bytes per row)
                    "reg" if in_framebuf && data_len >= 8 => {
                        fb_base = if address_cells >= 2 {
                            be64(data_ptr)
                        } else {
                            be32(data_ptr) as u64
                        };
                    }
                    "width"  if in_framebuf && data_len >= 4 => { fb_width  = be32(data_ptr); }
                    "height" if in_framebuf && data_len >= 4 => { fb_height = be32(data_ptr); }
                    "stride" if in_framebuf && data_len >= 4 => { fb_pitch  = be32(data_ptr); }

                    // ── PL011 UART node ───────────────────────────────────────
                    // "reg" gives the MMIO base address.  Only the first PL011
                    // found is recorded; kernel_main uses it to set the serial
                    // console base when the DTB path is active.
                    "reg" if in_pl011 && data_len >= 4 => {
                        if info.uart_base == 0 {
                            info.uart_base = if address_cells >= 2 {
                                be64(data_ptr)
                            } else {
                                be32(data_ptr) as u64
                            };
                        }
                    }

                    "reg" if in_memory && mm_idx < 32 => {
                        // "reg" = list of (address, size) pairs.
                        // Each address is address_cells × u32, size is size_cells × u32.
                        let entry_bytes = (address_cells + size_cells) as usize * 4;
                        // Guard: a zero entry_bytes would loop forever since `off`
                        // never advances.  A well-formed DTB always has both
                        // address_cells ≥ 1 and size_cells ≥ 1 for memory nodes.
                        if entry_bytes == 0 { break; }
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

                    // ── /chosen node: initrd ──────────────────────────────────────
                    // QEMU populates the /chosen node with initrd information when
                    // the -initrd parameter is used:
                    //   linux,initrd-start = <u64 physical address>
                    //   linux,initrd-end   = <u64 physical address>
                    "linux,initrd-start" if in_chosen && data_len >= 8 => {
                        info.initrd_base = be64(data_ptr);
                        // Debug: Use serial_print to see the initrd base address
                        extern "C" {
                            fn serial_print_bytes(ptr: *const u8, len: usize);
                        }
                        let msg = "[DTB] Found linux,initrd-start\n";
                        serial_print_bytes(msg.as_ptr(), msg.len());
                    }
                    "linux,initrd-end" if in_chosen && data_len >= 8 => {
                        let initrd_end = be64(data_ptr);
                        if initrd_end > info.initrd_base {
                            info.initrd_size = initrd_end - info.initrd_base;
                        }
                        // Debug: Use serial_print to see the initrd end address
                        extern "C" {
                            fn serial_print_bytes(ptr: *const u8, len: usize);
                        }
                        let msg = "[DTB] Found linux,initrd-end\n";
                        serial_print_bytes(msg.as_ptr(), msg.len());
                    }

                    _ => {}
                }
            }

            FDT_NOP  => {}
            FDT_END  => break,
            _        => break, // malformed
        }
    }

    // ── Memory reservation map ───────────────────────────────────────────────
    //
    // The FDT memory reservation map (off_mem_rsvmap) lists physical address
    // ranges that must NOT be used by the OS — typically firmware-private
    // regions such as ARM Trusted Firmware (ATF/BL31) at 0x0, VideoCore
    // shared buffers on RPi5, PSCI table, etc.
    //
    // Each entry is a pair of big-endian u64 (address, size).
    // The list is terminated by an all-zero entry.
    //
    // We add these to the MM array as MemoryType::Reserved so the buddy
    // allocator (which skips non-Available entries) leaves them alone.
    if memrsv_off < total_size {
        let mut rsvpos = dtb_phys + memrsv_off;
        loop {
            // Need 16 bytes for one entry.
            if rsvpos + 16 > dtb_end { break; }
            let rsv_base = be64(rsvpos as *const u8);
            let rsv_size = be64((rsvpos + 8) as *const u8);
            // All-zero entry terminates the list (FDT spec §5.3.2).
            if rsv_base == 0 && rsv_size == 0 { break; }
            if mm_idx < 64 {
                MM[mm_idx] = MemoryRegion {
                    base:   rsv_base,
                    length: rsv_size,
                    kind:   MemoryType::Reserved,
                };
                mm_idx += 1;
            }
            rsvpos += 16;
        }
    }

    info.memory_map      = core::ptr::addr_of!(MM) as *const MemoryRegion;
    info.memory_map_len  = mm_idx;
    // Populate framebuffer fields if a framebuffer node was found.
    if fb_base != 0 {
        info.framebuffer_base   = fb_base;
        info.framebuffer_width  = fb_width;
        info.framebuffer_height = fb_height;
        info.framebuffer_pitch  = fb_pitch;
    }
    info
}

/// Create a minimal default BootInfo for QEMU virt machine when no DTB is provided.
/// This is used when the kernel is loaded directly with `qemu -kernel`.
pub fn create_qemu_virt_default() -> BootInfo {
    // Use a static array that's always initialized
    static DEFAULT_MM: [MemoryRegion; 1] = [
        MemoryRegion { base: 0x40000000, length: 0x10000000, kind: MemoryType::Available }, // 256MB at 1GB
    ];

    let ptr = DEFAULT_MM.as_ptr();

    BootInfo {
        memory_map: ptr,
        memory_map_len: 1,
        framebuffer_base: 0,
        framebuffer_width: 0,
        framebuffer_height: 0,
        framebuffer_pitch: 0,
        rsdp_addr: 0,
        uart_base: 0x09000000, // QEMU virt PL011 UART
        initrd_base: 0,
        initrd_size: 0,
        hhdm_offset: 0,
    }
}
