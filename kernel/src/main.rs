//! Cyanos kernel entry point.
//!
//! `kernel_main` is called by the arch-specific `_start` stub (entry_*.s)
//! after the stack is set up and BSS is zeroed.  It receives the physical
//! address of the boot information structure (MBI for x86-64, DTB for AArch64).

#![no_std]
#![no_main]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![cfg_attr(target_arch = "x86_64", feature(sync_unsafe_cell))]

use core::panic::PanicInfo;
use core::alloc::{GlobalAlloc, Layout};

// ── Global Allocator ─────────────────────────────────────────────────────────

struct DummyAllocator;

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
    }
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

#[cfg(target_arch = "x86_64")]
extern crate arch_x86_64;
#[cfg(target_arch = "aarch64")]
extern crate arch_aarch64;

mod init;
mod syscall;
mod mem;

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("entry_aarch64.s"));

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("entry_x86_64.s"));

// ── Limine Requests ──────────────────────────────────────────────────────────

#[no_mangle]
#[link_section = ".limine_reqs"]
#[used]
pub static mut limine_base_revision: [u64; 3] = [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, 0];

#[no_mangle]
#[link_section = ".limine_reqs"]
#[used]
pub static ENTRY_POINT_REQUEST: boot::limine::EntryPointRequest = boot::limine::EntryPointRequest {
    id:       [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b, 0x13d86c035a1cd3e1, 0x2b0caa89d8f3026a],
    revision: 0,
    response: core::cell::UnsafeCell::new(core::ptr::null()),
    entry_point: crate::_start,
};

#[no_mangle]
#[link_section = ".limine_reqs"]
#[used]
pub static HHDM_REQUEST: boot::limine::Request<boot::limine::HhdmResponse> = boot::limine::Request {
    id:       [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b, 0x48d1236851f558a7, 0xff2b207279178864],
    revision: 0,
    response: core::cell::UnsafeCell::new(core::ptr::null()),
};

#[no_mangle]
#[link_section = ".limine_reqs"]
#[used]
pub static MEMMAP_REQUEST: boot::limine::Request<boot::limine::MemMapResponse> = boot::limine::Request {
    id:       [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b, 0x67cf3d9d378a806f, 0xe304acdfc50c3c62],
    revision: 0,
    response: core::cell::UnsafeCell::new(core::ptr::null()),
};

#[no_mangle]
#[link_section = ".limine_reqs"]
#[used]
pub static FRAMEBUFFER_REQUEST: boot::limine::Request<boot::limine::FramebufferResponse> = boot::limine::Request {
    id:       [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b, 0x9d5827dcd881dd75, 0xa3148604f6fab11b],
    revision: 0,
    response: core::cell::UnsafeCell::new(core::ptr::null()),
};

// ── Serial port ──────────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
unsafe fn early_serial_init() {
    use core::arch::asm;
    macro_rules! outb {
        ($port:expr, $val:expr) => {
            asm!("out dx, al", in("dx") $port as u16, in("al") $val as u8,
                 options(nomem, nostack));
        }
    }
    outb!(0x3F9, 0x00u8);
    outb!(0x3FB, 0x80u8);
    outb!(0x3F8, 0x01u8);
    outb!(0x3F9, 0x00u8);
    outb!(0x3FB, 0x03u8);
    outb!(0x3FA, 0xC7u8);
}

#[cfg(target_arch = "aarch64")]
unsafe fn early_serial_init() {}

#[no_mangle]
pub fn serial_read_byte() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::asm;
        let mut status: u8;
        asm!("in al, dx", out("al") status, in("dx") 0x3FDu16, options(nomem, nostack));
        if status & 0x01 != 0 {
            let mut b: u8;
            asm!("in al, dx", out("al") b, in("dx") 0x3F8u16, options(nomem, nostack));
            return Some(b);
        }
        return None;
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let base = 0x09000000usize;
        let fr = (base + 0x18) as *const u32;
        if fr.read_volatile() & (1 << 4) != 0 { return None; }
        let dr = base as *const u32;
        Some((dr.read_volatile() & 0xFF) as u8)
    }
}

#[no_mangle]
pub unsafe fn serial_write_byte(b: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::asm;
        loop {
            let mut status: u8;
            asm!("in al, dx", out("al") status, in("dx") 0x3FDu16, options(nomem, nostack));
            if status & 0x20 != 0 { break; }
        }
        asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nomem, nostack));
    }
    #[cfg(target_arch = "aarch64")]
    {
        let base = 0x09000000usize;
        let fr = (base + 0x18) as *const u32;
        while fr.read_volatile() & (1 << 5) != 0 {}
        let dr = base as *mut u32;
        dr.write_volatile(b as u32);
    }
}

pub fn serial_print(s: &str) {
    for b in s.bytes() {
        unsafe { serial_write_byte(b); }
        if b == b'\n' { unsafe { serial_write_byte(b'\r'); } }
    }
}

#[no_mangle]
pub fn serial_write_raw(buf: &[u8]) {
    for &b in buf { unsafe { serial_write_byte(b); } }
}

#[no_mangle]
pub extern "C" fn serial_print_bytes(ptr: *const u8, len: usize) {
    if ptr.is_null() { return; }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    serial_write_raw(bytes);
}

fn print_number(n: u32) {
    let mut buf = [0u8; 8];
    for i in 0..8 {
        let nibble = (n >> ((7 - i) * 4)) & 0xF;
        buf[i] = if nibble < 10 { b'0' + nibble as u8 } else { b'A' + (nibble - 10) as u8 };
    }
    let s = unsafe { core::str::from_utf8_unchecked(&buf) };
    serial_print(s);
}

// ── Kernel main ───────────────────────────────────────────────────────────────

#[repr(C, align(16))]
struct Stack<const N: usize>([u8; N]);

#[no_mangle]
static mut EARLY_STACK: Stack<0x10000> = Stack([0; 0x10000]);

extern "C" {
    fn _start() -> !;
}

#[no_mangle]
pub extern "C" fn kernel_main(boot_info_addr: usize) -> ! {
    unsafe { early_serial_init(); }
    serial_print("\n[CYANOS] Kernel starting...\n");

    serial_print("[INIT] Parsing boot info...\n");
    let mut boot_info = if boot_info_addr == 0 {
        unsafe { boot::limine::parse() }
    } else {
        unsafe { boot::multiboot2::parse(boot_info_addr) }
    };

    if boot_info_addr == 0 {
        let resp = unsafe { *HHDM_REQUEST.response.get() };
        if !resp.is_null() {
            boot_info.hhdm_offset = unsafe { (*resp).offset };
            serial_print("  HHDM Offset: 0x");
            print_number((boot_info.hhdm_offset >> 32) as u32);
            print_number(boot_info.hhdm_offset as u32);
            serial_print("\n");
        } else {
            serial_print("  WARNING: HHDM Request NOT satisfied, using fallback\n");
            boot_info.hhdm_offset = 0xffff800000000000;
        }

        // Set the global offset for the architecture driver
        #[cfg(target_arch = "x86_64")]
        unsafe { arch_x86_64::apic::set_hhdm_offset(boot_info.hhdm_offset); }
    }

    serial_print("[INIT] Architecture-specific init...\n");
    #[cfg(target_arch = "x86_64")]
    arch_x86_64::init(&boot_info);
    #[cfg(target_arch = "aarch64")]
    arch_aarch64::init(&boot_info);

    serial_print("[INIT] Memory management init...\n");
    mm::init_with_map(boot_info.memory_regions());

    serial_print("[INIT] Scheduler init...\n");
    sched::init();

    serial_print("[INIT] Spawning init task...\n");
    init::init_task_main();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_print("\n[CYANOS] KERNEL PANIC: ");
    if let Some(msg) = info.message().as_str() {
        serial_print(msg);
    }
    loop { core::hint::spin_loop(); }
}
