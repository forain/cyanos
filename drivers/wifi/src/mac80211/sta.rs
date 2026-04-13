//! Station (peer) table — mirrors net/mac80211/sta_info.c
//!
//! In AP or IBSS mode we track each associated station here.

use crate::ieee80211::MacAddr;

pub const STA_HASH_SIZE: usize = 256;
pub const MAX_STA: usize = 2007; // Linux: STA_MAX_STA

/// Station (peer) state machine.  Mirrors `enum ieee80211_sta_state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StaState {
    /// Not yet authenticated.
    None,
    /// Authenticated but not associated.
    Auth,
    /// Fully associated.
    Assoc,
    /// Authorized (keys installed, traffic allowed).
    Authorized,
}

/// Rate information for a single rate.
#[derive(Clone, Copy, Debug, Default)]
pub struct RateInfo {
    pub legacy: u16, // in 100 kbps
    pub mcs:    u8,
    pub nss:    u8,
    pub bw:     u8,  // 0=20, 1=40, 2=80, 3=160 MHz
    pub flags:  RateFlags,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct RateFlags: u8 {
        const MCS      = 1 << 0;
        const VHT_MCS  = 1 << 1;
        const SHORT_GI = 1 << 2;
        const HE_MCS   = 1 << 3;
    }
}

/// Per-TID block-ACK session state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaState {
    None,
    /// ADDBA Request sent.
    PendingResp,
    /// Session established.
    Established,
}

/// A station entry — mirrors `struct sta_info` in sta_info.h.
pub struct StaInfo {
    pub addr:          MacAddr,
    pub state:         StaState,
    pub aid:           u16,      // Association ID (AP-assigned)

    // ── Rate control ──────────────────────────────────────────────────────────
    pub last_rx_rate:  RateInfo,
    pub last_tx_rate:  RateInfo,

    // ── Statistics ────────────────────────────────────────────────────────────
    pub rx_packets:    u64,
    pub rx_bytes:      u64,
    pub tx_packets:    u64,
    pub tx_bytes:      u64,
    pub rx_dropped:    u32,
    pub tx_failed:     u32,

    // ── Signal ────────────────────────────────────────────────────────────────
    /// Signal strength of last frame in mBm.
    pub signal:        i32,
    /// Exponential moving average of signal.
    pub signal_avg:    i32,

    // ── Block-ACK ─────────────────────────────────────────────────────────────
    /// BA state per TID (0–7).
    pub ba_rx: [BaState; 8],
    pub ba_tx: [BaState; 8],

    // ── Capabilities ─────────────────────────────────────────────────────────
    pub ht_supported:  bool,
    pub vht_supported: bool,
    pub he_supported:  bool,

    /// WMM/QoS capable.
    pub qos:           bool,
}

impl StaInfo {
    pub fn new(addr: MacAddr) -> Self {
        Self {
            addr,
            state:         StaState::None,
            aid:           0,
            last_rx_rate:  RateInfo::default(),
            last_tx_rate:  RateInfo::default(),
            rx_packets:    0, rx_bytes: 0,
            tx_packets:    0, tx_bytes: 0,
            rx_dropped:    0, tx_failed: 0,
            signal:        0, signal_avg: 0,
            ba_rx: [BaState::None; 8],
            ba_tx: [BaState::None; 8],
            ht_supported:  false,
            vht_supported: false,
            he_supported:  false,
            qos:           false,
        }
    }

    pub fn is_authorized(&self) -> bool { self.state >= StaState::Authorized }
}

/// Station table — fixed-size hash map keyed by the last byte of MAC address.
pub struct StaTable {
    slots: [Option<StaInfo>; STA_HASH_SIZE],
    count: usize,
}

impl StaTable {
    pub const fn new() -> Self { Self { slots: [const { None }; STA_HASH_SIZE], count: 0 } }

    fn hash(addr: &MacAddr) -> usize { addr[5] as usize }

    pub fn insert(&mut self, sta: StaInfo) -> bool {
        let h = Self::hash(&sta.addr);
        if self.slots[h].is_none() {
            self.slots[h] = Some(sta);
            self.count += 1;
            true
        } else { false }
    }

    pub fn remove(&mut self, addr: &MacAddr) -> Option<StaInfo> {
        let h = Self::hash(addr);
        if self.slots[h].as_ref().map(|s| &s.addr) == Some(addr) {
            self.count -= 1;
            self.slots[h].take()
        } else { None }
    }

    pub fn get(&self, addr: &MacAddr) -> Option<&StaInfo> {
        let h = Self::hash(addr);
        self.slots[h].as_ref().filter(|s| &s.addr == addr)
    }

    pub fn get_mut(&mut self, addr: &MacAddr) -> Option<&mut StaInfo> {
        let h = Self::hash(addr);
        self.slots[h].as_mut().filter(|s| &s.addr == addr)
    }

    pub fn count(&self) -> usize { self.count }
}
