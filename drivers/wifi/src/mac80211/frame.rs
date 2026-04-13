//! 802.11 frame builder — mirrors ieee80211_build_* helpers from mac80211.

use crate::ieee80211::{fc, MacAddr};

pub const MAX_FRAME_LEN: usize = 2346; // 802.11 max MPDU size

/// Outgoing frame queued for transmission.
pub struct TxFrame {
    pub data: [u8; MAX_FRAME_LEN],
    pub len:  usize,
    /// Access Category (maps to TID / AC queue).
    pub ac:   u8,
}

impl TxFrame {
    fn new() -> Self {
        Self { data: [0; MAX_FRAME_LEN], len: 0, ac: 0 }
    }

    fn write(&mut self, src: &[u8]) {
        let end = self.len + src.len();
        self.data[self.len..end].copy_from_slice(src);
        self.len = end;
    }

    fn write_le16(&mut self, val: u16) {
        self.write(&val.to_le_bytes());
    }
}

// ── 3-address header helper ───────────────────────────────────────────────────

fn write_hdr(frame: &mut TxFrame, fc_val: u16, da: &MacAddr, sa: &MacAddr, bssid: &MacAddr) {
    frame.write_le16(fc_val);     // frame_control
    frame.write_le16(0);           // duration (filled by HW/firmware)
    frame.write(da);
    frame.write(sa);
    frame.write(bssid);
    frame.write_le16(0);           // seq_ctrl (filled by HW/firmware)
}

// ── Authentication frame ──────────────────────────────────────────────────────

/// Build an Open System Authentication frame (seq 1 or 3).
///
/// Mirrors ieee80211_send_auth() in net/mac80211/util.c.
pub fn build_auth_open(sa: &MacAddr, bssid: &MacAddr, seq: u16) -> TxFrame {
    let mut f = TxFrame::new();
    let fc_val = fc::FTYPE_MGMT | fc::STYPE_AUTH;
    write_hdr(&mut f, fc_val, bssid, sa, bssid);
    // Auth body: algorithm=0 (Open), seq, status=0
    f.write_le16(0); // Open System
    f.write_le16(seq);
    f.write_le16(0); // status = success
    f
}

/// Build a SAE (WPA3) auth commit frame (seq 1).
pub fn build_auth_sae(sa: &MacAddr, bssid: &MacAddr, sae_payload: &[u8]) -> TxFrame {
    let mut f = TxFrame::new();
    let fc_val = fc::FTYPE_MGMT | fc::STYPE_AUTH;
    write_hdr(&mut f, fc_val, bssid, sa, bssid);
    f.write_le16(3); // SAE algorithm
    f.write_le16(1); // seq
    f.write_le16(0); // status
    let copy_len = sae_payload.len().min(MAX_FRAME_LEN - f.len);
    f.write(&sae_payload[..copy_len]);
    f
}

// ── Association request ───────────────────────────────────────────────────────

/// Build an Association Request frame.
///
/// Mirrors ieee80211_send_assoc() in net/mac80211/mlme.c.
pub fn build_assoc_req(
    sa: &MacAddr,
    bssid: &MacAddr,
    capability: u16,
    listen_interval: u16,
    ssid: &[u8],
    supported_rates: &[u8],
) -> TxFrame {
    let mut f = TxFrame::new();
    let fc_val = fc::FTYPE_MGMT | fc::STYPE_ASSOC_REQ;
    write_hdr(&mut f, fc_val, bssid, sa, bssid);

    // Fixed-length fields
    f.write_le16(capability);
    f.write_le16(listen_interval);

    // SSID IE
    f.data[f.len] = crate::ieee80211::eid::SSID;
    f.data[f.len + 1] = ssid.len().min(32) as u8;
    f.len += 2;
    let slen = ssid.len().min(32);
    f.data[f.len..f.len + slen].copy_from_slice(&ssid[..slen]);
    f.len += slen;

    // Supported Rates IE
    let rlen = supported_rates.len().min(8);
    f.data[f.len] = crate::ieee80211::eid::SUPPORTED_RATES;
    f.data[f.len + 1] = rlen as u8;
    f.len += 2;
    f.data[f.len..f.len + rlen].copy_from_slice(&supported_rates[..rlen]);
    f.len += rlen;

    f
}

// ── Deauthentication / Disassociation ─────────────────────────────────────────

/// Build a Deauthentication frame.
pub fn build_deauth(sa: &MacAddr, bssid: &MacAddr, reason: u16) -> TxFrame {
    let mut f = TxFrame::new();
    write_hdr(&mut f, fc::FTYPE_MGMT | fc::STYPE_DEAUTH, bssid, sa, bssid);
    f.write_le16(reason);
    f
}

/// Build a Disassociation frame.
pub fn build_disassoc(sa: &MacAddr, bssid: &MacAddr, reason: u16) -> TxFrame {
    let mut f = TxFrame::new();
    write_hdr(&mut f, fc::FTYPE_MGMT | fc::STYPE_DISASSOC, bssid, sa, bssid);
    f.write_le16(reason);
    f
}

// ── Null function (power-save) ────────────────────────────────────────────────

/// Build a Null Function frame (used for PS poll / keep-alive).
pub fn build_nullfunc(sa: &MacAddr, bssid: &MacAddr, power_save: bool) -> TxFrame {
    let mut fc_val = fc::FTYPE_DATA | fc::STYPE_NULLFUNC | fc::TO_DS;
    if power_save { fc_val |= fc::PWR_MGT; }
    let mut f = TxFrame::new();
    write_hdr(&mut f, fc_val, bssid, sa, bssid);
    f
}

// ── Probe Request ─────────────────────────────────────────────────────────────

/// Build a Probe Request frame.
pub fn build_probe_req(
    sa: &MacAddr,
    ssid: &[u8], // empty = wildcard scan
    supported_rates: &[u8],
) -> TxFrame {
    let mut f = TxFrame::new();
    let fc_val = fc::FTYPE_MGMT | fc::STYPE_PROBE_REQ;
    write_hdr(&mut f, fc_val, &crate::ieee80211::BROADCAST_ADDR, sa,
              &crate::ieee80211::BROADCAST_ADDR);

    // SSID IE
    let slen = ssid.len().min(32);
    f.data[f.len] = crate::ieee80211::eid::SSID;
    f.data[f.len + 1] = slen as u8;
    f.len += 2;
    f.data[f.len..f.len + slen].copy_from_slice(&ssid[..slen]);
    f.len += slen;

    // Supported Rates IE
    let rlen = supported_rates.len().min(8);
    f.data[f.len] = crate::ieee80211::eid::SUPPORTED_RATES;
    f.data[f.len + 1] = rlen as u8;
    f.len += 2;
    f.data[f.len..f.len + rlen].copy_from_slice(&supported_rates[..rlen]);
    f.len += rlen;

    f
}
