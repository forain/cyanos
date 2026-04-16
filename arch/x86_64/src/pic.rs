//! 8259A Programmable Interrupt Controller (PIC) driver.
//!
//! Remaps the two PICs so that:
//!   Master (IRQ 0-7)  → INT 32-39
//!   Slave  (IRQ 8-15) → INT 40-47
//!
//! After init only IRQ0 (timer) is unmasked; all others are masked.
//!
//! Reference: Intel 8259A datasheet.

// ── I/O port addresses ────────────────────────────────────────────────────────

const PIC1_CMD:  u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD:  u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

// Initialization Command Words
const ICW1_INIT:   u8 = 0x11; // ICW1: init + expect ICW4
const ICW4_8086:   u8 = 0x01; // ICW4: 8086 mode, not buffered

// ── I/O helpers ───────────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack)
    );
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn outb(_port: u16, _val: u8) {}

#[cfg(target_arch = "x86_64")]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack)
    );
    val
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn inb(_port: u16) -> u8 { 0 }

/// Short I/O delay — write to port 0x80 (POST code port, safe to write).
#[cfg(target_arch = "x86_64")]
unsafe fn io_wait() {
    core::arch::asm!("out 0x80, al", in("al") 0u8, options(nomem, nostack));
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn io_wait() {}

// ── Public API ────────────────────────────────────────────────────────────────

/// Remap both PICs and unmask only IRQ0 (timer).
pub unsafe fn init() {
    // Save current IRQ masks.
    let mask1 = inb(PIC1_DATA);
    let mask2 = inb(PIC2_DATA);

    // ICW1 — start initialization sequence (cascade mode).
    outb(PIC1_CMD,  ICW1_INIT); io_wait();
    outb(PIC2_CMD,  ICW1_INIT); io_wait();

    // ICW2 — vector offsets.
    outb(PIC1_DATA, 32); io_wait(); // master: IRQ 0-7  → INT 32-39
    outb(PIC2_DATA, 40); io_wait(); // slave:  IRQ 8-15 → INT 40-47

    // ICW3 — cascade wiring.
    outb(PIC1_DATA, 4);  io_wait(); // master: slave on IRQ2 (bit 2 set)
    outb(PIC2_DATA, 2);  io_wait(); // slave: its cascade identity = 2

    // ICW4 — 8086 mode.
    outb(PIC1_DATA, ICW4_8086); io_wait();
    outb(PIC2_DATA, ICW4_8086); io_wait();

    // OCW1 — set IRQ masks.  Allow only IRQ0 (timer) on master; mask all slave.
    // Saved masks are discarded — we start fresh.
    let _ = (mask1, mask2);
    outb(PIC1_DATA, 0xFE); // 1111_1110: all masked except bit 0 (IRQ0)
    outb(PIC2_DATA, 0xFF); // 1111_1111: all slave IRQs masked
}

/// Send End-Of-Interrupt for IRQ number `irq` (0-based, 0-15).
///
/// For slave IRQs (8-15) an EOI is sent to both PICs.
pub unsafe fn eoi(irq: u8) {
    const OCW2_EOI: u8 = 0x20; // non-specific EOI command
    if irq >= 8 {
        outb(PIC2_CMD, OCW2_EOI);
    }
    outb(PIC1_CMD, OCW2_EOI);
}

/// Unmask an IRQ line (0-based).
pub unsafe fn unmask(irq: u8) {
    if irq < 8 {
        let m = inb(PIC1_DATA) & !(1 << irq);
        outb(PIC1_DATA, m);
    } else {
        let m = inb(PIC2_DATA) & !(1 << (irq - 8));
        outb(PIC2_DATA, m);
        // Also ensure IRQ2 (cascade line) is unmasked on master.
        let m1 = inb(PIC1_DATA) & !(1 << 2);
        outb(PIC1_DATA, m1);
    }
}

/// Mask an IRQ line (0-based).
pub unsafe fn mask(irq: u8) {
    if irq < 8 {
        let m = inb(PIC1_DATA) | (1 << irq);
        outb(PIC1_DATA, m);
    } else {
        let m = inb(PIC2_DATA) | (1 << (irq - 8));
        outb(PIC2_DATA, m);
    }
}
