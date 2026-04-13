//! USB device model — mirrors struct usb_device in include/linux/usb.h

use crate::descriptor::{DeviceDescriptor, ConfigDescriptor, UsbSpeed};
use crate::endpoint::Endpoint;

pub const USB_MAXCHILDREN: usize = 31;
pub const USB_MAX_EP:       usize = 32; // 16 ep numbers × 2 directions

/// USB device state machine — mirrors enum usb_device_state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Attached,
    Powered,
    Reconnecting,
    Unauthenticated,
    Default,    // After reset, before SET_ADDRESS
    Address,    // Address assigned, not yet configured
    Configured,
    Suspended,
}

/// Authorisation state (for USB authorisation framework).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthState { Unauthorized, Authorized }

/// A USB device — the central object of the USB driver model.
///
/// Mirrors Linux's `struct usb_device` but adapted for our microkernel:
/// no kernel memory management, no sysfs, configuration limited to 1.
pub struct UsbDevice {
    // ── Topology ──────────────────────────────────────────────────────────────
    /// USB device address assigned by hub driver (1–127; 0 = unconfigured).
    pub devnum:     u8,
    /// Depth in the USB device tree (root hub = 0).
    pub level:      u8,
    /// Port on the parent hub (0-based).
    pub port:       u8,
    pub speed:      UsbSpeed,
    pub state:      DeviceState,
    pub auth_state: AuthState,

    // ── Descriptors ───────────────────────────────────────────────────────────
    pub device_desc:   DeviceDescriptor,
    pub active_config: Option<ConfigDescriptor>,

    // ── Endpoints (active configuration) ─────────────────────────────────────
    /// Endpoint 0 is always control; slots 1..31 are filled from the active
    /// configuration's interfaces.
    pub ep: [Option<Endpoint>; USB_MAX_EP],

    // ── Power ─────────────────────────────────────────────────────────────────
    /// Current drawn from VBUS in mA.
    pub bus_ma: u16,

    // ── Strings (indices into descriptor strings table) ───────────────────────
    pub manufacturer: Option<u8>,
    pub product:      Option<u8>,
    pub serial:       Option<u8>,

    // ── Bus topology ─────────────────────────────────────────────────────────
    /// True if this device is a hub.
    pub is_hub: bool,
    /// For hubs: number of downstream ports.
    pub maxchild: u8,
}

impl UsbDevice {
    /// Create a device in Default state with only EP0.
    pub fn new(devnum: u8, speed: UsbSpeed) -> Self {
        Self {
            devnum,
            level: 0,
            port: 0,
            speed,
            state: DeviceState::Default,
            auth_state: AuthState::Unauthorized,
            device_desc: DeviceDescriptor::default(),
            active_config: None,
            ep: core::array::from_fn(|_| None),
            bus_ma: 0,
            manufacturer: None,
            product: None,
            serial: None,
            is_hub: false,
            maxchild: 0,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.state == DeviceState::Configured
    }

    pub fn vendor_id(&self)  -> u16 { self.device_desc.id_vendor.get() }
    pub fn product_id(&self) -> u16 { self.device_desc.id_product.get() }

    /// Look up an endpoint by number + direction (IN flag = bit 7).
    pub fn get_ep(&self, address: u8) -> Option<&Endpoint> {
        let dir = (address >> 7) as usize;
        let num = (address & 0x0F) as usize;
        let idx = dir * 16 + num;
        self.ep.get(idx)?.as_ref()
    }
}
