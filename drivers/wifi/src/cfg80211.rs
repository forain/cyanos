//! cfg80211 — wireless configuration layer
//!
//! Ported from include/net/cfg80211.h and net/wireless/core.c.
//! In Linux this is the interface between mac80211 (or full-MAC drivers)
//! and userspace (via nl80211).  Here it sits between mac80211 and our nl80211
//! IPC layer.

use crate::ieee80211::{Channel, MacAddr, StatusCode};

// ── SSID ──────────────────────────────────────────────────────────────────────

pub const IEEE80211_MAX_SSID_LEN: usize = 32;

/// An SSID — mirrors `struct cfg80211_ssid`.
#[derive(Clone, Copy, Debug)]
pub struct Ssid {
    pub ssid: [u8; IEEE80211_MAX_SSID_LEN],
    pub len:  u8,
}

impl Ssid {
    pub fn new(s: &[u8]) -> Self {
        let len = s.len().min(IEEE80211_MAX_SSID_LEN);
        let mut ssid = [0u8; IEEE80211_MAX_SSID_LEN];
        ssid[..len].copy_from_slice(&s[..len]);
        Self { ssid, len: len as u8 }
    }

    pub fn as_bytes(&self) -> &[u8] { &self.ssid[..self.len as usize] }
}

// ── BSS ───────────────────────────────────────────────────────────────────────

/// A Basic Service Set (access point or IBSS peer).
/// Mirrors `struct cfg80211_bss` (the public-facing portion).
#[derive(Clone, Copy, Debug)]
pub struct Bss {
    pub bssid:              MacAddr,
    pub channel:            Channel,
    /// TSF timer value from the beacon/probe response.
    pub tsf:                u64,
    pub beacon_interval:    u16, // in TUs (1 TU = 1024 µs)
    pub capability:         u16,
    pub signal_mbm:         i32, // signal in mBm (100 * dBm)
    pub ssid:               Ssid,
    /// Age of the BSS record in ms.
    pub age_ms:             u32,
}

impl Bss {
    pub fn signal_dbm(&self) -> i32 { self.signal_mbm / 100 }
    pub fn is_infrastructure(&self) -> bool {
        self.capability & crate::ieee80211::capab::ESS != 0
    }
}

// ── Scan request / result ─────────────────────────────────────────────────────

pub const CFG80211_MAX_SCAN_SSIDS:    usize = 16;
pub const CFG80211_MAX_SCAN_CHANNELS: usize = 64;

/// Scan request — mirrors `struct cfg80211_scan_request`.
pub struct ScanRequest {
    pub ssids:       [Option<Ssid>; CFG80211_MAX_SCAN_SSIDS],
    pub n_ssids:     u8,
    pub channels:    [Option<Channel>; CFG80211_MAX_SCAN_CHANNELS],
    pub n_channels:  u8,
    /// IEs to include in probe requests.
    pub ie:          [u8; 256],
    pub ie_len:      u16,
    pub flags:       ScanFlags,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ScanFlags: u32 {
        const LOW_PRIORITY    = 1 << 0;
        const FLUSH           = 1 << 1;
        const AP              = 1 << 2;
        const RANDOM_ADDR     = 1 << 3;
        const FILS_MAX_CHANNEL = 1 << 4;
        const ACCEPT_BCAST_PROBE_RESP = 1 << 5;
        const OCE_PROBE_REQ_DEFERRAL = 1 << 6;
        const OCE_PROBE_REQ_HIGH_TX_RATE = 1 << 7;
        const COLOCATED_6GHZ  = 1 << 8;
    }
}

/// Scan results delivered to the cfg80211 core.
pub struct ScanResults {
    pub bss:   [Option<Bss>; 32],
    pub count: u8,
    pub aborted: bool,
}

impl ScanResults {
    pub fn new() -> Self {
        Self { bss: [None; 32], count: 0, aborted: false }
    }

    pub fn add(&mut self, bss: Bss) -> bool {
        if self.count as usize >= self.bss.len() { return false; }
        self.bss[self.count as usize] = Some(bss);
        self.count += 1;
        true
    }

    pub fn iter(&self) -> impl Iterator<Item = &Bss> {
        self.bss[..self.count as usize].iter().filter_map(|b| b.as_ref())
    }
}

// ── Connection parameters ─────────────────────────────────────────────────────

/// Authentication algorithm IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthType {
    OpenSystem,
    SharedKey,
    Ft,
    NetworkEap,
    Sae,
    FilsSharedKey,
    FilsSharedKeyPfs,
}

/// Connect parameters — mirrors `struct cfg80211_connect_params`.
pub struct ConnectParams {
    pub channel:    Option<Channel>,
    pub bssid:      Option<MacAddr>,
    pub ssid:       Ssid,
    pub auth_type:  AuthType,
    pub ie:         [u8; 512],
    pub ie_len:     u16,
    pub privacy:    bool,
    /// WEP key (legacy).
    pub key:        Option<[u8; 32]>,
    pub key_len:    u8,
}

/// Connection result — mirrors what cfg80211_connect_result() reports.
#[derive(Clone, Copy, Debug)]
pub struct ConnectResult {
    pub bssid:          MacAddr,
    pub req_ie:         [u8; 256],
    pub req_ie_len:     u16,
    pub resp_ie:        [u8; 256],
    pub resp_ie_len:    u16,
    pub status:         StatusCode,
    pub timeout:        bool,
}

// ── Key configuration ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CipherSuite {
    Wep40,
    Tkip,
    Ccmp128,
    Wep104,
    AesCmac,
    Gcmp128,
    Gcmp256,
    Ccmp256,
}

/// Key configuration — mirrors `struct key_params`.
pub struct KeyParams {
    pub key:     [u8; 32],
    pub key_len: u8,
    pub seq:     [u8; 16],
    pub seq_len: u8,
    pub cipher:  CipherSuite,
}

// ── Wireless device ops ───────────────────────────────────────────────────────

/// cfg80211 operations — the interface from cfg80211 down to the driver.
/// Mirrors `struct cfg80211_ops` from include/net/cfg80211.h.
pub trait WirelessOps {
    fn scan(&mut self, req: ScanRequest) -> Result<(), ()>;
    fn connect(&mut self, params: ConnectParams) -> Result<(), ()>;
    fn disconnect(&mut self, reason: u16) -> Result<(), ()>;
    fn add_key(&mut self, idx: u8, pairwise: bool, mac: Option<MacAddr>, params: KeyParams)
        -> Result<(), ()>;
    fn del_key(&mut self, idx: u8, pairwise: bool, mac: Option<MacAddr>) -> Result<(), ()>;
    fn set_default_key(&mut self, idx: u8) -> Result<(), ()>;
    fn set_tx_power(&mut self, power_mbm: i32) -> Result<(), ()>;
    fn get_tx_power(&self) -> i32;
    fn set_channel(&mut self, channel: Channel) -> Result<(), ()>;
}
