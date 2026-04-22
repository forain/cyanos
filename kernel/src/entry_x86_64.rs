use core::arch::naked_asm;

#[repr(C, align(16))]
struct StackAlign<T>(T);

static STACK: StackAlign<[u8; 1024 * 1024]> = StackAlign([0; 1024 * 1024]);

#[unsafe(naked)]
#[no_mangle]
extern "C" fn _start() {
    naked_asm!(
        "
        // Set up stack (16-byte aligned as required by System V ABI)
        lea rsp, [rip + {stack} + {stack_size} - 24]

        // Debug: Write 'A' to COM1 (0x3F8) to test if we reach this point
        mov al, 'A'
        mov dx, 0x3F8
        out dx, al

        // Determine boot mode: Multiboot2 or Limine
        xor rdi, rdi            // Default to 0 (Limine mode)
        cmp eax, 0x36D76289     // Check for Multiboot2 magic
        jne .Lcall_kernel
        mov rdi, rbx            // Multiboot2: use info address

    .Lcall_kernel:
        // Debug: Write 'B' to COM1 before calling kernel_main
        mov al, 'B'
        mov dx, 0x3F8
        out dx, al

        call {kernel_main}

    .Lhalt:
        // Debug: Write 'C' if we return from kernel_main
        mov al, 'C'
        mov dx, 0x3F8
        out dx, al

        hlt
        jmp .Lhalt
        ",

        stack = sym STACK,
        stack_size = const core::mem::size_of::<StackAlign<[u8; 1024 * 1024]>>(),
        kernel_main = sym crate::kernel_main,
    );
}