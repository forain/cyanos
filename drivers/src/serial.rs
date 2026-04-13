//! 16550-compatible UART driver (PC serial / QEMU debug console).

use super::{Driver, DriverError};

const COM1: u16 = 0x3F8;

pub struct Serial {
    base: u16,
}

impl Serial {
    pub const fn new() -> Self { Self { base: COM1 } }

    #[cfg(target_arch = "x86_64")]
    fn outb(&self, offset: u16, val: u8) {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") self.base + offset,
                in("al") val,
                options(nomem, nostack)
            );
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn outb(&self, _offset: u16, _val: u8) {}

    pub fn write_byte(&self, b: u8) {
        self.outb(0, b);
    }

    pub fn write_str(&self, s: &str) {
        for b in s.bytes() {
            self.write_byte(b);
        }
    }
}

impl Driver for Serial {
    fn probe(&mut self) -> Result<(), DriverError> {
        // Disable interrupts, set baud 115200, 8N1.
        self.outb(1, 0x00); // IER off
        self.outb(3, 0x80); // DLAB
        self.outb(0, 0x01); // baud lo (115200)
        self.outb(1, 0x00); // baud hi
        self.outb(3, 0x03); // 8N1
        self.outb(2, 0xC7); // FIFO
        Ok(())
    }

    fn handle(&mut self, msg: ipc::Message) -> ipc::Message {
        // Tag 1 = write bytes from msg.data[0..msg.data[0] as usize].
        if msg.tag == 1 {
            let len = msg.data[0] as usize;
            for &b in &msg.data[1..1 + len.min(55)] {
                self.write_byte(b);
            }
        }
        ipc::Message::empty()
    }
}
