//! x86_64 kernel entry point following RedoxOS pattern
//!
//! This replaces the assembly file approach with a naked Rust function
//! that is more maintainable and follows modern Rust kernel patterns.

use core::arch::naked_asm;
use core::cell::SyncUnsafeCell;

/// Test of zero values in BSS.
static BSS_TEST_ZERO: SyncUnsafeCell<usize> = SyncUnsafeCell::new(0);
/// Test of non-zero values in data.
static DATA_TEST_NONZERO: SyncUnsafeCell<usize> = SyncUnsafeCell::new(usize::MAX);

#[repr(C, align(16))]
struct StackAlign<T>(T);

static STACK: SyncUnsafeCell<StackAlign<[u8; 128 * 1024]>> =
    SyncUnsafeCell::new(StackAlign([0; 128 * 1024]));

/// Entry point called by the bootloader
///
/// This function:
/// 1. Verifies BSS is zero and data section is non-zero
/// 2. Sets up the stack
/// 3. Jumps to kernel_main
#[unsafe(naked)]
#[no_mangle]
extern "C" fn kstart() -> ! {
    naked_asm!(
            "
            // BSS should already be zero
            cmp qword ptr [rip + {bss_test_zero}], 0
            jne .Lkstart_crash
            cmp qword ptr [rip + {data_test_nonzero}], 0
            je .Lkstart_crash

            // Set up stack - System V ABI requires 16-byte alignment before call
            // Since we jump rather than call, offset by 8 bytes
            lea rsp, [rip + {stack}+{stack_size}-24]

            // Call kernel_main(boot_info_addr = 0)
            // For x86_64 Limine, boot info comes from static structs, not args
            xor rdi, rdi
            call {kernel_main}

        .Lkstart_crash:
            xor rax, rax
            jmp rax
            ",
            bss_test_zero = sym BSS_TEST_ZERO,
            data_test_nonzero = sym DATA_TEST_NONZERO,
            stack = sym STACK,
            stack_size = const core::mem::size_of_val(&STACK),
            kernel_main = sym crate::kernel_main,
        );
}