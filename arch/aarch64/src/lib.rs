//! AArch64 architecture support (ARMv8-A).

#![no_std]

pub mod exception;
pub mod gic;
pub mod paging;
pub mod timer;

/// Initialise AArch64 hardware.
///
/// Call order matters:
///   1. exception vectors (VBAR_EL1) — first, so any fault during init is caught
///   2. GIC distributor + CPU interface — before the timer sets up the IRQ
///   3. generic timer — arms the countdown and unmasks IRQs
pub fn init() {
    exception::init();
    gic::init();
    timer::init();
}
