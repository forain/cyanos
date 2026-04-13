//! Scan logic — mirrors net/mac80211/scan.c
//!
//! Handles received beacon / probe response frames and builds the BSS list.

use crate::ieee80211::{IeIter, eid, Channel, Band, ChannelFlags};
use crate::cfg80211::{Bss, Ssid};
use super::{Mac80211, Ieee80211Ops};

/// Parse a received Beacon or Probe Response and upsert a BSS entry.
///
/// Mirrors ieee80211_bss_info_update() / ieee80211_rx_mgmt_beacon() in scan.c.
pub fn handle_beacon_or_probe<D: Ieee80211Ops>(_mac: &mut Mac80211<D>, frame: &[u8]) {
    // Minimum: 24-byte header + 8-byte timestamp + 2 beacon interval + 2 capability
    if frame.len() < 36 { return; }

    // Source address = addr2 (bytes 10..16)
    let bssid: [u8; 6] = frame[16..22].try_into().unwrap_or([0; 6]);

    // Fixed parameters start at offset 24.
    let tsf = u64::from_le_bytes(frame[24..32].try_into().unwrap_or([0; 8]));
    let beacon_interval = u16::from_le_bytes([frame[32], frame[33]]);
    let capability      = u16::from_le_bytes([frame[34], frame[35]]);

    // Parse information elements.
    let ie_buf = &frame[36..];
    let mut ssid = Ssid::new(b"");
    let mut ds_channel: Option<u8> = None;

    for (id, payload) in IeIter::new(ie_buf) {
        match id {
            eid::SSID if payload.len() <= 32 => { ssid = Ssid::new(payload); }
            eid::DS_PARAMS if payload.len() >= 1 => { ds_channel = Some(payload[0]); }
            _ => {}
        }
    }

    // Build a minimal Channel descriptor.  In real mac80211 the channel is
    // looked up from the hardware's supported channel list.
    let channel = Channel {
        band:     Band::Ghz2,
        center_freq: ds_channel.map(|c| 2407 + c as u32 * 5).unwrap_or(2412),
        hw_value: ds_channel.unwrap_or(1) as u16,
        flags:    ChannelFlags::empty(),
        max_power:     20,
        max_reg_power: 20,
    };

    let bss = Bss {
        bssid,
        channel,
        tsf,
        beacon_interval,
        capability,
        signal_mbm: -7000, // placeholder: -70 dBm
        ssid,
        age_ms: 0,
    };

    // In real mac80211 the result is forwarded to cfg80211 which maintains
    // a kernel-managed BSS table.  Here we just hand it to the scan state.
    // (The real driver calls cfg80211_inform_bss_frame_data.)
    let _ = bss; // TODO: store in scan result cache
}
