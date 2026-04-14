//! x86-64 architecture support.

#![no_std]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

pub mod gdt;
pub mod idt;
pub mod paging;
#[cfg(target_arch = "x86_64")]
pub mod syscall;

/// Initialise x86-64 hardware: GDT, IDT, TSS, SYSCALL.
pub fn init() {
    gdt::init();
    idt::init();
    #[cfg(target_arch = "x86_64")]
    syscall::init();
}
