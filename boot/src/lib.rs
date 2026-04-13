//! Boot protocol definitions — structures shared between the bootloader
//! and the kernel (multiboot2 / UEFI hand-off info).

#![no_std]

/// Physical memory region types (multiboot2 §3.6.8 / UEFI MemoryType).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryType {
    Available = 1,
    Reserved  = 2,
    AcpiReclaimable = 3,
    AcpiNvs  = 4,
    BadMemory = 5,
}

/// A single entry in the memory map passed by the bootloader.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryType,
}

/// Top-level structure passed from bootloader to `kernel_main`.
#[repr(C)]
pub struct BootInfo {
    pub memory_map: *const MemoryRegion,
    pub memory_map_len: usize,
    pub framebuffer_base: u64,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_pitch: u32,
    pub rsdp_addr: u64,  // ACPI root pointer.
}
