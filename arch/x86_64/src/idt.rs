//! Interrupt Descriptor Table (IDT) — exception and IRQ handlers.

use core::mem::size_of;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IdtEntry {
    offset_low:  u16,
    selector:    u16,
    ist:         u8,
    type_attr:   u8,
    offset_mid:  u16,
    offset_high: u32,
    _reserved:   u32,
}

impl IdtEntry {
    fn new(handler: usize, selector: u16, ist: u8, type_attr: u8) -> Self {
        Self {
            offset_low:  handler as u16,
            selector,
            ist,
            type_attr,
            offset_mid:  (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved:   0,
        }
    }
}

#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([IdtEntry {
    offset_low: 0, selector: 0, ist: 0, type_attr: 0,
    offset_mid: 0, offset_high: 0, _reserved: 0,
}; 256]);

#[repr(C, packed)]
struct IdtPointer { limit: u16, base: u64 }

pub fn init() {
    unsafe {
        // Install a generic fault handler for all exceptions.
        for i in 0..32usize {
            IDT.0[i] = IdtEntry::new(generic_fault as *const () as usize, 0x08, 0, 0x8E);
        }
        let ptr = IdtPointer {
            limit: (size_of::<Idt>() - 1) as u16,
            base:  core::ptr::addr_of!(IDT) as u64,
        };
        core::arch::asm!("lidt [{ptr}]", ptr = in(reg) &ptr, options(nostack));
    }
}

#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn generic_fault(_frame: InterruptStackFrame) {
    panic!("unhandled CPU exception");
}

#[cfg(not(target_arch = "x86_64"))]
extern "C" fn generic_fault(_frame: InterruptStackFrame) {
    panic!("unhandled CPU exception");
}

/// Minimal interrupt stack frame (x86-64).
#[repr(C)]
pub struct InterruptStackFrame {
    pub ip: u64,
    pub cs: u64,
    pub flags: u64,
    pub sp: u64,
    pub ss: u64,
}
