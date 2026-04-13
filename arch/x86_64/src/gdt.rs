//! Global Descriptor Table (GDT) + Task State Segment (TSS) setup.

use core::mem::size_of;

#[repr(C, packed)]
struct GdtEntry(u64);

#[repr(C, align(16))]
struct Gdt {
    null:        GdtEntry,
    kernel_code: GdtEntry,
    kernel_data: GdtEntry,
    user_code:   GdtEntry,
    user_data:   GdtEntry,
}

static GDT: Gdt = Gdt {
    null:        GdtEntry(0),
    kernel_code: GdtEntry(0x00AF_9A00_0000_FFFF), // 64-bit code, DPL 0
    kernel_data: GdtEntry(0x00CF_9200_0000_FFFF), // data, DPL 0
    user_code:   GdtEntry(0x00AF_FA00_0000_FFFF), // 64-bit code, DPL 3
    user_data:   GdtEntry(0x00CF_F200_0000_FFFF), // data, DPL 3
};

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base:  u64,
}

pub fn init() {
    let ptr = GdtPointer {
        limit: (size_of::<Gdt>() - 1) as u16,
        base:  &GDT as *const _ as u64,
    };
    unsafe {
        core::arch::asm!(
            "lgdt [{ptr}]",
            ptr = in(reg) &ptr,
            options(nostack)
        );
    }
}
