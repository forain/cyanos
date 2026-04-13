//! WiFi subsystem — ported from Linux net/wireless/ and net/mac80211/
//!
//! Layer map (mirrors Linux):
//!   ieee80211  — 802.11 protocol types (include/linux/ieee80211.h)
//!   cfg80211   — configuration layer (include/net/cfg80211.h)
//!   mac80211   — software MAC (net/mac80211/)
//!   nl80211    — IPC control interface (userspace ↔ kernel, via our IPC ports)

#![no_std]

pub mod cfg80211;
pub mod ieee80211;
pub mod mac80211;
pub mod nl80211;
