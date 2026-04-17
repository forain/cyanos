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

/// Interrupt stack frame pushed by the CPU on exception entry (x86-64).
#[repr(C)]
pub struct InterruptStackFrame {
    pub ip:    u64,
    pub cs:    u64,
    pub flags: u64,
    pub sp:    u64,
    pub ss:    u64,
}

pub fn init() {
    unsafe {
        // Exceptions without an error code: 0-7, 9, 15-16, 18-20, 28.
        for i in 0..32usize {
            IDT.0[i] = IdtEntry::new(fault_no_err as *const () as usize, 0x08, 0, 0x8E);
        }

        // Exceptions that push an error code: 8, 10-14, 17, 21, 29-30.
        for &v in &[8u8, 10, 11, 12, 13, 17, 21, 29, 30] {
            IDT.0[v as usize] =
                IdtEntry::new(fault_with_err as *const () as usize, 0x08, 0, 0x8E);
        }

        // Vector 8 = double fault — must use IST1 so it runs on a dedicated
        // stack.  Without IST the handler would execute on the same exhausted
        // or corrupted stack that caused the double fault, triple-faulting.
        // IST1 corresponds to TSS.ist[0], initialised in gdt::init().
        IDT.0[8] = IdtEntry::new(fault_with_err as *const () as usize, 0x08, 1, 0x8E);

        // Vector 14 = page fault — needs CR2 in addition to error code.
        IDT.0[14] = IdtEntry::new(page_fault as *const () as usize, 0x08, 0, 0x8E);

        // Vector 32 = IRQ0 (8253/8254 timer after PIC remapping).
        IDT.0[32] = IdtEntry::new(timer_irq as *const () as usize, 0x08, 0, 0x8E);

        #[cfg(target_arch = "x86_64")]
        let ptr = IdtPointer {
            limit: (size_of::<Idt>() - 1) as u16,
            base:  core::ptr::addr_of!(IDT) as u64,
        };
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("lidt [{}]", in(reg) &ptr, options(nostack));
    }
}

// ── Minimal serial output for exception dumps ─────────────────────────────────
// Direct port I/O to COM1 (0x3F8) avoids any dependency on the drivers crate.

#[cfg(target_arch = "x86_64")]
fn serial_byte(b: u8) {
    unsafe {
        // Spin on LSR.THRE (bit 5) — transmit-holding-register empty.
        loop {
            let lsr: u8;
            core::arch::asm!(
                "in al, dx", out("al") lsr, in("dx") 0x3F8u16 + 5,
                options(nomem, nostack)
            );
            if lsr & 0x20 != 0 { break; }
        }
        core::arch::asm!(
            "out dx, al", in("dx") 0x3F8u16, in("al") b,
            options(nomem, nostack)
        );
    }
}

#[cfg(target_arch = "x86_64")]
fn serial_str(s: &[u8]) {
    for &b in s { serial_byte(b); }
}

/// Print a u64 as 16 hex digits.
#[cfg(target_arch = "x86_64")]
fn serial_hex64(v: u64) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut buf = [0u8; 16];
    for i in 0..16 {
        buf[15 - i] = HEX[((v >> (i * 4)) & 0xF) as usize];
    }
    serial_str(&buf);
}

// ── Exception entry point shared by all handlers ──────────────────────────────

#[cfg(target_arch = "x86_64")]
fn print_exception(frame: &InterruptStackFrame, vector: u64, error_code: u64) {
    serial_str(b"\r\n*** KERNEL EXCEPTION ***\r\n");
    serial_str(b"Vector=0x");   serial_hex64(vector);     serial_str(b"\r\n");
    serial_str(b"ErrCode=0x");  serial_hex64(error_code); serial_str(b"\r\n");
    serial_str(b"RIP=0x");      serial_hex64(frame.ip);   serial_str(b"\r\n");
    serial_str(b"CS=0x");       serial_hex64(frame.cs);   serial_str(b"\r\n");
    serial_str(b"RFLAGS=0x");   serial_hex64(frame.flags);serial_str(b"\r\n");
    serial_str(b"RSP=0x");      serial_hex64(frame.sp);   serial_str(b"\r\n");
    serial_str(b"SS=0x");       serial_hex64(frame.ss);   serial_str(b"\r\n");
}

// ── Exception handlers ────────────────────────────────────────────────────────

/// Returns true if the exception was taken from ring 3 (user mode).
#[cfg(target_arch = "x86_64")]
#[inline]
fn from_user(frame: &InterruptStackFrame) -> bool {
    frame.cs & 0x3 == 3
}

/// Handler for exceptions that do NOT push an error code.
///
/// If from user space, kill the task.  If from the kernel, halt — it's a bug.
#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn fault_no_err(frame: InterruptStackFrame) {
    if from_user(&frame) {
        serial_str(b"user fault (no errcode): task killed\r\n");
        sched::exit(1);
    } else {
        print_exception(&frame, 0xFF, 0);
        loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
    }
}

/// Handler for exceptions that push an error code.
///
/// If from user space, kill the task.  If from the kernel, halt — it's a bug.
#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn fault_with_err(frame: InterruptStackFrame, error_code: u64) {
    if from_user(&frame) {
        serial_str(b"user fault (errcode): task killed\r\n");
        let _ = error_code;
        sched::exit(1);
    } else {
        print_exception(&frame, 0xFF, error_code);
        loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
    }
}

/// Page fault handler — also reads CR2 (faulting virtual address).
///
/// Error code bit 0 (P): 0 = not-present, 1 = protection violation.
///
/// For user-mode not-present faults we first try the demand-paging path.
/// If that succeeds the handler returns normally and execution resumes.
/// All other user faults kill the task; kernel faults halt.
#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn page_fault(frame: InterruptStackFrame, error_code: u64) {
    let cr2: u64;
    unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack)); }

    if from_user(&frame) {
        // Bit 0 of the error code: 0 = page not present (translation fault).
        // Try demand paging before giving up.
        if error_code & 1 == 0 && sched::handle_page_fault(cr2 as usize) {
            return; // fault handled — resume user task
        }
        serial_str(b"user page fault CR2=0x"); serial_hex64(cr2);
        serial_str(b" err=0x"); serial_hex64(error_code);
        serial_str(b": task killed\r\n");
        sched::exit(1);
    } else {
        print_exception(&frame, 14, error_code);
        serial_str(b"CR2=0x"); serial_hex64(cr2); serial_str(b"\r\n");
        loop { unsafe { core::arch::asm!("hlt", options(nomem, nostack)); } }
    }
}

// Non-x86 stubs (satisfy the compiler on other targets).
#[cfg(not(target_arch = "x86_64"))]
extern "C" fn fault_no_err(_frame: InterruptStackFrame) {
    loop {}
}
#[cfg(not(target_arch = "x86_64"))]
extern "C" fn fault_with_err(_frame: InterruptStackFrame, _error_code: u64) {
    loop {}
}
#[cfg(not(target_arch = "x86_64"))]
extern "C" fn page_fault(_frame: InterruptStackFrame, _error_code: u64) {
    loop {}
}

/// Timer IRQ handler — APIC timer at 100 Hz.
///
/// Sends LAPIC EOI, drives the scheduler tick, then checks if the running
/// task should be preempted.  `sched::preempt_check()` calls `yield_now()`
/// if needed; the `iretq` epilogue then resumes the correct task.
#[cfg(target_arch = "x86_64")]
extern "x86-interrupt" fn timer_irq(_frame: InterruptStackFrame) {
    super::apic::eoi();
    super::timer::on_tick();
    sched::preempt_check();
}

#[cfg(not(target_arch = "x86_64"))]
extern "C" fn timer_irq(_frame: InterruptStackFrame) {
    // No-op: timer module is only present on x86_64.
}
