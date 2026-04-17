//! PID-1 init task — first process after the kernel bootstraps.
//!
//! Sets up the in-kernel servers (VFS, net, TTY), probes hardware drivers,
//! then hands off to `init_server::init_main()` which runs the POSIX smoke
//! tests and a minimal shell demo before entering the event loop.

use crate::serial_print;
use wifi::mac80211::Mac80211;
use wifi::cfg80211::{ScanRequest, ScanFlags};

// ── Static I/O hooks for init_server ─────────────────────────────────────────

/// Kernel-side I/O callbacks passed to the init server library.
static INIT_IO: init_server::IoHooks = init_server::IoHooks {
    print_str:  |s|   crate::serial_print(s),
    write_raw:  |buf| crate::serial_write_raw(buf),
    read_byte:  ||    crate::serial_read_byte(),
};

// ── Driver probes ─────────────────────────────────────────────────────────────

fn probe_usb() {
    serial_print("[CYANOS] init: USB probe deferred (requires PCI/ECAM enumeration)\n");
}

fn probe_wifi() {
    let mut mac: Mac80211<wifi::virtio_wifi::VirtioWifi> = wifi::virtio_wifi::create();
    match mac.bring_up() {
        Ok(()) => {
            serial_print("[CYANOS] init: WiFi interface up\n");
            let req = ScanRequest {
                ssids:      [None; wifi::cfg80211::CFG80211_MAX_SCAN_SSIDS],
                n_ssids:    0,
                channels:   [None; wifi::cfg80211::CFG80211_MAX_SCAN_CHANNELS],
                n_channels: 0,
                ie:         [0u8; 256],
                ie_len:     0,
                flags:      ScanFlags::empty(),
            };
            match mac.scan(req) {
                Ok(())  => serial_print("[CYANOS] init: WiFi scan started\n"),
                Err(e)  => {
                    serial_print("[CYANOS] init: WiFi scan error: ");
                    serial_print(if e == -16 { "EBUSY\n" } else { "unknown\n" });
                }
            }
        }
        Err(e) => {
            serial_print("[CYANOS] init: WiFi bring_up failed: ");
            serial_print(if e == -19 { "ENODEV\n" } else { "unknown\n" });
        }
    }
}

// ── PID-1 task entry ──────────────────────────────────────────────────────────

/// Entry point for the kernel's PID-1 init task.  Never returns.
pub fn init_task_main() -> ! {
    serial_print("[CYANOS] init: task started (PID 1)\n");

    // ── Initialise in-kernel servers ──────────────────────────────────────────
    match vfs_server::init(1) {
        Some(port) => {
            crate::syscall::set_vfs_server_port(port);
            serial_print("[CYANOS] init: VFS server ready\n");
        }
        None => serial_print("[CYANOS] init: VFS server init FAILED\n"),
    }

    // ── IPC smoke test ────────────────────────────────────────────────────────
    match ipc::port::create(1) {
        Some(port) => {
            let _ = ipc::port::send(port, ipc::Message::empty());
            match ipc::port::recv(port) {
                Some(_) => serial_print("[CYANOS] init: IPC loopback OK\n"),
                None    => serial_print("[CYANOS] init: IPC loopback FAILED\n"),
            }
            ipc::port::close(port);
        }
        None => serial_print("[CYANOS] init: IPC port alloc failed\n"),
    }

    // ── Driver probe ──────────────────────────────────────────────────────────
    probe_usb();
    probe_wifi();

    // ── Hand off to init server ───────────────────────────────────────────────
    init_server::init_main(&INIT_IO);
}
