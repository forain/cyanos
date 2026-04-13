//! xHCI Device/Slot/Endpoint contexts — from drivers/usb/host/xhci.h
//!
//! The xHCI uses a two-level context structure:
//!   DeviceContext  (output) — 32 bytes slot + 32 bytes × 31 endpoints = 1024 bytes
//!   InputContext   (input)  — 32 bytes control + DeviceContext (for commands)

// ── Slot context ──────────────────────────────────────────────────────────────

/// Slot context — 32 bytes.  Mirrors `struct xhci_slot_ctx`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, align(32))]
pub struct SlotContext {
    /// DW0: Route string [19:0] | Speed [23:20] | MTT [25] | Hub [26] | Last ctx [31:27]
    pub dev_info: u32,
    /// DW1: Max exit latency [15:0] | Root hub port [23:16] | Num ports [31:24]
    pub dev_info2: u32,
    /// DW2: Interrupter target [31:22] | TT hub slot ID [15:8] | TT port [7:0]
    pub tt_info: u32,
    /// DW3: Device address [7:0] | Slot state [26:23]
    pub dev_state: u32,
    _rsvd: [u32; 4],
}

impl SlotContext {
    // dev_info bit fields (DW0)
    pub fn route_string(&self) -> u32 { self.dev_info & 0x000F_FFFF }
    pub fn speed(&self) -> u8 { ((self.dev_info >> 20) & 0xF) as u8 }
    pub fn is_mtt(&self) -> bool { self.dev_info & (1 << 25) != 0 }
    pub fn is_hub(&self) -> bool { self.dev_info & (1 << 26) != 0 }
    pub fn last_ctx(&self) -> u8 { ((self.dev_info >> 27) & 0x1F) as u8 }

    // dev_state (DW3)
    pub fn device_address(&self) -> u8 { (self.dev_state & 0xFF) as u8 }
    pub fn slot_state(&self) -> u8 { ((self.dev_state >> 23) & 0xF) as u8 }

    /// Build DW0 from components.
    pub fn build_dev_info(route: u32, speed: u8, hub: bool, last_ctx: u8) -> u32 {
        (route & 0xF_FFFF)
        | ((speed as u32 & 0xF) << 20)
        | ((hub as u32) << 26)
        | ((last_ctx as u32 & 0x1F) << 27)
    }
}

// Slot state values
pub const SLOT_STATE_DISABLED:    u8 = 0;
pub const SLOT_STATE_DEFAULT:     u8 = 1;
pub const SLOT_STATE_ADDRESSED:   u8 = 2;
pub const SLOT_STATE_CONFIGURED:  u8 = 3;

// Speed values (match xHCI §6.2.2.1 Table 26)
pub const SLOT_SPEED_FS: u8 = 1; // Full-speed
pub const SLOT_SPEED_LS: u8 = 2; // Low-speed
pub const SLOT_SPEED_HS: u8 = 3; // High-speed
pub const SLOT_SPEED_SS: u8 = 4; // SuperSpeed Gen1×1
pub const SLOT_SPEED_SS_GEN2X1: u8 = 5;

// ── Endpoint context ──────────────────────────────────────────────────────────

/// Endpoint context — 32 bytes.  Mirrors `struct xhci_ep_ctx`.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, align(32))]
pub struct EndpointContext {
    /// DW0: EP state [2:0] | Mult [9:8] | MaxPStreams [14:10] | LSA [15] | Interval [23:16]
    pub ep_info: u32,
    /// DW1: CErr [2:1] | EP type [5:3] | HID [7] | Max burst [15:8] | Max pkt size [31:16]
    pub ep_info2: u32,
    /// DW2-DW3: Transfer ring dequeue pointer (64-bit, bit 0 = DCS)
    pub deq_lo: u32,
    pub deq_hi: u32,
    /// DW4: Average TRB length [15:0] | Max ESIT payload lo [31:16]
    pub tx_info: u32,
    _rsvd: [u32; 3],
}

impl EndpointContext {
    // ep_info (DW0)
    pub fn ep_state(&self) -> u8 { (self.ep_info & 0x7) as u8 }
    pub fn mult(&self) -> u8 { ((self.ep_info >> 8) & 0x3) as u8 }
    pub fn max_p_streams(&self) -> u8 { ((self.ep_info >> 10) & 0x1F) as u8 }
    pub fn interval(&self) -> u8 { ((self.ep_info >> 16) & 0xFF) as u8 }

    // ep_info2 (DW1)
    pub fn err_count(&self) -> u8 { ((self.ep_info2 >> 1) & 0x3) as u8 }
    pub fn ep_type(&self) -> u8 { ((self.ep_info2 >> 3) & 0x7) as u8 }
    pub fn max_burst(&self) -> u8 { ((self.ep_info2 >> 8) & 0xFF) as u8 }
    pub fn max_packet_size(&self) -> u16 { (self.ep_info2 >> 16) as u16 }

    pub fn transfer_ring_deq(&self) -> u64 {
        (self.deq_lo as u64) | ((self.deq_hi as u64) << 32)
    }

    /// Build DW1 for a given endpoint type, error count, burst, and max packet size.
    pub fn build_ep_info2(ep_type: u8, cerr: u8, max_burst: u8, max_packet: u16) -> u32 {
        ((cerr as u32 & 0x3) << 1)
        | ((ep_type as u32 & 0x7) << 3)
        | ((max_burst as u32) << 8)
        | ((max_packet as u32) << 16)
    }
}

// Endpoint state values
pub const EP_STATE_DISABLED: u8 = 0;
pub const EP_STATE_RUNNING:  u8 = 1;
pub const EP_STATE_HALTED:   u8 = 2;
pub const EP_STATE_STOPPED:  u8 = 3;
pub const EP_STATE_ERROR:    u8 = 4;

// xHCI endpoint type codes (ep_info2 bits 5:3)
pub const EP_TYPE_ISOCH_OUT:  u8 = 1;
pub const EP_TYPE_BULK_OUT:   u8 = 2;
pub const EP_TYPE_INTR_OUT:   u8 = 3;
pub const EP_TYPE_CTRL:       u8 = 4;
pub const EP_TYPE_ISOCH_IN:   u8 = 5;
pub const EP_TYPE_BULK_IN:    u8 = 6;
pub const EP_TYPE_INTR_IN:    u8 = 7;

// ── Device context (output) ───────────────────────────────────────────────────

/// Output device context — slot context + 31 endpoint contexts = 1024 bytes.
/// The HC writes this; the driver reads it.
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
pub struct DeviceContext {
    pub slot: SlotContext,
    pub ep:   [EndpointContext; 31],
}

// ── Input context (command input) ────────────────────────────────────────────

/// Input control context — 32 bytes.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C, align(32))]
pub struct InputControlContext {
    /// Drop flags — bit n = drop endpoint context n.
    pub drop_flags: u32,
    /// Add flags — bit n = add endpoint context n; bit 0 = slot context.
    pub add_flags:  u32,
    _rsvd: [u32; 6],
}

/// Full input context: control + device context.
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
pub struct InputContext {
    pub ctrl:   InputControlContext,
    pub device: DeviceContext,
}

impl InputContext {
    /// Configure for ADDRESS_DEVICE: add slot (bit 0) + EP0 (bit 1).
    pub fn for_address_device(slot: SlotContext, ep0: EndpointContext) -> Self {
        let mut ic = Self::default();
        ic.ctrl.add_flags = 0b11; // slot + EP0
        ic.device.slot = slot;
        ic.device.ep[0] = ep0;
        ic
    }

    /// Configure for CONFIGURE_ENDPOINT: add/drop the specified endpoints.
    pub fn for_configure_ep(
        drop_mask: u32, add_mask: u32,
        slot: SlotContext, eps: &[(usize, EndpointContext)],
    ) -> Self {
        let mut ic = Self::default();
        ic.ctrl.drop_flags = drop_mask;
        ic.ctrl.add_flags  = add_mask;
        ic.device.slot = slot;
        for &(idx, ctx) in eps {
            if idx < 31 { ic.device.ep[idx] = ctx; }
        }
        ic
    }
}
