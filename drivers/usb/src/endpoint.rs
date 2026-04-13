//! USB endpoint model — mirrors struct usb_host_endpoint in include/linux/usb.h

use crate::descriptor::{EndpointDescriptor, SsEndpointCompDescriptor,
                        USB_ENDPOINT_XFERTYPE_MASK, USB_ENDPOINT_XFER_CONTROL,
                        USB_ENDPOINT_XFER_ISOC, USB_ENDPOINT_XFER_BULK,
                        USB_ENDPOINT_XFER_INT, USB_DIR_IN};

/// Transfer type — maps directly to bmAttributes bits 1:0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferType {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

impl TransferType {
    pub fn from_bm(bm: u8) -> Self {
        match bm & USB_ENDPOINT_XFERTYPE_MASK {
            USB_ENDPOINT_XFER_CONTROL => Self::Control,
            USB_ENDPOINT_XFER_ISOC    => Self::Isochronous,
            USB_ENDPOINT_XFER_BULK    => Self::Bulk,
            USB_ENDPOINT_XFER_INT     => Self::Interrupt,
            _ => unreachable!(),
        }
    }
}

/// Endpoint data direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction { In, Out }

/// Runtime representation of a single USB endpoint.
///
/// Mirrors `struct usb_host_endpoint` — descriptor plus operational state.
pub struct Endpoint {
    pub desc:       EndpointDescriptor,
    pub ss_ep_comp: Option<SsEndpointCompDescriptor>,
    pub xfer_type:  TransferType,
    pub direction:  Direction,
    /// Toggle bit for non-isochronous endpoints (DATA0 / DATA1).
    pub toggle:     bool,
    /// Bandwidth reservation in bytes per (micro)frame.
    pub bandwidth:  u32,
}

impl Endpoint {
    pub fn new(desc: EndpointDescriptor, ss: Option<SsEndpointCompDescriptor>) -> Self {
        let xfer_type = TransferType::from_bm(desc.bm_attributes);
        let direction = if desc.b_endpoint_address & USB_DIR_IN != 0 {
            Direction::In
        } else {
            Direction::Out
        };
        Self {
            desc,
            ss_ep_comp: ss,
            xfer_type,
            direction,
            toggle: false,
            bandwidth: 0,
        }
    }

    pub fn number(&self) -> u8 { self.desc.number() }
    pub fn max_packet(&self) -> u16 { self.desc.max_packet() }

    /// Pipe encoding used by Linux's usb_fill_*_urb helpers:
    ///   bits 30:29 = transfer type, bit 7 = direction IN, bits 14:8 = devnum,
    ///   bits 3:0   = endpoint number.
    pub fn encode_pipe(&self, devnum: u8) -> u32 {
        let tt = match self.xfer_type {
            TransferType::Control     => 0u32,
            TransferType::Isochronous => 1,
            TransferType::Bulk        => 2,
            TransferType::Interrupt   => 3,
        };
        let dir = if self.direction == Direction::In { 0x80u32 } else { 0 };
        (tt << 29) | ((devnum as u32) << 8) | dir | self.number() as u32
    }
}
