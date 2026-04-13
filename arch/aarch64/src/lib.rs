//! AArch64 architecture support (ARMv8-A, used in Android/Termux devices).

#![no_std]

pub mod exception;
pub mod paging;

/// Initialise AArch64 hardware: exception vectors, MMU.
pub fn init() {
    exception::init();
}
