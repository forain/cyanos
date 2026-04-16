//! Serial console driver.
//!
//! x86-64:  16550A UART at COM1 (0x3F8), programmed via `out` instructions.
//! AArch64: PL011 UART at 0x0900_0000 (QEMU virt), via arch_serial_putc/init
//!          symbols exported by arch-aarch64::uart (resolved at link time).

use super::{Driver, DriverError};

const COM1: u16 = 0x3F8;

// ── Architecture-specific serial output ──────────────────────────────────────

#[cfg(target_arch = "x86_64")]
extern "C" {}  // nothing extra needed; all ops are inline asm below

#[cfg(not(target_arch = "x86_64"))]
extern "C" {
    /// Initialise the PL011 UART — provided by arch-aarch64::uart.
    fn arch_serial_init();
    /// Write one byte to the PL011 TX FIFO — provided by arch-aarch64::uart.
    fn arch_serial_putc(c: u8);
}

// ── Driver struct ─────────────────────────────────────────────────────────────

pub struct Serial {
    pub base: u16,
}

impl Serial {
    pub const fn new() -> Self { Self { base: COM1 } }

    pub fn write_byte(&self, b: u8) {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") self.base,
                    in("al") b,
                    options(nomem, nostack)
                );
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        unsafe { arch_serial_putc(b); }
    }

    pub fn write_str(&self, s: &str) {
        for b in s.bytes() { self.write_byte(b); }
    }
}

impl Driver for Serial {
    fn probe(&mut self) -> Result<(), DriverError> {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            // 16550A init: disable interrupts, set 115200 8N1, enable FIFO.
            let outb = |off: u16, val: u8| {
                core::arch::asm!(
                    "out dx, al",
                    in("dx") self.base + off,
                    in("al") val,
                    options(nomem, nostack)
                );
            };
            outb(1, 0x00); // IER: disable interrupts
            outb(3, 0x80); // LCR: DLAB on
            outb(0, 0x01); // DLL: baud lo (divisor 1 → 115200 with 1.8432 MHz clock)
            outb(1, 0x00); // DLM: baud hi
            outb(3, 0x03); // LCR: 8N1, DLAB off
            outb(2, 0xC7); // FCR: enable + clear FIFOs, 14-byte trigger
        }
        #[cfg(not(target_arch = "x86_64"))]
        unsafe { arch_serial_init(); }
        Ok(())
    }

    fn handle(&mut self, msg: ipc::Message) -> ipc::Message {
        if msg.tag == 1 {
            let len = msg.data[0] as usize;
            for &b in &msg.data[1..1 + len.min(55)] {
                self.write_byte(b);
            }
        }
        ipc::Message::empty()
    }
}
