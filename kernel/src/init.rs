//! PID-1 init task — first process after the kernel bootstraps.
//!
//! In a microkernel all "system" work lives in user-space servers that init
//! spawns and supervises.  For now this is a kernel-mode task that exercises
//! context switching and IPC before parking in a yield loop.

use crate::serial_print;
use wifi::mac80211::Mac80211;
use wifi::cfg80211::{ScanRequest, ScanFlags};

/// Probe USB xHCI controller and bring it up.
///
/// # Why no static MMIO address
///
/// QEMU `-machine virt` (AArch64) exposes USB only when the user passes
/// `-device qemu-xhci,id=xhci` on the command line.  When present, the
/// controller's MMIO base is assigned dynamically through ECAM (PCI
/// configuration space) — there is no fixed physical address.  The correct
/// way to find it is to:
///   1. Walk the DTB `/soc/pcie@...` node to get the ECAM base.
///   2. Enumerate PCI class 0x0C03 (USB/xHCI) devices.
///   3. Read BAR0 of the discovered device for the MMIO base.
///
/// DTB enumeration is not yet implemented (tracked separately).  Until then
/// USB probe is skipped with an informational message.
///
/// On x86-64 the same applies: xHCI is a PCI device discovered through ACPI
/// or direct PCI config-space scan, not a fixed MMIO address.
fn probe_usb() {
    serial_print("[CYANOS] init: USB probe deferred (requires PCI/ECAM enumeration)\n");
}

/// Probe WiFi using the virtio-wifi stub and trigger an initial scan.
///
/// The virtio-wifi driver is a no-hardware simulation suitable for QEMU.
/// A real driver would register with mac80211 via the `Ieee80211Ops` trait.
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

/// Entry point for the init task.  Must never return (`fn() -> !`).
pub fn init_task_main() -> ! {
    serial_print("[CYANOS] init: task started (PID 1)\n");

    // ── Driver probe ──────────────────────────────────────────────────────────
    probe_usb();
    probe_wifi();

    // ── IPC smoke test ────────────────────────────────────────────────────────
    // Allocate a port, send one message to ourselves, receive it back.
    // This exercises the full send → unblock → recv path.
    match ipc::port::create(1) {
        Some(port) => {
            let _ = ipc::port::send(port, ipc::Message::empty());

            // recv() is non-blocking; the message was just enqueued so it is
            // immediately available.
            match ipc::port::recv(port) {
                Some(_) => serial_print("[CYANOS] init: IPC loopback OK\n"),
                None    => serial_print("[CYANOS] init: IPC loopback FAILED — no message\n"),
            }

            // Enter an event loop: service messages and print a heartbeat every
            // 100 ticks (≈1 s at 100 Hz) so we can confirm the timer fires.
            serial_print("[CYANOS] init: entering event loop\n");
            let mut last_heartbeat: u64 = 0;
            loop {
                // Service all pending messages without blocking.
                while let Some(_msg) = ipc::port::recv(port) {
                    serial_print("[CYANOS] init: message received\n");
                }

                // Heartbeat: print once per second.
                let t = sched::ticks();
                if t.wrapping_sub(last_heartbeat) >= 100 {
                    last_heartbeat = t;
                    serial_print("[CYANOS] init: heartbeat\n");
                }

                // Yield rather than hard-block so the tick check above can fire.
                sched::yield_now();
            }
        }
        None => {
            serial_print("[CYANOS] init: IPC port alloc failed — yielding forever\n");
            loop { sched::yield_now(); }
        }
    }
}
