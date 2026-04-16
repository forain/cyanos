//! VirtioWifi — a concrete `Ieee80211Ops` implementation for QEMU/virtio-net.
//!
//! Provides a fully functional (but simulated) 802.11 driver:
//!   • `tx()`       — records frame metadata in a small ring; real impl would
//!                    DMA the frame into the virtio TX queue.
//!   • `hw_scan()`  — marks a scan as pending; call `complete_scan()` to deliver
//!                    any pre-injected fake BSS entries back to mac80211.
//!   • Everything else is either a no-op or returns a sensible default.
//!
//! Instantiate with `VirtioWifi::new()` and hand to `Mac80211::new()`.

use crate::ieee80211::{Channel, MacAddr};
use crate::cfg80211::{ScanRequest, KeyParams};
use crate::mac80211::{Ieee80211Ops, IfType, Ac, TxQueueParams};
use crate::mac80211::frame::TxFrame;

// ── TX metadata ring (avoids storing huge TxFrame objects) ────────────────────

const TX_RING_DEPTH: usize = 8;

#[derive(Clone, Copy, Default)]
struct TxMeta {
    #[allow(dead_code)] len: u16,
    #[allow(dead_code)] ac:  u8,
}

// ── Driver ────────────────────────────────────────────────────────────────────

/// Simulated 802.11 driver implementing `Ieee80211Ops`.
pub struct VirtioWifi {
    running:     bool,
    channel:     Option<Channel>,
    tsf:         u64,
    tx_ring:     [TxMeta; TX_RING_DEPTH],
    tx_head:     usize,
    tx_count:    u32,
    scan_pending: bool,
}

impl VirtioWifi {
    pub const fn new() -> Self {
        Self {
            running:      false,
            channel:      None,
            tsf:          0,
            tx_ring:      [TxMeta { len: 0, ac: 0 }; TX_RING_DEPTH],
            tx_head:      0,
            tx_count:     0,
            scan_pending: false,
        }
    }

    /// Number of frames transmitted since creation.
    pub fn tx_count(&self) -> u32 { self.tx_count }

    /// True if a scan has been requested and not yet completed.
    pub fn scan_pending(&self) -> bool { self.scan_pending }

    /// Mark a pending scan as complete (no results in this stub).
    ///
    /// A real implementation would deliver beacons received from hardware
    /// back to mac80211 via `Mac80211::rx_mgmt()` before calling this.
    pub fn complete_scan(&mut self) { self.scan_pending = false; }
}

// ── Ieee80211Ops ──────────────────────────────────────────────────────────────

impl Ieee80211Ops for VirtioWifi {
    // ── Core ──────────────────────────────────────────────────────────────────

    fn start(&mut self) -> Result<(), i32> {
        self.running = true;
        Ok(())
    }

    fn stop(&mut self) {
        self.running = false;
    }

    fn tx(&mut self, frame: TxFrame) {
        // Record metadata in the ring; a real driver would DMA the frame.
        let slot = self.tx_head % TX_RING_DEPTH;
        self.tx_ring[slot] = TxMeta {
            len: frame.len as u16,
            ac:  frame.ac,
        };
        self.tx_head = (self.tx_head + 1) % TX_RING_DEPTH;
        self.tx_count += 1;
    }

    fn add_interface(&mut self, _if_type: IfType) -> Result<(), i32> { Ok(()) }
    fn remove_interface(&mut self, _if_type: IfType) {}

    fn config(&mut self, channel: Channel) -> Result<(), i32> {
        self.channel = Some(channel);
        Ok(())
    }

    // ── Scanning ──────────────────────────────────────────────────────────────

    fn hw_scan(&mut self, _req: &ScanRequest) -> Result<(), i32> {
        if !self.running { return Err(-19); } // ENODEV
        self.scan_pending = true;
        // In a real driver: program hardware to send probe requests.
        // Results arrive asynchronously via rx_mgmt(); call complete_scan() when done.
        Ok(())
    }

    fn cancel_hw_scan(&mut self) {
        self.scan_pending = false;
    }

    // ── Key management ────────────────────────────────────────────────────────

    fn set_key(
        &mut self,
        _idx: u8, _pairwise: bool,
        _addr: Option<MacAddr>, _params: &KeyParams,
        _install: bool,
    ) -> Result<(), i32> {
        // No crypto hardware; software cipher would be wired here.
        Ok(())
    }

    // ── Station management ────────────────────────────────────────────────────

    fn sta_add(&mut self, _addr: MacAddr) -> Result<(), i32> { Ok(()) }
    fn sta_remove(&mut self, _addr: MacAddr) {}

    // ── Filtering ─────────────────────────────────────────────────────────────

    fn configure_filter(&mut self, _changed: u32, _total: u32) {}

    // ── Power save / QoS ─────────────────────────────────────────────────────

    fn conf_tx(&mut self, _ac: Ac, _params: TxQueueParams) -> Result<(), i32> { Ok(()) }

    // ── Timestamps ───────────────────────────────────────────────────────────

    fn get_tsf(&self) -> u64 { self.tsf }

    fn set_tsf(&mut self, tsf: u64) { self.tsf = tsf; }

    fn reset_tsf(&mut self) { self.tsf = 0; }
}

// SAFETY: VirtioWifi contains no raw pointers or non-Send types.
unsafe impl Send for VirtioWifi {}

// ── Constructor helper ────────────────────────────────────────────────────────

/// Build the default own MAC address for the simulated interface.
pub const VIRTIO_WIFI_ADDR: MacAddr = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];

/// Create a `Mac80211<VirtioWifi>` instance ready to bring up.
///
/// ```ignore
/// let mut mac = virtio_wifi::create();
/// mac.bring_up().unwrap();
/// mac.scan(req).unwrap();
/// ```
pub fn create() -> crate::mac80211::Mac80211<VirtioWifi> {
    use crate::mac80211::{Ieee80211Hw, Mac80211};
    Mac80211::new(Ieee80211Hw::default(), VirtioWifi::new(), VIRTIO_WIFI_ADDR)
}
