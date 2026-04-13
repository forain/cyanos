//! 802.11 MAC frame types — ported from include/linux/ieee80211.h
//!
//! All multi-byte frame fields are little-endian per the 802.11 spec.

// ── Ethernet (MAC address) ────────────────────────────────────────────────────

pub const ETH_ALEN: usize = 6;
pub type MacAddr = [u8; ETH_ALEN];

pub const BROADCAST_ADDR: MacAddr = [0xFF; ETH_ALEN];

// ── Frame Control field (2 bytes, LE) ─────────────────────────────────────────

/// Frame control field bit masks.
pub mod fc {
    pub const PROTO_VERSION_MASK: u16 = 0x0003;
    pub const TYPE_MASK:          u16 = 0x000C;
    pub const SUBTYPE_MASK:       u16 = 0x00F0;
    pub const TO_DS:              u16 = 0x0100;
    pub const FROM_DS:            u16 = 0x0200;
    pub const MORE_FRAGS:         u16 = 0x0400;
    pub const RETRY:              u16 = 0x0800;
    pub const PWR_MGT:            u16 = 0x1000;
    pub const MORE_DATA:          u16 = 0x2000;
    pub const PROTECTED:          u16 = 0x4000;
    pub const ORDER:              u16 = 0x8000;

    // Frame types
    pub const FTYPE_MGMT: u16 = 0x0000;
    pub const FTYPE_CTL:  u16 = 0x0004;
    pub const FTYPE_DATA: u16 = 0x0008;
    pub const FTYPE_EXT:  u16 = 0x000C;

    // Management subtypes
    pub const STYPE_ASSOC_REQ:   u16 = 0x0000;
    pub const STYPE_ASSOC_RESP:  u16 = 0x0010;
    pub const STYPE_REASSOC_REQ: u16 = 0x0020;
    pub const STYPE_REASSOC_RESP: u16 = 0x0030;
    pub const STYPE_PROBE_REQ:   u16 = 0x0040;
    pub const STYPE_PROBE_RESP:  u16 = 0x0050;
    pub const STYPE_BEACON:      u16 = 0x0080;
    pub const STYPE_ATIM:        u16 = 0x0090;
    pub const STYPE_DISASSOC:    u16 = 0x00A0;
    pub const STYPE_AUTH:        u16 = 0x00B0;
    pub const STYPE_DEAUTH:      u16 = 0x00C0;
    pub const STYPE_ACTION:      u16 = 0x00D0;
    pub const STYPE_ACTION_NO_ACK: u16 = 0x00E0;

    // Control subtypes
    pub const STYPE_BACK_REQ: u16 = 0x0080;
    pub const STYPE_BACK:     u16 = 0x0090;
    pub const STYPE_PSPOLL:   u16 = 0x00A0;
    pub const STYPE_RTS:      u16 = 0x00B0;
    pub const STYPE_CTS:      u16 = 0x00C0;
    pub const STYPE_ACK:      u16 = 0x00D0;
    pub const STYPE_CFEND:    u16 = 0x00E0;

    // Data subtypes
    pub const STYPE_DATA:         u16 = 0x0000;
    pub const STYPE_DATA_CFACK:   u16 = 0x0010;
    pub const STYPE_DATA_CFPOLL:  u16 = 0x0020;
    pub const STYPE_NULLFUNC:     u16 = 0x0040;
    pub const STYPE_QOS_DATA:     u16 = 0x0080;
    pub const STYPE_QOS_NULLFUNC: u16 = 0x00C0;
}

/// Decode frame type from frame_control.
pub fn frame_type(fc: u16) -> u16 { fc & fc::TYPE_MASK }
/// Decode frame subtype.
pub fn frame_subtype(fc: u16) -> u16 { fc & (fc::TYPE_MASK | fc::SUBTYPE_MASK) }
/// True if this is a management frame.
pub fn is_mgmt(fc: u16) -> bool { frame_type(fc) == fc::FTYPE_MGMT }
/// True if this is a data frame.
pub fn is_data(fc: u16) -> bool { frame_type(fc) == fc::FTYPE_DATA }
/// True if frame is QoS data.
pub fn is_qos_data(fc: u16) -> bool { fc & (fc::TYPE_MASK | 0x0080) == (fc::FTYPE_DATA | 0x0080) }

// ── Frame headers ─────────────────────────────────────────────────────────────

/// Standard 3-address 802.11 header (24 bytes).
/// Mirrors `struct ieee80211_hdr_3addr`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct Hdr3addr {
    pub frame_control: u16, // LE
    pub duration_id:   u16, // LE
    pub addr1: MacAddr,     // RA (receiver address)
    pub addr2: MacAddr,     // TA (transmitter address)
    pub addr3: MacAddr,     // BSS ID / DA / SA
    pub seq_ctrl: u16,      // sequence + fragment number (LE)
}

/// QoS 3-address header (26 bytes).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct QosHdr {
    pub frame_control: u16,
    pub duration_id:   u16,
    pub addr1: MacAddr,
    pub addr2: MacAddr,
    pub addr3: MacAddr,
    pub seq_ctrl: u16,
    pub qos_ctrl: u16, // LE
}

/// Sequence control field helpers.
pub fn seq_number(seq_ctrl: u16) -> u16 { seq_ctrl >> 4 }
pub fn frag_number(seq_ctrl: u16) -> u8 { (seq_ctrl & 0xF) as u8 }

/// QoS control field helpers.
pub fn qos_tid(qos: u16) -> u8 { (qos & 0xF) as u8 }
pub fn qos_eosp(qos: u16) -> bool { qos & 0x0010 != 0 }
pub fn qos_ack_policy(qos: u16) -> u8 { ((qos >> 5) & 0x3) as u8 }
pub fn qos_amsdu(qos: u16) -> bool { qos & 0x0080 != 0 }

// ── Capability information field (§9.4.1.4) ───────────────────────────────────

pub mod capab {
    pub const ESS:             u16 = 1 << 0;
    pub const IBSS:            u16 = 1 << 1;
    pub const CF_POLLABLE:     u16 = 1 << 2;
    pub const CF_POLL_REQUEST: u16 = 1 << 3;
    pub const PRIVACY:         u16 = 1 << 4;
    pub const SHORT_PREAMBLE:  u16 = 1 << 5;
    pub const PBCC:            u16 = 1 << 6;
    pub const CHANNEL_AGILITY: u16 = 1 << 7;
    pub const SPECTRUM_MGMT:   u16 = 1 << 8;
    pub const QOS:             u16 = 1 << 9;
    pub const SHORT_SLOT_TIME: u16 = 1 << 10;
    pub const APSD:            u16 = 1 << 11;
    pub const RADIO_MEASURE:   u16 = 1 << 12;
    pub const DSSS_OFDM:       u16 = 1 << 13;
    pub const DEL_BACK:        u16 = 1 << 14;
    pub const IMM_BACK:        u16 = 1 << 15;
}

// ── Status / reason codes ─────────────────────────────────────────────────────

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCode {
    Success                = 0,
    UnspecifiedFailure     = 1,
    CapsUnsupported        = 10,
    AssocDeniedUnspec      = 12,
    NotSupportedAuthAlg    = 13,
    UnknownAuthTransaction = 14,
    AuthTimeout            = 16,
    ApUnableToHandleNewSta = 17,
    AssocDeniedRates       = 18,
    InvalidIe              = 40,
    InvalidAkmp            = 43,
    UnsuppRsnVersion       = 44,
    InvalidRsnIeCap        = 45,
    CipherSuiteRejected    = 46,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasonCode {
    Unspecified             = 1,
    PrevAuthNotValid        = 2,
    DeauthLeaving           = 3,
    DisassocDueToInactivity = 4,
    DisassocApBusy          = 5,
    Class2FrameFromNonauth  = 6,
    Class3FrameFromNonassoc = 7,
    DisassocStaHasLeft      = 8,
    StaReqAssocWithoutAuth  = 9,
    MicFailure              = 14,
    FourwayHandshakeTimeout = 15,
    GroupKeyUpdateTimeout   = 16,
    IeIn4wayDiffers         = 17,
    InvalidGroupCipher      = 18,
    InvalidPairwiseCipher   = 19,
}

// ── Information elements ──────────────────────────────────────────────────────

/// IE element IDs (§9.4.2).
pub mod eid {
    pub const SSID:            u8 = 0;
    pub const SUPPORTED_RATES: u8 = 1;
    pub const DS_PARAMS:       u8 = 3;
    pub const CF_PARAMS:       u8 = 4;
    pub const TIM:             u8 = 5;
    pub const IBSS_PARAMS:     u8 = 6;
    pub const COUNTRY:         u8 = 7;
    pub const CHALLENGE:       u8 = 16;
    pub const PWR_CONSTRAINT:  u8 = 32;
    pub const PWR_CAPABILITY:  u8 = 33;
    pub const TPC_REQUEST:     u8 = 34;
    pub const TPC_REPORT:      u8 = 35;
    pub const SUPPORTED_CHANS: u8 = 36;
    pub const CHAN_SWITCH_ANN:  u8 = 37;
    pub const MEASURE_REQUEST:  u8 = 38;
    pub const MEASURE_REPORT:   u8 = 39;
    pub const QUIET:            u8 = 40;
    pub const IBSS_DFS:         u8 = 41;
    pub const ERP_INFO:         u8 = 42;
    pub const HT_CAPABILITY:    u8 = 45;
    pub const RSN:              u8 = 48;
    pub const EXT_SUPP_RATES:   u8 = 50;
    pub const HT_OPERATION:     u8 = 61;
    pub const VHT_CAPABILITY:   u8 = 191;
    pub const VHT_OPERATION:    u8 = 192;
    pub const VENDOR_SPECIFIC:  u8 = 221;
    pub const EXTENSION:        u8 = 255; // 802.11ax+
}

/// Parse a single IE from a raw byte slice.
/// Returns (element_id, payload) or None if truncated.
pub fn parse_ie(buf: &[u8]) -> Option<(u8, &[u8])> {
    if buf.len() < 2 { return None; }
    let id  = buf[0];
    let len = buf[1] as usize;
    if buf.len() < 2 + len { return None; }
    Some((id, &buf[2..2 + len]))
}

/// Iterator over information elements in a beacon/probe response body.
pub struct IeIter<'a> {
    buf: &'a [u8],
}

impl<'a> IeIter<'a> {
    pub fn new(buf: &'a [u8]) -> Self { Self { buf } }
}

impl<'a> Iterator for IeIter<'a> {
    type Item = (u8, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        let (id, payload) = parse_ie(self.buf)?;
        self.buf = &self.buf[2 + payload.len()..];
        Some((id, payload))
    }
}

// ── Channel / band ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Ghz2,   // 2.4 GHz
    Ghz5,   // 5 GHz
    Ghz6,   // 6 GHz (Wi-Fi 6E)
    Ghz60,  // 60 GHz (WiGig)
}

/// A single 802.11 channel.  Mirrors `struct ieee80211_channel`.
#[derive(Clone, Copy, Debug)]
pub struct Channel {
    pub band:      Band,
    pub center_freq: u32, // MHz
    pub hw_value:  u16,   // driver-specific HW channel index
    pub flags:     ChannelFlags,
    pub max_power: i8,    // dBm
    pub max_reg_power: i8,
}

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct ChannelFlags: u32 {
        const DISABLED        = 1 << 0;
        const NO_IR           = 1 << 1;  // no initiating radiation (passive only)
        const RADAR           = 1 << 3;  // DFS required
        const NO_HT40PLUS     = 1 << 6;
        const NO_HT40MINUS    = 1 << 7;
        const NO_OFDM         = 1 << 8;
        const NO_80MHZ        = 1 << 9;
        const NO_160MHZ       = 1 << 10;
        const INDOOR_ONLY     = 1 << 11;
        const IR_CONCURRENT   = 1 << 12;
        const NO_20MHZ        = 1 << 13;
        const NO_EHT          = 1 << 14;
    }
}

/// Convert 2.4 GHz channel number (1–14) to MHz.
pub fn chan_to_freq_2ghz(channel: u8) -> u32 {
    if channel == 14 { 2484 } else { 2407 + channel as u32 * 5 }
}

/// Convert 5 GHz channel number to MHz.
pub fn chan_to_freq_5ghz(channel: u8) -> u32 {
    5000 + channel as u32 * 5
}
