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

/// x86_64 serial output for early debugging.
///
/// Uses 16550 UART at COM1 (0x3F8).
#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub unsafe extern "C" fn arch_serial_putc(c: u8) {
    use core::arch::asm;

    // Wait for transmit holding register to be empty (bit 5 of LSR)
    loop {
        let lsr: u8;
        asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nomem, nostack));
        if lsr & 0x20 != 0 { break; }
    }

    // Send the character
    asm!("out dx, al", in("dx") 0x3F8u16, in("al") c, options(nomem, nostack));
}
