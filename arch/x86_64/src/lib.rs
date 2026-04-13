//! x86-64 architecture support.

#![no_std]
#![feature(abi_x86_interrupt)]

pub mod gdt;
pub mod idt;
pub mod paging;

/// Initialise x86-64 hardware: GDT, IDT, TSS.
pub fn init() {
    gdt::init();
    idt::init();
}
