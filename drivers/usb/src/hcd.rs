//! Host Controller Driver (HCD) trait — mirrors linux/usb/hcd.h struct usb_hcd
//!
//! Each HCI (xHCI, EHCI, OHCI) implements this trait, providing the hardware
//! abstraction layer that the USB core (hub driver, class drivers) talks to.

use crate::descriptor::{DeviceDescriptor, ConfigDescriptor};
use crate::device::UsbDevice;
use crate::transfer::Urb;

/// HCD hardware state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HcdState {
    Halt,
    Running,
    Suspended,
    Error,
}

/// Error type for HCD operations.
#[derive(Clone, Copy, Debug)]
pub enum HcdError {
    /// Device disconnected.
    NoDevice,
    /// Endpoint STALL.
    Stall,
    /// Transfer timed out.
    Timeout,
    /// DMA or hardware error.
    HardwareError,
    /// Bad parameter.
    Invalid,
}

/// The Host Controller Driver trait — implemented by xhci, ehci, etc.
pub trait HostControllerDriver {
    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Initialise hardware and bring HC to running state.
    fn start(&mut self) -> Result<(), HcdError>;

    /// Stop the host controller cleanly.
    fn stop(&mut self);

    fn state(&self) -> HcdState;

    // ── Transfer submission ───────────────────────────────────────────────────

    /// Submit an Urb to the HC for async processing.
    /// The HC signals completion via `urb.completion_port`.
    fn submit_urb(&mut self, urb: Urb) -> Result<(), HcdError>;

    /// Cancel a previously submitted Urb.
    fn kill_urb(&mut self, urb_context: u64);

    // ── Enumeration helpers (used by hub driver) ──────────────────────────────

    /// Read `DeviceDescriptor` from a device at its current address.
    fn get_device_descriptor(&mut self, dev: &UsbDevice)
        -> Option<DeviceDescriptor>;

    /// Read `ConfigDescriptor` for configuration index `cfg_idx`.
    fn get_config_descriptor(&mut self, dev: &UsbDevice, cfg_idx: u8)
        -> Option<ConfigDescriptor>;

    /// Issue SET_ADDRESS request.
    fn set_address(&mut self, dev: &mut UsbDevice, address: u8);

    /// Issue SET_CONFIGURATION request.
    fn set_configuration(&mut self, dev: &mut UsbDevice, config_value: u8);

    // ── Hub port management ───────────────────────────────────────────────────

    /// Return (wPortStatus, wPortChange) for a root-hub port (1-indexed).
    fn get_port_status(&mut self, port: u8) -> Option<(u16, u16)>;

    /// Issue a port reset pulse.
    fn port_reset(&mut self, port: u8);

    /// SetPortFeature class request.
    fn set_port_feature(&mut self, port: u8, feature: u16);

    /// ClearPortFeature class request.
    fn clear_port_feature(&mut self, port: u8, feature: u16);

    // ── Bandwidth management ──────────────────────────────────────────────────

    /// Reserve periodic bandwidth for an endpoint.
    /// Returns allocated microframe budget in bytes or error.
    fn alloc_bandwidth(&mut self, dev: &UsbDevice, ep_address: u8, bytes_per_frame: u32)
        -> Result<u32, HcdError>;

    fn free_bandwidth(&mut self, dev: &UsbDevice, ep_address: u8);
}
