//! Linear framebuffer driver (UEFI GOP / VESA / multiboot2).
//!
//! Boot-time flow:
//!   1. The boot parser (multiboot2 / DTB) calls `set_boot_framebuffer()` with
//!      the parameters it found in the boot information structure.
//!   2. The driver server calls `probe()`.  If boot info was recorded it
//!      initialises `self` from that info; otherwise it returns `NotFound`.

use spin::Mutex;
use super::{Driver, DriverError};

// ── Boot-time registration ────────────────────────────────────────────────────

struct FramebufferInfo {
    base:   u64,
    width:  u32,
    height: u32,
    pitch:  u32,
}

static BOOT_FB: Mutex<Option<FramebufferInfo>> = Mutex::new(None);

/// Record framebuffer parameters discovered from boot information.
///
/// Must be called before the driver server runs `probe()`.  Safe to call
/// multiple times; only the last call takes effect.
pub fn set_boot_framebuffer(base: u64, width: u32, height: u32, pitch: u32) {
    *BOOT_FB.lock() = Some(FramebufferInfo { base, width, height, pitch });
}

// ── Driver struct ─────────────────────────────────────────────────────────────

pub struct Framebuffer {
    base:   *mut u32,
    width:  usize,
    height: usize,
    pitch:  usize, // bytes per row
}

// Safety: kernel owns the framebuffer exclusively.
unsafe impl Send for Framebuffer {}
unsafe impl Sync for Framebuffer {}

impl Framebuffer {
    /// Construct an uninitialised framebuffer driver.
    ///
    /// `probe()` must be called (and succeed) before any drawing methods.
    pub const fn new() -> Self {
        Self {
            base:   core::ptr::null_mut(),
            width:  0,
            height: 0,
            pitch:  0,
        }
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
    /// Initialise from boot-provided parameters.
    ///
    /// Returns `Err(DriverError::NotFound)` if the bootloader did not supply a
    /// linear framebuffer (e.g. text-mode boot, or the DTB has no /framebuffer
    /// node).
    fn probe(&mut self) -> Result<(), DriverError> {
        let info = BOOT_FB.lock().take().ok_or(DriverError::NotFound)?;

        if info.base == 0 || info.width == 0 || info.height == 0 || info.pitch == 0 {
            return Err(DriverError::NotFound);
        }

        self.base   = info.base as *mut u32;
        self.width  = info.width  as usize;
        self.height = info.height as usize;
        self.pitch  = info.pitch  as usize;
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
