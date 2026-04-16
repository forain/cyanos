//! Authentication / Association state machine — mirrors net/mac80211/mlme.c

use super::{Mac80211, Ieee80211Ops, StaConnState};
extern crate ipc;
use super::frame::{build_assoc_req, TxFrame};

// ── Authentication (Open System) ──────────────────────────────────────────────

/// Called when an Authentication frame is received.
///
/// Mirrors ieee80211_rx_mgmt_auth() in mlme.c.
pub fn handle_auth<D: Ieee80211Ops>(mac: &mut Mac80211<D>, frame: &[u8]) {
    if mac.state() != StaConnState::Authenticating { return; }
    // Auth frame body: algorithm(2) + seq(2) + status(2)
    // Header is 24 bytes.
    if frame.len() < 30 { return; }

    let seq    = u16::from_le_bytes([frame[26], frame[27]]);
    let status = u16::from_le_bytes([frame[28], frame[29]]);

    if seq != 2 { return; } // we expect seq=2 (AP's response to our seq=1)

    if status != 0 {
        // Auth rejected — go back to disconnected.
        mac.scan_done(); // reuse: sets state to Disconnected
        return;
    }

    // Authentication succeeded — send Association Request.
    if let Some(bss) = mac.current_bss() {
        let bssid  = bss.bssid;
        let capab  = bss.capability;
        let ssid_b = {
            let s = bss.ssid;
            let mut buf = [0u8; 32];
            buf[..s.len as usize].copy_from_slice(s.as_bytes());
            (buf, s.len as usize)
        };
        let own_addr = *mac.own_addr();
        let assoc = build_assoc_req(
            &own_addr, &bssid, capab, 10,
            &ssid_b.0[..ssid_b.1],
            &[0x82, 0x84, 0x8B, 0x96, 0x24, 0x30, 0x48, 0x6C], // 802.11g basic rates
        );
        // Advance state before TX so any fast completion is handled.
        let _ = bss; // release borrow before calling driver
        mac.set_state(StaConnState::Associating);
        mac.driver_tx(assoc);
    }
}

// ── Association ───────────────────────────────────────────────────────────────

/// Called when an Association Response frame is received.
///
/// Mirrors ieee80211_rx_mgmt_assoc_resp() in mlme.c.
pub fn handle_assoc_resp<D: Ieee80211Ops>(mac: &mut Mac80211<D>, frame: &[u8]) {
    if mac.state() != StaConnState::Associating { return; }
    // Assoc resp body: capability(2) + status(2) + AID(2) [+ IEs]
    if frame.len() < 30 { return; }

    let status = u16::from_le_bytes([frame[26], frame[27]]);
    let _aid   = u16::from_le_bytes([frame[28], frame[29]]) & 0x3FFF;

    if status != 0 {
        mac.set_state(StaConnState::Disconnected);
        return;
    }

    mac.set_state(StaConnState::Associated);

    // Notify the nl80211 userspace daemon via IPC.
    // BSSID is at bytes 16..22 of the 802.11 MAC header.
    let bssid: crate::ieee80211::MacAddr = frame[16..22].try_into().unwrap_or([0u8; 6]);
    mac.notify_associated(&bssid, _aid);
}

// ── Private Mac80211 accessors needed by this module ─────────────────────────
// These are defined as inherent methods on Mac80211 in mod.rs; the impl block
// here adds the extra helpers this module needs without violating privacy.
impl<D: Ieee80211Ops> Mac80211<D> {
    pub(super) fn own_addr(&self) -> &crate::ieee80211::MacAddr { &self.own_addr }
    pub(super) fn set_state(&mut self, s: StaConnState) { self.state = s; }
    pub(super) fn driver_tx(&mut self, f: TxFrame) { self.driver.tx(f); }

    /// Send an nl80211 IPC notification if a port is registered.
    ///
    /// Called from `handle_assoc_resp` on successful association.
    /// Payload: 6-byte BSSID + 2-byte AID (LE).
    pub(super) fn notify_associated(&self, bssid: &crate::ieee80211::MacAddr, aid: u16) {
        if let Some(port) = self.nl80211_port {
            let mut payload = [0u8; crate::ieee80211::ETH_ALEN + 2];
            payload[..crate::ieee80211::ETH_ALEN].copy_from_slice(bssid);
            payload[crate::ieee80211::ETH_ALEN..].copy_from_slice(&aid.to_le_bytes());
            let msg = crate::nl80211::encode(crate::nl80211::Nl80211Cmd::Connect, &payload);
            let _ = ipc::port::send(port, msg);
        }
    }
}
