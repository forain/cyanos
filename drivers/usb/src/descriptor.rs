//! USB Descriptor types — faithfully ported from include/uapi/linux/usb/ch9.h
//!
//! All multi-byte fields are little-endian (LE), matching the USB spec and
//! Linux's __le16 / __le32 types.  We use a newtype Le16 to make this explicit.

use core::fmt;

/// Little-endian u16 as it appears on the wire / in descriptor memory.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct Le16(pub u16);

impl Le16 {
    pub fn get(self) -> u16 { u16::from_le(self.0) }
}

impl fmt::Debug for Le16 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:04x}", self.get())
    }
}

// ── Descriptor type codes ────────────────────────────────────────────────────

pub const DT_DEVICE:        u8 = 0x01;
pub const DT_CONFIG:        u8 = 0x02;
pub const DT_STRING:        u8 = 0x03;
pub const DT_INTERFACE:     u8 = 0x04;
pub const DT_ENDPOINT:      u8 = 0x05;
pub const DT_DEVICE_QUAL:   u8 = 0x06;
pub const DT_OTHER_SPEED:   u8 = 0x07;
pub const DT_INTERFACE_PWR: u8 = 0x08;
pub const DT_HUB:           u8 = 0x29;
pub const DT_SS_HUB:        u8 = 0x2A;
pub const DT_SS_ENDPOINT:   u8 = 0x30;

// ── Device class codes ───────────────────────────────────────────────────────

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceClass {
    PerInterface   = 0x00, // class info in each interface
    Audio          = 0x01,
    Comm           = 0x02,
    Hid            = 0x03,
    Physical       = 0x05,
    StillImage     = 0x06,
    Printer        = 0x07,
    MassStorage    = 0x08,
    Hub            = 0x09,
    CdcData        = 0x0A,
    CscId          = 0x0B,
    ContentSec     = 0x0D,
    Video          = 0x0E,
    WirelessCtrl   = 0xE0,
    Misc           = 0xEF,
    AppSpec        = 0xFE,
    VendorSpec     = 0xFF,
}

// ── USB speeds ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbSpeed {
    Unknown,
    Low,        // USB 1.0  1.5 Mb/s
    Full,       // USB 1.1  12  Mb/s
    High,       // USB 2.0  480 Mb/s
    Wireless,   // Wireless USB (defunct)
    Super,      // USB 3.0  5   Gb/s
    SuperPlus,  // USB 3.1  10  Gb/s
    SuperPlusGen2x2, // USB 3.2  20  Gb/s
}

// ── Device descriptor (18 bytes, USB spec §9.6.1) ────────────────────────────

/// `usb_device_descriptor` — from include/uapi/linux/usb/ch9.h
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct DeviceDescriptor {
    pub b_length:             u8,   // always 18
    pub b_descriptor_type:    u8,   // DT_DEVICE
    pub bcd_usb:              Le16, // e.g. 0x0200 = USB 2.0
    pub b_device_class:       u8,
    pub b_device_sub_class:   u8,
    pub b_device_protocol:    u8,
    pub b_max_packet_size0:   u8,   // EP0 max packet size
    pub id_vendor:            Le16,
    pub id_product:           Le16,
    pub bcd_device:           Le16,
    pub i_manufacturer:       u8,   // string descriptor index
    pub i_product:            u8,
    pub i_serial_number:      u8,
    pub b_num_configurations: u8,
}

impl DeviceDescriptor {
    pub fn usb_version(&self) -> (u8, u8) {
        let v = self.bcd_usb.get();
        ((v >> 8) as u8, (v & 0xFF) as u8)
    }
}

// ── Configuration descriptor (9 bytes, §9.6.3) ──────────────────────────────

pub const USB_CONFIG_ATTR_SELF_POWERED:  u8 = 0x40;
pub const USB_CONFIG_ATTR_REMOTE_WAKEUP: u8 = 0x20;

/// `usb_config_descriptor`
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct ConfigDescriptor {
    pub b_length:               u8,  // always 9
    pub b_descriptor_type:      u8,  // DT_CONFIG
    pub w_total_length:         Le16, // includes all sub-descriptors
    pub b_num_interfaces:       u8,
    pub b_configuration_value:  u8,
    pub i_configuration:        u8,
    pub bm_attributes:          u8,  // bit 7 reserved=1, 6 self-powered, 5 remote wake
    pub b_max_power:            u8,  // in 2 mA units (USB 2) or 8 mA (USB 3)
}

impl ConfigDescriptor {
    pub fn is_self_powered(&self) -> bool {
        self.bm_attributes & USB_CONFIG_ATTR_SELF_POWERED != 0
    }
    pub fn max_power_ma(&self, speed: UsbSpeed) -> u32 {
        let units: u32 = match speed {
            UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::SuperPlusGen2x2 => 8,
            _ => 2,
        };
        self.b_max_power as u32 * units
    }
}

// ── Interface descriptor (9 bytes, §9.6.5) ──────────────────────────────────

/// `usb_interface_descriptor`
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct InterfaceDescriptor {
    pub b_length:              u8,  // always 9
    pub b_descriptor_type:     u8,  // DT_INTERFACE
    pub b_interface_number:    u8,
    pub b_alternate_setting:   u8,
    pub b_num_endpoints:       u8,  // excluding EP0
    pub b_interface_class:     u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol:  u8,
    pub i_interface:           u8,  // string index
}

// ── Endpoint descriptor (7 bytes + optional, §9.6.6) ────────────────────────

/// Endpoint address direction bit.
pub const USB_DIR_IN:  u8 = 0x80;
pub const USB_DIR_OUT: u8 = 0x00;

/// Transfer type bits in bmAttributes.
pub const USB_ENDPOINT_XFERTYPE_MASK: u8 = 0x03;
pub const USB_ENDPOINT_XFER_CONTROL:  u8 = 0x00;
pub const USB_ENDPOINT_XFER_ISOC:     u8 = 0x01;
pub const USB_ENDPOINT_XFER_BULK:     u8 = 0x02;
pub const USB_ENDPOINT_XFER_INT:      u8 = 0x03;

/// Isochronous sync type (bits 3:2 of bmAttributes)
pub const USB_ENDPOINT_SYNCTYPE:      u8 = 0x0C;
pub const USB_ENDPOINT_SYNC_NONE:     u8 = 0x00;
pub const USB_ENDPOINT_SYNC_ASYNC:    u8 = 0x04;
pub const USB_ENDPOINT_SYNC_ADAPTIVE: u8 = 0x08;
pub const USB_ENDPOINT_SYNC_SYNC:     u8 = 0x0C;

/// Isochronous usage type (bits 5:4 of bmAttributes)
pub const USB_ENDPOINT_USAGE_MASK:    u8 = 0x30;
pub const USB_ENDPOINT_USAGE_DATA:    u8 = 0x00;
pub const USB_ENDPOINT_USAGE_FEEDBACK: u8 = 0x10;
pub const USB_ENDPOINT_USAGE_IMPLICIT_FB: u8 = 0x20;

/// `usb_endpoint_descriptor`
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct EndpointDescriptor {
    pub b_length:          u8,   // 7 (standard), 9 (audio)
    pub b_descriptor_type: u8,   // DT_ENDPOINT
    pub b_endpoint_address: u8,  // bit 7 = direction, bits 3:0 = number
    pub bm_attributes:     u8,
    pub w_max_packet_size: Le16,
    pub b_interval:        u8,   // polling interval in (micro)frames
    // Audio class extension (bLength = 9):
    pub b_refresh:         u8,
    pub b_synch_address:   u8,
}

impl EndpointDescriptor {
    pub fn number(&self) -> u8    { self.b_endpoint_address & 0x0F }
    pub fn is_in(&self)  -> bool  { self.b_endpoint_address & USB_DIR_IN != 0 }
    pub fn xfer_type(&self) -> u8 { self.bm_attributes & USB_ENDPOINT_XFERTYPE_MASK }
    pub fn max_packet(&self) -> u16 { self.w_max_packet_size.get() & 0x07FF }
    /// Additional transactions per microframe (high-speed only), 0–2.
    pub fn extra_transactions(&self) -> u8 {
        ((self.w_max_packet_size.get() >> 11) & 0x3) as u8
    }
}

// ── SuperSpeed endpoint companion (§9.6.7) ───────────────────────────────────

/// `usb_ss_ep_comp_descriptor` — appended after each endpoint descriptor for SS
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct SsEndpointCompDescriptor {
    pub b_length:           u8,  // 6
    pub b_descriptor_type:  u8,  // DT_SS_ENDPOINT
    pub b_max_burst:        u8,  // 0–15: max burst size - 1
    pub bm_attributes:      u8,  // bulk: bits 4:0 = max streams; iso: bits 1:0 = mult
    pub w_bytes_per_interval: Le16,
}

// ── String descriptor ────────────────────────────────────────────────────────

/// `usb_string_descriptor` header (variable-length, UTF-16LE payload follows)
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct StringDescriptorHeader {
    pub b_length:          u8,
    pub b_descriptor_type: u8,  // DT_STRING
    // followed by (b_length - 2) / 2 UTF-16LE code units
}

// ── Hub descriptor (ch11.h) ──────────────────────────────────────────────────

/// Hub characteristics — wHubCharacteristics bits.
pub const HUB_CHAR_LPSM:        u16 = 0x0003; // logical power switching mode
pub const HUB_CHAR_COMMON_LPSM: u16 = 0x0000; // ganged
pub const HUB_CHAR_INDV_PORT_LPSM: u16 = 0x0001; // per-port
pub const HUB_CHAR_NO_LPSM:     u16 = 0x0002;
pub const HUB_CHAR_COMPOUND:    u16 = 0x0004;
pub const HUB_CHAR_OCPM:        u16 = 0x0018; // over-current protection
pub const HUB_CHAR_GLOBAL_OCPM: u16 = 0x0000;
pub const HUB_CHAR_INDV_PORT_OCPM: u16 = 0x0008;
pub const HUB_CHAR_NO_OCPM:     u16 = 0x0010;
pub const HUB_CHAR_TTTT:        u16 = 0x0060; // TT think time
pub const HUB_CHAR_PORTIND:     u16 = 0x0080; // port indicators

/// `usb_hub_descriptor` (USB 2.0, variable length)
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct HubDescriptor {
    pub b_desc_length:         u8,
    pub b_descriptor_type:     u8,   // 0x29
    pub b_nbr_ports:           u8,
    pub w_hub_characteristics: Le16,
    pub b_pwr_on_2_pwr_good:   u8,   // in 2 ms units
    pub b_hub_contr_current:   u8,   // in mA
    // DeviceRemovable and PortPwrCtrlMask bitmaps follow (variable)
}

// ── Port status/change bits (ch11.h) ─────────────────────────────────────────

/// wPortStatus bits
pub const PORT_STATUS_CONNECTION:  u16 = 0x0001;
pub const PORT_STATUS_ENABLE:      u16 = 0x0002;
pub const PORT_STATUS_SUSPEND:     u16 = 0x0004;
pub const PORT_STATUS_OVERCURRENT: u16 = 0x0008;
pub const PORT_STATUS_RESET:       u16 = 0x0010;
pub const PORT_STATUS_L1:          u16 = 0x0020;
pub const PORT_STATUS_POWER:       u16 = 0x0100;
pub const PORT_STATUS_LOW_SPEED:   u16 = 0x0200;
pub const PORT_STATUS_HIGH_SPEED:  u16 = 0x0400;
pub const PORT_STATUS_TEST:        u16 = 0x0800;
pub const PORT_STATUS_INDICATOR:   u16 = 0x1000;

/// wPortChange bits
pub const PORT_CHANGE_CONNECTION:  u16 = 0x0001;
pub const PORT_CHANGE_ENABLE:      u16 = 0x0002;
pub const PORT_CHANGE_SUSPEND:     u16 = 0x0004;
pub const PORT_CHANGE_OVERCURRENT: u16 = 0x0008;
pub const PORT_CHANGE_RESET:       u16 = 0x0010;
pub const PORT_CHANGE_L1:          u16 = 0x0020;
pub const PORT_CHANGE_BH_RESET:    u16 = 0x0020;
pub const PORT_CHANGE_LINK_STATE:  u16 = 0x0040;
pub const PORT_CHANGE_CONFIG_ERR:  u16 = 0x0080;

// ── Setup packet (§9.3) ──────────────────────────────────────────────────────

/// `usb_ctrlrequest` — 8-byte SETUP token payload.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, packed)]
pub struct CtrlRequest {
    pub bm_request_type: u8,
    pub b_request:       u8,
    pub w_value:         Le16,
    pub w_index:         Le16,
    pub w_length:        Le16,
}

/// bmRequestType direction bit.
pub const USB_DIR_OUT_BIT: u8 = 0x00;
pub const USB_DIR_IN_BIT:  u8 = 0x80;

/// bmRequestType type bits (bits 6:5).
pub const USB_TYPE_STANDARD: u8 = 0x00;
pub const USB_TYPE_CLASS:    u8 = 0x20;
pub const USB_TYPE_VENDOR:   u8 = 0x40;

/// bmRequestType recipient bits (bits 4:0).
pub const USB_RECIP_DEVICE:    u8 = 0x00;
pub const USB_RECIP_INTERFACE: u8 = 0x01;
pub const USB_RECIP_ENDPOINT:  u8 = 0x02;
pub const USB_RECIP_OTHER:     u8 = 0x03;

/// Standard bRequest codes (§9.4).
pub const USB_REQ_GET_STATUS:        u8 = 0x00;
pub const USB_REQ_CLEAR_FEATURE:     u8 = 0x01;
pub const USB_REQ_SET_FEATURE:       u8 = 0x03;
pub const USB_REQ_SET_ADDRESS:       u8 = 0x05;
pub const USB_REQ_GET_DESCRIPTOR:    u8 = 0x06;
pub const USB_REQ_SET_DESCRIPTOR:    u8 = 0x07;
pub const USB_REQ_GET_CONFIGURATION: u8 = 0x08;
pub const USB_REQ_SET_CONFIGURATION: u8 = 0x09;
pub const USB_REQ_GET_INTERFACE:     u8 = 0x0A;
pub const USB_REQ_SET_INTERFACE:     u8 = 0x0B;
pub const USB_REQ_SYNCH_FRAME:       u8 = 0x0C;
pub const USB_REQ_SET_SEL:           u8 = 0x30;
pub const USB_REQ_SET_ISOCH_DELAY:   u8 = 0x31;
