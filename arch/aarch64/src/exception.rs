//! AArch64 exception vector table and handler stubs.

/// Install the exception vector table (VBAR_EL1).
pub fn init() {
    unsafe {
        core::arch::asm!(
            "adr x0, __exception_vectors",
            "msr VBAR_EL1, x0",
            "isb",
            options(nostack)
        );
    }
}

// The actual vector table must be defined in assembly (16-byte aligned slots).
core::arch::global_asm!(r#"
.section .text
.balign 2048
__exception_vectors:
    // Synchronous EL1t
    b exception_handler
    .balign 128
    // IRQ EL1t
    b exception_handler
    .balign 128
    // FIQ EL1t
    b exception_handler
    .balign 128
    // SError EL1t
    b exception_handler
    .balign 128
    // (EL1h, EL0 64-bit, EL0 32-bit slots follow — all redirect for now)
    .rept 12
    b exception_handler
    .balign 128
    .endr
"#);

#[no_mangle]
unsafe extern "C" fn exception_handler() {
    panic!("unhandled AArch64 exception");
}
