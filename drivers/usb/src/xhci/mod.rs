//! xHCI (eXtensible Host Controller Interface) driver
//!
//! Ported from Linux drivers/usb/host/xhci.c and xhci.h.
//! Implements USB 3.x (SuperSpeed) host control with backward compat for USB 2/1.
//!
//! Register layout mirrors the xHCI 1.2 specification §5.

pub mod ring;
pub mod context;
pub mod regs;

use crate::hcd::{HostControllerDriver, HcdError, HcdState};
use crate::descriptor::{DeviceDescriptor, ConfigDescriptor};
use crate::device::UsbDevice;
use crate::transfer::Urb;
use ring::{Ring, RingType, Trb, TrbType};
use context::DeviceContext;
use regs::XhciRegs;

// ── xHCI constants ────────────────────────────────────────────────────────────

pub const XHCI_MAX_SLOTS:    usize = 256;
pub const XHCI_MAX_PORTS:    usize = 127;
pub const XHCI_MAX_INTRS:    usize = 128;
pub const TRBS_PER_SEGMENT:  usize = 256;
pub const TRB_SEGMENT_SIZE:  usize = TRBS_PER_SEGMENT * 16; // 16 bytes per TRB
pub const TRB_MAX_BUFF_SIZE: usize = 65536;
pub const EP_CTX_PER_DEV:    usize = 31;

// ── Slot state ────────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct SlotData {
    device_context: DeviceContext,
    /// Transfer rings, one per endpoint (index = ep context index 1..=30).
    transfer_rings: [Option<Ring>; EP_CTX_PER_DEV + 1],
}

// ── xHCI driver ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub struct Xhci {
    /// MMIO base address of xHCI capability registers.
    mmio_base: usize,
    regs: XhciRegs,
    state: HcdState,

    /// Command ring.
    cmd_ring: Ring,
    /// Event ring (interrupter 0).
    event_ring: Ring,

    /// Slot data indexed by slot ID (1-based; slot 0 unused).
    slots: [Option<SlotData>; XHCI_MAX_SLOTS],

    /// Device Context Base Address Array (DCBAA) physical address.
    /// Points to an array of 64-bit pointers to each slot's output device context.
    dcbaa_phys: u64,

    /// Cycle State Bit for the command ring (alternates each pass).
    cmd_ccs: bool,
}

impl Xhci {
    /// Construct an xHCI driver for an HC at `mmio_base`.
    ///
    /// # Safety
    /// `mmio_base` must point to a valid, mapped xHCI MMIO region.
    pub unsafe fn new(mmio_base: usize) -> Self {
        Self {
            mmio_base,
            regs: XhciRegs::new(mmio_base),
            state: HcdState::Halt,
            cmd_ring:   Ring::new(RingType::Command),
            event_ring: Ring::new(RingType::Event),
            slots: core::array::from_fn(|_| None),
            dcbaa_phys: 0,
            cmd_ccs: true,
        }
    }

    // ── Initialisation sequence (xHCI spec §4.2) ─────────────────────────────

    unsafe fn reset(&mut self) -> Result<(), HcdError> {
        use regs::*;
        // Set CMD_RESET; wait for it to clear (HC sets it back to 0 when done).
        let cmd = self.regs.op_read32(OP_USBCMD);
        self.regs.op_write32(OP_USBCMD, cmd | CMD_RESET);
        // Spin until CNR (Controller Not Ready) clears and RESET clears.
        let mut spins = 0usize;
        loop {
            let sts = self.regs.op_read32(OP_USBSTS);
            let cmd2 = self.regs.op_read32(OP_USBCMD);
            if sts & STS_CNR == 0 && cmd2 & CMD_RESET == 0 { break; }
            spins += 1;
            if spins > 1_000_000 { return Err(HcdError::HardwareError); }
            core::hint::spin_loop();
        }
        Ok(())
    }

    unsafe fn init_rings(&mut self) {
        use regs::*;
        // Write DCBAA pointer.
        self.regs.op_write64(OP_DCBAAP, self.dcbaa_phys);

        // Set up command ring: write CRCR with ring ptr + CCS bit.
        let crcr = self.cmd_ring.phys_base() | CMD_RING_CYCLE as u64;
        self.regs.op_write64(OP_CRCR, crcr);

        // Set up event ring segment table for interrupter 0.
        let ir = &mut self.regs;
        ir.ir_write32(0, IR_ERSTSZ, 1); // 1-entry ERST
        ir.ir_write64(0, IR_ERSTBA, self.event_ring.phys_base());
        ir.ir_write64(0, IR_ERDP, self.event_ring.phys_base());
    }

    // ── Command ring helpers ──────────────────────────────────────────────────

    /// Enqueue a command TRB and ring the command doorbell.
    #[allow(dead_code)]
    unsafe fn send_command(&mut self, trb: Trb) {
        self.cmd_ring.enqueue(trb);
        // Ring HC command doorbell (doorbell 0, target = 0).
        let db_offset = self.regs.db_offset();
        let db_base = self.mmio_base + db_offset as usize;
        (db_base as *mut u32).write_volatile(0);
    }

    /// Issue ENABLE_SLOT and return the allocated slot ID.
    #[allow(dead_code)]
    unsafe fn enable_slot(&mut self) -> Result<u8, HcdError> {
        let trb = Trb::command(TrbType::EnableSlot, 0, 0, 0);
        self.send_command(trb);
        // In real driver: wait for Command Completion Event on event ring.
        // Here we return a placeholder — real impl polls event_ring.
        Ok(1) // TODO: poll event ring for slot ID
    }
}

impl HostControllerDriver for Xhci {
    fn start(&mut self) -> Result<(), HcdError> {
        unsafe {
            self.reset()?;
            self.init_rings();

            use regs::*;
            // Enable interrupts + run.
            let cmd = self.regs.op_read32(OP_USBCMD);
            self.regs.op_write32(OP_USBCMD, cmd | CMD_RUN | CMD_EIE);
            self.state = HcdState::Running;
        }
        Ok(())
    }

    fn stop(&mut self) {
        unsafe {
            use regs::*;
            let cmd = self.regs.op_read32(OP_USBCMD);
            self.regs.op_write32(OP_USBCMD, cmd & !CMD_RUN);
            self.state = HcdState::Halt;
        }
    }

    fn state(&self) -> HcdState { self.state }

    fn submit_urb(&mut self, _urb: Urb) -> Result<(), HcdError> {
        // TODO: map URB to one or more Transfer TRBs, enqueue to the device's
        // transfer ring for the target endpoint, ring the endpoint doorbell.
        Ok(())
    }

    fn kill_urb(&mut self, _urb_context: u64) {
        // TODO: issue STOP_ENDPOINT command, dequeue the matching TRB.
    }

    fn get_device_descriptor(&mut self, _dev: &UsbDevice) -> Option<DeviceDescriptor> {
        // TODO: build a SETUP+DATA+STATUS URB, submit synchronously.
        None
    }

    fn get_config_descriptor(&mut self, _dev: &UsbDevice, _cfg_idx: u8)
        -> Option<ConfigDescriptor>
    {
        None // TODO
    }

    fn set_address(&mut self, _dev: &mut UsbDevice, _address: u8) {
        // TODO: issue ADDRESS_DEVICE command TRB.
    }

    fn set_configuration(&mut self, _dev: &mut UsbDevice, _config_value: u8) {
        // TODO: issue CONFIGURE_ENDPOINT command TRB.
    }

    fn get_port_status(&mut self, port: u8) -> Option<(u16, u16)> {
        unsafe {
            let offset = regs::OP_PORTSC + (port as usize - 1) * 0x10;
            let portsc = self.regs.op_read32(offset);
            // wPortStatus = bits 15:0, wPortChange = bits 31:16 (in USB hub terms).
            // xHCI PORTSC packs them differently — translate here.
            let status = portsc_to_port_status(portsc);
            let change = portsc_to_port_change(portsc);
            Some((status, change))
        }
    }

    fn port_reset(&mut self, port: u8) {
        unsafe {
            let offset = regs::OP_PORTSC + (port as usize - 1) * 0x10;
            let portsc = self.regs.op_read32(offset);
            // Set PR bit; preserve RW bits, clear RW1C bits.
            self.regs.op_write32(offset, (portsc & regs::PORTSC_RW_MASK) | regs::PORTSC_PR);
        }
    }

    fn set_port_feature(&mut self, _port: u8, _feature: u16) { /* TODO */ }
    fn clear_port_feature(&mut self, _port: u8, _feature: u16) { /* TODO */ }

    fn alloc_bandwidth(&mut self, _dev: &UsbDevice, _ep: u8, bpf: u32)
        -> Result<u32, HcdError>
    {
        Ok(bpf) // TODO: real bandwidth accounting
    }

    fn free_bandwidth(&mut self, _dev: &UsbDevice, _ep: u8) {}
}

// ── PORTSC translation helpers ────────────────────────────────────────────────

fn portsc_to_port_status(portsc: u32) -> u16 {
    use crate::descriptor::{
        PORT_STATUS_CONNECTION, PORT_STATUS_ENABLE,
        PORT_STATUS_RESET, PORT_STATUS_POWER,
    };
    use regs::*;
    let mut s: u16 = 0;
    if portsc & PORTSC_CCS  != 0 { s |= PORT_STATUS_CONNECTION; }
    if portsc & PORTSC_PED  != 0 { s |= PORT_STATUS_ENABLE; }
    if portsc & PORTSC_PR   != 0 { s |= PORT_STATUS_RESET; }
    if portsc & PORTSC_PP   != 0 { s |= PORT_STATUS_POWER; }
    s
}

fn portsc_to_port_change(portsc: u32) -> u16 {
    use crate::descriptor::{
        PORT_CHANGE_CONNECTION, PORT_CHANGE_RESET,
    };
    use regs::*;
    let mut c: u16 = 0;
    if portsc & PORTSC_CSC  != 0 { c |= PORT_CHANGE_CONNECTION; }
    if portsc & PORTSC_PRC  != 0 { c |= PORT_CHANGE_RESET; }
    c
}
