//! mac80211 — software MAC layer
//!
//! Ported from net/mac80211/.  Provides the 802.11 state machine, frame
//! building/parsing, scan logic, authentication, and association.
//!
//! Hardware-specific drivers implement the `Ieee80211Ops` trait and register
//! with the mac80211 core via `Ieee80211Hw`.

pub mod frame;
pub mod scan;
pub mod auth;
pub mod sta;

use crate::ieee80211::{Channel, MacAddr};
use crate::cfg80211::{Bss, ScanRequest, ConnectParams, KeyParams};
use frame::TxFrame;

// ── Hardware flags ────────────────────────────────────────────────────────────

bitflags::bitflags! {
    /// ieee80211_hw.flags — hardware capability bits.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct HwFlags: u64 {
        /// HW computes and checks FCS.
        const HAS_RATE_CONTROL          = 1 << 0;
        const RX_INCLUDES_FCS           = 1 << 1;
        const HOST_BROADCAST_PS_BUFFERING = 1 << 2;
        const SIGNAL_UNSPEC             = 1 << 3;
        const SIGNAL_DBM                = 1 << 4;
        const NEED_DTIM_BEFORE_ASSOC    = 1 << 5;
        const SPECTRUM_MGMT             = 1 << 6;
        const AMPDU_AGGREGATION         = 1 << 7;
        const SUPPORTS_PS               = 1 << 8;
        const PS_NULLFUNC_STACK         = 1 << 9;
        const SUPPORTS_DYNAMIC_PS       = 1 << 10;
        const MFP_CAPABLE               = 1 << 11;
        const WANT_MONITOR_VIF          = 1 << 12;
        const NO_AUTO_VIF               = 1 << 13;
        const SW_CRYPTO_CONTROL         = 1 << 14;
        const SUPPORT_FAST_XMIT         = 1 << 15;
        const REPORTS_TX_ACK_STATUS     = 1 << 16;
        const CONNECTION_MONITOR        = 1 << 17;
        const QUEUE_CONTROL             = 1 << 18;
        const SUPPORTS_PER_STA_GTK      = 1 << 19;
        const AP_LINK_PS                = 1 << 20;
        const TX_AMPDU_SETUP_IN_HW      = 1 << 21;
        const SUPPORTS_RC_TABLE         = 1 << 22;
        const P2P_DEV_ADDR_FOR_INTF     = 1 << 23;
        const TIMING_BEACON_ONLY        = 1 << 24;
        const USES_RSS                  = 1 << 25;
        const TX_AMSDU                  = 1 << 26;
        const TX_FRAG_LIST              = 1 << 27;
        const REPORTS_LOW_ACK           = 1 << 28;
        const SUPPORTS_TX_FRAG         = 1 << 29;
        const SUPPORTS_TDLS_BUFFER_STA  = 1 << 30;
        const DEAUTH_NEED_MGD_TX_PREP   = 1 << 31;
        const DOESNT_SUPPORT_QOS_NDP    = 1 << 32;
        const BUFF_MMPDU_TXQ            = 1 << 33;
        const SUPPORTS_VHT_EXT_NSS_BW   = 1 << 34;
        const STA_MMPDU_TXQ             = 1 << 35;
        const TX_STATUS_NO_AMPDU_LEN    = 1 << 36;
        const SUPPORTS_MULTI_BSSID      = 1 << 37;
        const SUPPORTS_ONLY_HE_MULTI_BSSID = 1 << 38;
    }
}

// ── Virtual interface type ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfType {
    Station,
    Ap,
    Monitor,
    Ibss,
    Mesh,
    P2pClient,
    P2pGo,
    P2pDevice,
    Nan,
}

// ── TX queue AC mapping ───────────────────────────────────────────────────────

pub const IEEE80211_NUM_ACS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ac {
    Vo = 0, // Voice
    Vi = 1, // Video
    Be = 2, // Best Effort
    Bk = 3, // Background
}

/// Per-AC TX queue parameters.  Mirrors `struct ieee80211_tx_queue_params`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TxQueueParams {
    pub txop:  u16, // TXOP limit in 32 µs units
    pub cw_min: u16,
    pub cw_max: u16,
    pub aifs:  u8,
    pub uapsd: bool,
}

// ── ieee80211_hw ──────────────────────────────────────────────────────────────

/// Hardware description — passed to mac80211 during driver registration.
/// Mirrors the public fields of `struct ieee80211_hw`.
pub struct Ieee80211Hw {
    pub flags:               HwFlags,
    /// Extra headroom required in Tx SKBs.
    pub extra_tx_headroom:   u16,
    /// Channel change time in µs.
    pub channel_change_time: u32,
    pub max_signal:          i32,
    pub max_antenna_gain:    i32,
    /// Number of simultaneous TX queues.
    pub queues:              u8,
    /// Rate table size per band.
    pub max_rates:           u8,
    pub vif_data_size:       u16,
    pub sta_data_size:       u16,
}

impl Default for Ieee80211Hw {
    fn default() -> Self {
        Self {
            flags: HwFlags::empty(),
            extra_tx_headroom:   0,
            channel_change_time: 100_000,
            max_signal:          -10,
            max_antenna_gain:    0,
            queues:              4,
            max_rates:           4,
            vif_data_size:       0,
            sta_data_size:       0,
        }
    }
}

// ── ieee80211_ops ─────────────────────────────────────────────────────────────

/// Driver callbacks — mirrors `struct ieee80211_ops`.
///
/// All methods take `&mut self` (the driver instance) and follow the same
/// contract as their Linux counterparts.
pub trait Ieee80211Ops: Send {
    // ── Core ──────────────────────────────────────────────────────────────────
    /// Enable the hardware.
    fn start(&mut self) -> Result<(), i32>;
    /// Disable the hardware.
    fn stop(&mut self);
    /// Transmit a frame. Must not block.
    fn tx(&mut self, frame: TxFrame);
    /// Add a virtual interface.
    fn add_interface(&mut self, if_type: IfType) -> Result<(), i32>;
    /// Remove a virtual interface.
    fn remove_interface(&mut self, if_type: IfType);
    /// Configure the radio: channel, power, etc.
    fn config(&mut self, channel: Channel) -> Result<(), i32>;

    // ── Scanning ──────────────────────────────────────────────────────────────
    /// Start a hardware-assisted scan.
    fn hw_scan(&mut self, req: &ScanRequest) -> Result<(), i32>;
    /// Cancel an in-progress scan.
    fn cancel_hw_scan(&mut self);

    // ── Key management ────────────────────────────────────────────────────────
    fn set_key(&mut self, idx: u8, pairwise: bool, addr: Option<MacAddr>,
               params: &KeyParams, install: bool) -> Result<(), i32>;

    // ── Station management ────────────────────────────────────────────────────
    fn sta_add(&mut self, addr: MacAddr) -> Result<(), i32>;
    fn sta_remove(&mut self, addr: MacAddr);

    // ── Filtering ─────────────────────────────────────────────────────────────
    /// Set RX packet filter flags.
    fn configure_filter(&mut self, changed: u32, total: u32);

    // ── Power save ────────────────────────────────────────────────────────────
    fn conf_tx(&mut self, ac: Ac, params: TxQueueParams) -> Result<(), i32>;

    // ── Timestamps ────────────────────────────────────────────────────────────
    fn get_tsf(&self) -> u64;
    fn set_tsf(&mut self, tsf: u64);
    fn reset_tsf(&mut self);
}

// ── mac80211 state machine ────────────────────────────────────────────────────

/// Connection state (STA mode).  Mirrors `enum ieee80211_sta_state` and
/// the internal managed-device FSM in net/mac80211/mlme.c.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaConnState {
    Disconnected,
    Scanning,
    Authenticating,
    Associating,
    Associated,
    DisconnectRequested,
}

/// mac80211 core — owns the driver and manages the connection FSM.
pub struct Mac80211<D: Ieee80211Ops> {
    pub hw:    Ieee80211Hw,
    driver:    D,
    state:     StaConnState,
    current_bss: Option<Bss>,
    own_addr:  MacAddr,
    /// IPC port to notify on association events (nl80211 userspace daemon).
    nl80211_port: Option<ipc::Port>,
}

impl<D: Ieee80211Ops> Mac80211<D> {
    pub fn new(hw: Ieee80211Hw, driver: D, addr: MacAddr) -> Self {
        Self {
            hw, driver,
            state: StaConnState::Disconnected,
            current_bss: None,
            own_addr: addr,
            nl80211_port: None,
        }
    }

    /// Register the IPC port that the nl80211 userspace daemon listens on.
    pub fn set_nl80211_port(&mut self, port: ipc::Port) {
        self.nl80211_port = Some(port);
    }

    pub fn state(&self) -> StaConnState { self.state }
    pub fn current_bss(&self) -> Option<&Bss> { self.current_bss.as_ref() }

    /// Start the hardware and bring the radio up.
    pub fn bring_up(&mut self) -> Result<(), i32> {
        self.driver.start()?;
        self.driver.add_interface(IfType::Station)?;
        Ok(())
    }

    /// Trigger a scan.  Results are delivered asynchronously via rx_mgmt().
    pub fn scan(&mut self, req: ScanRequest) -> Result<(), i32> {
        if self.state != StaConnState::Disconnected { return Err(-16); } // EBUSY
        self.state = StaConnState::Scanning;
        self.driver.hw_scan(&req)
    }

    /// Called by the driver when a management frame is received.
    pub fn rx_mgmt(&mut self, frame: &[u8]) {
        use crate::ieee80211::{fc, is_mgmt, frame_subtype};
        if frame.len() < 2 { return; }
        let fc_val = u16::from_le_bytes([frame[0], frame[1]]);
        if !is_mgmt(fc_val) { return; }

        match frame_subtype(fc_val) {
            s if s == fc::FTYPE_MGMT | fc::STYPE_BEACON
              || s == fc::FTYPE_MGMT | fc::STYPE_PROBE_RESP => {
                scan::handle_beacon_or_probe(self, frame);
            }
            s if s == fc::FTYPE_MGMT | fc::STYPE_AUTH => {
                auth::handle_auth(self, frame);
            }
            s if s == fc::FTYPE_MGMT | fc::STYPE_ASSOC_RESP => {
                auth::handle_assoc_resp(self, frame);
            }
            s if s == fc::FTYPE_MGMT | fc::STYPE_DEAUTH
              || s == fc::FTYPE_MGMT | fc::STYPE_DISASSOC => {
                self.state = StaConnState::Disconnected;
                self.current_bss = None;
            }
            _ => {}
        }
    }

    /// Notify driver that a scan is complete.
    pub fn scan_done(&mut self) {
        if self.state == StaConnState::Scanning {
            self.state = StaConnState::Disconnected;
        }
    }

    /// Initiate connection to a BSS.
    pub fn connect(&mut self, bss: Bss, _params: ConnectParams) -> Result<(), i32> {
        self.driver.config(bss.channel)?;
        self.current_bss = Some(bss);
        self.state = StaConnState::Authenticating;
        // Send Authentication frame (Open System, seq 1).
        let auth_frame = frame::build_auth_open(&self.own_addr, &bss.bssid, 1);
        self.driver.tx(auth_frame);
        Ok(())
    }

    /// Disconnect from the current BSS.
    pub fn disconnect(&mut self, reason: u16) -> Result<(), i32> {
        if let Some(bss) = self.current_bss.take() {
            let deauth = frame::build_deauth(&self.own_addr, &bss.bssid, reason);
            self.driver.tx(deauth);
        }
        self.state = StaConnState::Disconnected;
        Ok(())
    }
}
