use core::arch::naked_asm;

#[repr(C, align(16))]
struct StackAlign<T>(T);

static STACK: StackAlign<[u8; 1024 * 1024]> = StackAlign([0; 1024 * 1024]);

#[unsafe(naked)]
#[no_mangle]
extern "C" fn _start() {
    naked_asm!(
        "
        // Set up stack (16-byte aligned)
        adrp x1, {stack}
        add x1, x1, :lo12:{stack}
        mov x2, {stack_size} - 16
        add sp, x1, x2

        // For aarch64, boot_info_addr is 0 (use Limine mode)
        mov x0, 0
        mov lr, 0
        b {kernel_main}

    .Lhalt:
        wfi
        b .Lhalt
        ",

        stack = sym STACK,
        stack_size = const core::mem::size_of::<StackAlign<[u8; 1024 * 1024]>>(),
        kernel_main = sym crate::kernel_main,
    );
}