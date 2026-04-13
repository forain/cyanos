//! USB transfer (URB) — ported from include/linux/usb.h struct urb
//!
//! An Urb (USB Request Block) is the async unit of work submitted to the HCD.
//! Linux's urb lives in the kernel with DMA mapping; here we use physical
//! addresses directly and a simple callback tag (IPC port) instead of a
//! function pointer.

use ipc::Port;

// ── Transfer flags ────────────────────────────────────────────────────────────

bitflags::bitflags! {
    /// URB transfer_flags — mirrors Linux URB_* constants.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct TransferFlags: u32 {
        /// Return -EREMOTEIO on short read (don't accept short transfers).
        const SHORT_NOT_OK        = 0x0001;
        /// ISO: start transfer as soon as possible.
        const ISO_ASAP            = 0x0002;
        /// Buffer is already DMA-mapped; skip mapping.
        const NO_TRANSFER_DMA_MAP = 0x0004;
        /// Append a zero-length packet to terminate bulk OUT.
        const ZERO_PACKET         = 0x0040;
        /// Don't generate a completion interrupt (batch submission).
        const NO_INTERRUPT        = 0x0080;
        /// Free transfer_buffer on completion.
        const FREE_BUFFER         = 0x0100;
        /// Direction: IN from device (informational for some HCDs).
        const DIR_IN              = 0x0200;
    }
}

// ── Transfer result ───────────────────────────────────────────────────────────

/// `urb->status` equivalents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrbStatus {
    /// Transfer completed successfully.
    Ok,
    /// Transfer was cancelled via `usb_kill_urb`.
    Cancelled,
    /// Device disconnected mid-transfer.
    NoDevice,
    /// STALL handshake received.
    Stall,
    /// Babble: device sent too much data.
    Overflow,
    /// CRC or bit-stuffing error.
    BitError,
    /// No response from device (timeout).
    NoResponse,
    /// Buffer overrun / underrun.
    BufferError,
    /// ISO: all packets errored.
    AllIsoErrors,
    /// Unspecified HC error.
    HcError,
}

// ── ISO packet descriptor ─────────────────────────────────────────────────────

/// Per-packet state for isochronous transfers.
/// Mirrors `struct usb_iso_packet_descriptor`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IsoPacketDescriptor {
    /// Byte offset of this packet within `transfer_buffer`.
    pub offset: u32,
    /// Requested packet length.
    pub length: u32,
    /// Actual bytes transferred (filled by HCD on completion).
    pub actual_length: u32,
    /// Per-packet status (filled by HCD).
    pub status: i32,
}

// ── URB ───────────────────────────────────────────────────────────────────────

pub const MAX_ISO_PACKETS: usize = 128;

/// USB Request Block — the fundamental async transfer descriptor.
///
/// Submit via `HostControllerDriver::submit_urb`.
/// The HCD fills `actual_length` / `status` and signals `completion_port`.
pub struct Urb {
    // ── Identifying info ──────────────────────────────────────────────────────
    /// USB device address (1–127).
    pub dev_address: u8,
    /// Encoded pipe: transfer type | direction | devaddr | ep number.
    pub pipe: u32,
    /// Bulk stream ID (0 = no stream).
    pub stream_id: u16,

    // ── Transfer data ─────────────────────────────────────────────────────────
    pub transfer_flags: TransferFlags,
    /// Physical address of the I/O buffer.
    pub transfer_buffer_phys: u64,
    /// Length of `transfer_buffer` in bytes.
    pub transfer_buffer_length: u32,
    /// Actual bytes transferred — written by HCD on completion.
    pub actual_length: u32,

    // ── Control transfers ─────────────────────────────────────────────────────
    /// Physical address of the 8-byte SETUP packet (control only).
    pub setup_packet_phys: u64,

    // ── Isochronous transfers ─────────────────────────────────────────────────
    /// Frame number to start on (ISO only; -1 = ASAP when ISO_ASAP set).
    pub start_frame: i32,
    /// Number of ISO packets in `iso_frame_desc`.
    pub number_of_packets: u16,
    /// Polling interval in (micro)frames.
    pub interval: u8,

    // ── Completion ────────────────────────────────────────────────────────────
    /// IPC port to notify on completion (instead of a callback fn pointer).
    pub completion_port: Port,
    /// Driver-defined context tag echoed back in the completion message.
    pub context: u64,

    // ── Status (filled by HCD) ────────────────────────────────────────────────
    pub status: UrbStatus,

    // ── ISO per-packet descriptors ────────────────────────────────────────────
    pub iso_frame_desc: [IsoPacketDescriptor; MAX_ISO_PACKETS],
}

impl Urb {
    /// Construct a bulk-OUT URB.
    pub fn bulk_out(
        dev_address: u8, ep: u8,
        buf_phys: u64, len: u32,
        completion_port: Port, context: u64,
    ) -> Self {
        // pipe: bulk(2) | OUT | devnum | ep
        let pipe = (2u32 << 29) | ((dev_address as u32) << 8) | ep as u32;
        Self::new(dev_address, pipe, buf_phys, len, completion_port, context)
    }

    /// Construct a bulk-IN URB.
    pub fn bulk_in(
        dev_address: u8, ep: u8,
        buf_phys: u64, len: u32,
        completion_port: Port, context: u64,
    ) -> Self {
        let pipe = (2u32 << 29) | 0x80 | ((dev_address as u32) << 8) | ep as u32;
        let mut urb = Self::new(dev_address, pipe, buf_phys, len, completion_port, context);
        urb.transfer_flags |= TransferFlags::DIR_IN;
        urb
    }

    fn new(
        dev_address: u8, pipe: u32,
        buf_phys: u64, len: u32,
        completion_port: Port, context: u64,
    ) -> Self {
        Self {
            dev_address,
            pipe,
            stream_id: 0,
            transfer_flags: TransferFlags::empty(),
            transfer_buffer_phys: buf_phys,
            transfer_buffer_length: len,
            actual_length: 0,
            setup_packet_phys: 0,
            start_frame: -1,
            number_of_packets: 0,
            interval: 0,
            completion_port,
            context,
            status: UrbStatus::Ok,
            iso_frame_desc: [IsoPacketDescriptor { offset: 0, length: 0,
                                                   actual_length: 0, status: 0 };
                             MAX_ISO_PACKETS],
        }
    }
}
