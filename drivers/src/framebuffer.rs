//! Linear framebuffer driver (UEFI GOP / VESA).

use super::{Driver, DriverError};

pub struct Framebuffer {
    base: *mut u32,
    width: usize,
    height: usize,
    pitch: usize, // bytes per row
}

// Safety: kernel owns the framebuffer exclusively.
unsafe impl Send for Framebuffer {}
unsafe impl Sync for Framebuffer {}

impl Framebuffer {
    /// Construct from bootloader-provided parameters.
    pub unsafe fn new(base: *mut u32, width: usize, height: usize, pitch: usize) -> Self {
        Self { base, width, height, pitch }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            unsafe {
                let offset = y * (self.pitch / 4) + x;
                self.base.add(offset).write_volatile(color);
            }
        }
    }

    pub fn clear(&mut self, color: u32) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.set_pixel(x, y, color);
            }
        }
    }
}

impl Driver for Framebuffer {
    fn probe(&mut self) -> Result<(), DriverError> {
        // Already set up by bootloader; nothing to do.
        Ok(())
    }

    fn handle(&mut self, msg: ipc::Message) -> ipc::Message {
        // Tag 1 = clear with colour in data[0..4].
        if msg.tag == 1 {
            let color = u32::from_le_bytes(msg.data[0..4].try_into().unwrap_or([0; 4]));
            self.clear(color);
        }
        ipc::Message::empty()
    }
}
