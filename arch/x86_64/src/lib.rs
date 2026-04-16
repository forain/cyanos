//! x86-64 architecture support.

#![no_std]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

pub mod gdt;
pub mod idt;
pub mod paging;
#[cfg(target_arch = "x86_64")]
pub mod apic;
#[cfg(target_arch = "x86_64")]
pub mod pic;
#[cfg(target_arch = "x86_64")]
pub mod smp;
#[cfg(target_arch = "x86_64")]
pub mod syscall;
#[cfg(target_arch = "x86_64")]
pub mod timer;

/// Initialise x86-64 hardware: GDT, IDT, APIC, APIC timer, SYSCALL.
///
/// Init order matters:
///   1. GDT  — segments must be valid before IDT exceptions fire.
///   2. IDT  — exception/IRQ handlers must exist before APIC unmasks.
///   3. APIC — masks 8259 PIC, enables LAPIC; must precede timer init.
///   4. Timer — programs APIC timer (calibration uses PIT ch2 briefly).
///   5. SYSCALL — LSTAR/STAR/SFMASK, independent of interrupt routing.
pub fn init() {
    gdt::init();
    idt::init();
    #[cfg(target_arch = "x86_64")]
    unsafe { apic::init(); }
    #[cfg(target_arch = "x86_64")]
    unsafe { timer::init(); }
    #[cfg(target_arch = "x86_64")]
    syscall::init();
}
