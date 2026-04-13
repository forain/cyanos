//! xHCI MMIO register definitions — from drivers/usb/host/xhci.h
//!
//! Register offsets are relative to the xHCI MMIO base unless noted.

// ── Capability register offsets (relative to MMIO base) ──────────────────────

pub const CAP_CAPLENGTH:  usize = 0x00; // u8: cap reg length (also op regs offset)
pub const CAP_HCIVERSION: usize = 0x02; // u16: HC interface version BCD
pub const CAP_HCSPARAMS1: usize = 0x04; // u32: structural params 1
pub const CAP_HCSPARAMS2: usize = 0x08; // u32: structural params 2
pub const CAP_HCSPARAMS3: usize = 0x0C; // u32: structural params 3
pub const CAP_HCCPARAMS1: usize = 0x10; // u32: capability params 1
pub const CAP_DBOFF:      usize = 0x14; // u32: doorbell offset
pub const CAP_RTSOFF:     usize = 0x18; // u32: runtime register space offset
pub const CAP_HCCPARAMS2: usize = 0x1C; // u32: capability params 2

// ── HCSPARAMS1 bit fields ─────────────────────────────────────────────────────

pub const HCSPARAMS1_MAX_SLOTS_MASK: u32 = 0x0000_00FF;
pub const HCSPARAMS1_MAX_INTRS_MASK: u32 = 0x0007_FF00;
pub const HCSPARAMS1_MAX_PORTS_MASK: u32 = 0xFF00_0000;

// ── Operational register offsets (relative to op_base = MMIO + CAPLENGTH) ────

pub const OP_USBCMD:  usize = 0x00; // u32
pub const OP_USBSTS:  usize = 0x04; // u32
pub const OP_PAGESIZE: usize = 0x08; // u32
pub const OP_DNCTRL:  usize = 0x14; // u32: device notification control
pub const OP_CRCR:    usize = 0x18; // u64: command ring control
pub const OP_DCBAAP:  usize = 0x30; // u64: device context base addr array ptr
pub const OP_CONFIG:  usize = 0x38; // u32: config register

/// PORTSC base (port 1 at this offset, port n at +0x10*(n-1)).
pub const OP_PORTSC:  usize = 0x400;

// ── USBCMD bits ───────────────────────────────────────────────────────────────

pub const CMD_RUN:     u32 = 1 << 0;  // Run/Stop
pub const CMD_RESET:   u32 = 1 << 1;  // HC Reset
pub const CMD_EIE:     u32 = 1 << 2;  // Interrupt Enable
pub const CMD_HSEIE:   u32 = 1 << 3;  // Host System Error Interrupt Enable
pub const CMD_LRESET:  u32 = 1 << 7;  // Light HC Reset
pub const CMD_CSS:     u32 = 1 << 8;  // Controller Save State
pub const CMD_CRS:     u32 = 1 << 9;  // Controller Restore State
pub const CMD_EWE:     u32 = 1 << 10; // Enable Wrap Event
pub const CMD_ETE:     u32 = 1 << 14; // Extended TBC Enable

// ── USBSTS bits ───────────────────────────────────────────────────────────────

pub const STS_HALT:    u32 = 1 << 0;
pub const STS_FATAL:   u32 = 1 << 2;
pub const STS_EINT:    u32 = 1 << 3;
pub const STS_PORT:    u32 = 1 << 4;
pub const STS_SAVE:    u32 = 1 << 8;
pub const STS_RESTORE: u32 = 1 << 9;
pub const STS_SRE:     u32 = 1 << 10;
pub const STS_CNR:     u32 = 1 << 11; // Controller Not Ready
pub const STS_HCE:     u32 = 1 << 12;

// ── CRCR bits ─────────────────────────────────────────────────────────────────

pub const CMD_RING_CYCLE:   u64 = 1 << 0;
pub const CMD_RING_PAUSE:   u64 = 1 << 1;
pub const CMD_RING_ABORT:   u64 = 1 << 2;
pub const CMD_RING_RUNNING: u64 = 1 << 3;

// ── PORTSC bits ───────────────────────────────────────────────────────────────

pub const PORTSC_CCS:  u32 = 1 << 0;  // Current Connect Status
pub const PORTSC_PED:  u32 = 1 << 1;  // Port Enabled/Disabled
pub const PORTSC_OCA:  u32 = 1 << 3;  // Over-current Active
pub const PORTSC_PR:   u32 = 1 << 4;  // Port Reset
pub const PORTSC_PLS:  u32 = 0xF << 5; // Port Link State (bits 8:5)
pub const PORTSC_PP:   u32 = 1 << 9;  // Port Power
pub const PORTSC_SPEED: u32 = 0xF << 10; // Port Speed (bits 13:10)
pub const PORTSC_PIC:  u32 = 3 << 14; // Port Indicator Control
pub const PORTSC_LWS:  u32 = 1 << 16; // Link Write Strobe (RW must write 1)
pub const PORTSC_CSC:  u32 = 1 << 17; // Connect Status Change (RW1C)
pub const PORTSC_PEC:  u32 = 1 << 18; // Port Enabled/Disabled Change (RW1C)
pub const PORTSC_WRC:  u32 = 1 << 19; // Warm Port Reset Change (RW1C)
pub const PORTSC_OCC:  u32 = 1 << 20; // Over-current Change (RW1C)
pub const PORTSC_PRC:  u32 = 1 << 21; // Port Reset Change (RW1C)
pub const PORTSC_PLC:  u32 = 1 << 22; // Port Link State Change (RW1C)
pub const PORTSC_CEC:  u32 = 1 << 23; // Config Error Change (RW1C)
pub const PORTSC_CAS:  u32 = 1 << 24; // Cold Attach Status
pub const PORTSC_WCE:  u32 = 1 << 25; // Wake on Connect Enable
pub const PORTSC_WDE:  u32 = 1 << 26; // Wake on Disconnect Enable
pub const PORTSC_WOE:  u32 = 1 << 27; // Wake on Over-current Enable
pub const PORTSC_DR:   u32 = 1 << 30; // Device Removable
pub const PORTSC_WPR:  u32 = 1 << 31; // Warm Port Reset

/// Mask for read-write fields (preserve when writing, don't toggle RW1C bits).
pub const PORTSC_RW_MASK: u32 =
    PORTSC_PP | PORTSC_PIC | PORTSC_LWS | PORTSC_WCE | PORTSC_WDE | PORTSC_WOE;

// ── Runtime interrupter register offsets (relative to rt_base = MMIO + RTSOFF) ─

pub const RT_MFINDEX: usize = 0x000; // u32: microframe index

/// Interrupter n register set starts at rt_base + 0x20 + n*0x20.
pub const IR_IMAN:   usize = 0x00; // u32: interrupt management
pub const IR_IMOD:   usize = 0x04; // u32: interrupt moderation
pub const IR_ERSTSZ: usize = 0x08; // u32: event ring segment table size
pub const IR_ERSTBA: usize = 0x10; // u64: event ring segment table base addr
pub const IR_ERDP:   usize = 0x18; // u64: event ring dequeue pointer

// ── IMAN bits ─────────────────────────────────────────────────────────────────

pub const IMAN_IP: u32 = 1 << 0; // Interrupt Pending (RW1C)
pub const IMAN_IE: u32 = 1 << 1; // Interrupt Enable

// ── IMOD default value ────────────────────────────────────────────────────────

/// Default: interrupt moderation interval = 4000 (× 250 ns = 1 ms).
pub const IMOD_DEFAULT: u32 = 4000;

// ── ERST Dequeue pointer bits ─────────────────────────────────────────────────

pub const ERDP_EHB: u64 = 1 << 3; // Event Handler Busy (RW1C)

// ── Register accessor ─────────────────────────────────────────────────────────

/// Thin wrapper over the xHCI MMIO region, providing typed read/write helpers.
pub struct XhciRegs {
    pub mmio_base: usize,
    pub op_base:   usize,
    pub rt_base:   usize,
    pub db_base:   usize,
}

impl XhciRegs {
    /// # Safety
    /// `mmio_base` must be a valid, mapped xHCI MMIO base address.
    pub unsafe fn new(mmio_base: usize) -> Self {
        let cap_len = (mmio_base as *const u8).add(CAP_CAPLENGTH).read_volatile() as usize;
        let hccparams1 = (mmio_base as *const u32).add(CAP_HCCPARAMS1 / 4).read_volatile();
        let db_offset  = (mmio_base as *const u32).add(CAP_DBOFF  / 4).read_volatile() as usize & !3;
        let rt_offset  = (mmio_base as *const u32).add(CAP_RTSOFF / 4).read_volatile() as usize & !0x1F;
        let _ = hccparams1;
        Self {
            mmio_base,
            op_base: mmio_base + cap_len,
            rt_base: mmio_base + rt_offset,
            db_base: mmio_base + db_offset,
        }
    }

    pub unsafe fn op_read32(&self, off: usize) -> u32 {
        ((self.op_base + off) as *const u32).read_volatile()
    }
    pub unsafe fn op_write32(&mut self, off: usize, val: u32) {
        ((self.op_base + off) as *mut u32).write_volatile(val)
    }
    pub unsafe fn op_read64(&self, off: usize) -> u64 {
        ((self.op_base + off) as *const u64).read_volatile()
    }
    pub unsafe fn op_write64(&mut self, off: usize, val: u64) {
        ((self.op_base + off) as *mut u64).write_volatile(val)
    }

    /// Interrupter n register read (32-bit).
    pub unsafe fn ir_read32(&self, n: usize, off: usize) -> u32 {
        let base = self.rt_base + 0x20 + n * 0x20;
        ((base + off) as *const u32).read_volatile()
    }
    pub unsafe fn ir_write32(&mut self, n: usize, off: usize, val: u32) {
        let base = self.rt_base + 0x20 + n * 0x20;
        ((base + off) as *mut u32).write_volatile(val)
    }
    pub unsafe fn ir_write64(&mut self, n: usize, off: usize, val: u64) {
        let base = self.rt_base + 0x20 + n * 0x20;
        ((base + off) as *mut u64).write_volatile(val)
    }

    /// Raw offset of the doorbell array from MMIO base.
    pub fn db_offset(&self) -> u32 {
        (self.db_base - self.mmio_base) as u32
    }
}
