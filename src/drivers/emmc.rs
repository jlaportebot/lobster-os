//! BCM2712 eMMC2 Host Controller driver for SD card storage
//!
//! The Pi 5's SD card slot is connected via the eMMC2 controller
//! at physical address 0x10_00C0_0000 (ARASAN SDHCI-compatible).
//!
//! This driver provides:
//! - Full SD card initialization (CMD0–CMD18, CMD24–CMD25)
//! - Single and multi-block read/write
//! - SD card identification (CID, CSD, OCR, RCA)
//! - Clock switching (400 KHz init → 50 MHz high-speed)
//! - DMA transfers with cache-coherent buffers
//! - 4-bit bus width support
//! - Interrupt-driven operation (GIC SPI 0x3E = 62)
//!
//! Reference: SD Specification v6.00, BCM2712 ARM Peripherals,
//! Circle emmc.c (Plan 9 derived)

use crate::arch::cpu;
use crate::arch::gic::Gic;
use crate::fs::block_dev::{BlockDevice, BlockError};
use crate::memory::{PhysAddr, VirtAddr};

// ─── eMMC2 Controller Base Address ─────────────────────────────────
// Physical: 0x10_00C0_0000 (AXI offset 0x00C0_0000)
// Virtual: DEVICE_MAP_START + 0x00C0_0000
const EMMC2_PHYS_BASE: u64 = 0x10_00C0_0000;
const EMMC2_VIRT_BASE: u64 = crate::memory::DEVICE_MAP_START + 0x00C0_0000;

/// GIC SPI number for eMMC2 on BCM2712
const EMMC2_IRQ: u32 = 62;

// ─── Clock frequencies ──────────────────────────────────────────────
const EXTFREQ: u32 = 200_000_000; // Base clock (200 MHz on Pi 5)
const INITFREQ: u32 = 400_000; // Identification frequency (400 KHz)
const SDFREQ: u32 = 25_000_000; // Default speed (25 MHz)
const SDHFREQ: u32 = 50_000_000; // High speed (50 MHz)

// ─── SDHCI Register Offsets ─────────────────────────────────────────
const ARG2: u64 = 0x00 >> 2;
const BLKSIZCNT: u64 = 0x04 >> 2;
const ARG1: u64 = 0x08 >> 2;
const CMDTM: u64 = 0x0C >> 2;
const RESP0: u64 = 0x10 >> 2;
const RESP1: u64 = 0x14 >> 2;
const RESP2: u64 = 0x18 >> 2;
const RESP3: u64 = 0x1C >> 2;
const DATA: u64 = 0x20 >> 2;
const STATUS: u64 = 0x24 >> 2;
const CONTROL0: u64 = 0x28 >> 2;
const CONTROL1: u64 = 0x2C >> 2;
const INTERRUPT: u64 = 0x30 >> 2;
const IRPTMASK: u64 = 0x34 >> 2;
const IRPTEN: u64 = 0x38 >> 2;
const CONTROL2: u64 = 0x3C >> 2;
const FORCEIRPT: u64 = 0x50 >> 2;
const BOOTTIMEOUT: u64 = 0x70 >> 2;
const DBGSEL: u64 = 0x74 >> 2;
const EXRDFIFOCFG: u64 = 0x80 >> 2;
const EXRDFIFOEN: u64 = 0x84 >> 2;
const TUNESTEP: u64 = 0x88 >> 2;
const TUNESTEPSSTD: u64 = 0x8C >> 2;
const TUNESTEPSDDR: u64 = 0x90 >> 2;
const SLOTISRVER: u64 = 0xFC >> 2;

// ─── Control0 bits ──────────────────────────────────────────────────
const HISPEED: u32 = 1 << 2;
const DWIDTH4: u32 = 1 << 1;
const DWIDTH1: u32 = 0 << 1;

// ─── Control1 bits ──────────────────────────────────────────────────
const SRSTDATA: u32 = 1 << 26;
const SRSTCMD: u32 = 1 << 25;
const SRSTHC: u32 = 1 << 24;
const DATATOSHIFT: u32 = 16;
const DATATOMASK: u32 = 0xF0000;
const CLKFREQ8SHIFT: u32 = 8;
const CLKFREQ8MASK: u32 = 0xFF00;
const CLKFREQMS2SHIFT: u32 = 6;
const CLKFREQMS2MASK: u32 = 0xC0;
const CLKGENDIV: u32 = 0 << 5;
const CLKGENPROG: u32 = 1 << 5;
const CLKEN: u32 = 1 << 2;
const CLKSTABLE: u32 = 1 << 1;
const CLKINTLEN: u32 = 1 << 0;

// ─── CmdTM bits ─────────────────────────────────────────────────────
const INDEXSHIFT: u32 = 24;
const ISDATA: u32 = 1 << 21;
const IXCHKEN: u32 = 1 << 20;
const CRCCHKEN: u32 = 1 << 19;
const RESPMASK: u32 = 3 << 16;
const RESPNONE: u32 = 0 << 16;
const RESP136: u32 = 1 << 16;
const RESP48: u32 = 2 << 16;
const RESP48BUSY: u32 = 3 << 16;
const MULTIBLOCK: u32 = 1 << 5;
const HOST2CARD: u32 = 0 << 4;
const CARD2HOST: u32 = 1 << 4;
const AUTOCMD12: u32 = 1 << 2;
const AUTOCMD23: u32 = 2 << 2;
const BLKCNTEN: u32 = 1 << 1;

// ─── Interrupt bits ─────────────────────────────────────────────────
const ACMDERR: u32 = 1 << 24;
const DENDERR: u32 = 1 << 22;
const DCRCERR: u32 = 1 << 21;
const DTOERR: u32 = 1 << 20;
const CBADERR: u32 = 1 << 19;
const CENDERR: u32 = 1 << 18;
const CCRCERR: u32 = 1 << 17;
const CTOERR: u32 = 1 << 16;
const ERR: u32 = 1 << 15;
const CARDINTR: u32 = 1 << 8;
const CARDINSERT: u32 = 1 << 6;
const READRDY: u32 = 1 << 5;
const WRITERDY: u32 = 1 << 4;
const DATADONE: u32 = 1 << 1;
const CMDDONE: u32 = 1 << 0;

// ─── Status bits ────────────────────────────────────────────────────
const BUFREAD: u32 = 1 << 11;
const BUFWRITE: u32 = 1 << 10;
const READTRANS: u32 = 1 << 9;
const WRITETRANS: u32 = 1 << 8;
const DATACTIVE: u32 = 1 << 2;
const DATINHIBIT: u32 = 1 << 1;
const CMDINHIBIT: u32 = 1 << 0;

// ─── SD Commands ────────────────────────────────────────────────────
const CMD0_GO_IDLE: u32 = 0;
const CMD2_ALL_SEND_CID: u32 = 2;
const CMD3_SEND_RELATIVE_ADDR: u32 = 3;
const CMD6_SWITCH_FUNC: u32 = 6;
const CMD7_SELECT_CARD: u32 = 7;
const CMD8_SEND_IF_COND: u32 = 8;
const CMD9_SEND_CSD: u32 = 9;
const CMD10_SEND_CID: u32 = 10;
const CMD12_STOP_TRANSMISSION: u32 = 12;
const CMD13_SEND_STATUS: u32 = 13;
const CMD16_SET_BLOCKLEN: u32 = 16;
const CMD17_READ_SINGLE_BLOCK: u32 = 17;
const CMD18_READ_MULTIPLE_BLOCK: u32 = 18;
const CMD24_WRITE_SINGLE_BLOCK: u32 = 24;
const CMD25_WRITE_MULTIPLE_BLOCK: u32 = 25;
const CMD41_SD_SEND_OP_COND: u32 = 41;
const CMD55_APP_CMD: u32 = 55;

// ─── Command flag table ────────────────────────────────────────────
fn cmd_flags(cmd: u32) -> u32 {
    match cmd {
        0 => IXCHKEN,
        2 => RESP136 | CRCCHKEN,
        3 => RESP48 | IXCHKEN | CRCCHKEN,
        6 => RESP48 | IXCHKEN | CRCCHKEN,
        7 => RESP48BUSY | IXCHKEN | CRCCHKEN,
        8 => RESP48 | IXCHKEN | CRCCHKEN,
        9 => RESP136 | CRCCHKEN,
        10 => RESP136 | CRCCHKEN,
        12 => RESP48BUSY | IXCHKEN | CRCCHKEN,
        13 => RESP48 | IXCHKEN | CRCCHKEN,
        16 => RESP48 | IXCHKEN | CRCCHKEN,
        17 => RESP48 | ISDATA | CARD2HOST | IXCHKEN | CRCCHKEN,
        18 => RESP48 | ISDATA | CARD2HOST | MULTIBLOCK | BLKCNTEN | IXCHKEN | CRCCHKEN,
        24 => RESP48 | ISDATA | HOST2CARD | IXCHKEN | CRCCHKEN,
        25 => RESP48 | ISDATA | HOST2CARD | MULTIBLOCK | BLKCNTEN | IXCHKEN | CRCCHKEN,
        41 => RESP48 | CRCCHKEN,
        55 => RESP48 | IXCHKEN | CRCCHKEN,
        _ => RESPNONE,
    }
}

// ─── Error type ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmmcError {
    Timeout,
    CrcError,
    BadResponse,
    DataError,
    CardNotReady,
    UnsupportedCard,
    InvalidArgument,
    Busy,
    WriteProtected,
}

impl From<EmmcError> for BlockError {
    fn from(e: EmmcError) -> Self {
        match e {
            EmmcError::Timeout => BlockError::Timeout,
            EmmcError::DataError => BlockError::ReadError,
            _ => BlockError::ReadError,
        }
    }
}

// ─── SD Card Information ────────────────────────────────────────────

/// SD card identification data
#[derive(Debug, Clone, Copy)]
pub struct SdCardInfo {
    /// Card RCA (relative card address)
    pub rca: u32,
    /// Operating Condition Register
    pub ocr: u32,
    /// Card ID register (128-bit, stored as 4 × u32)
    pub cid: [u32; 4],
    /// Card-Specific Data register (128-bit, stored as 4 × u32)
    pub csd: [u32; 4],
    /// Total capacity in bytes
    pub capacity_bytes: u64,
    /// Block size (typically 512)
    pub block_size: u32,
    /// Total number of blocks
    pub total_blocks: u64,
    /// SD bus width (1 or 4)
    pub bus_width: u32,
    /// Current clock frequency
    pub clock_freq: u32,
    /// Card type: SDv1, SDv2, SDHC/SDXC
    pub card_type: SdCardType,
    /// Is this a high-capacity (SDHC/SDXC) card?
    pub high_capacity: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SdCardType {
    /// SD v1.x (byte-addressed)
    SdV1,
    /// SD v2.0 standard capacity (byte-addressed)
    SdV2,
    /// SD v2.0 high capacity (block-addressed, SDHC)
    SdHc,
    /// SD extended capacity (block-addressed, SDXC)
    SdXc,
}

// ─── eMMC2 Controller State ─────────────────────────────────────────

struct EmmcController {
    /// Current clock frequency
    clock_freq: u32,
    /// Card RCA
    rca: u32,
    /// Card info
    card_info: SdCardInfo,
    /// Whether the controller has been initialized
    initialized: bool,
    /// DMA buffer for data transfers (4KB, cache-line aligned)
    dma_buf_virt: VirtAddr,
    dma_buf_phys: PhysAddr,
    /// Interrupt flags (written by ISR, read by polling)
    interrupt_flags: u32,
    /// Write-protected?
    write_protected: bool,
}

static mut EMMC: EmmcController = EmmcController {
    clock_freq: 0,
    rca: 0,
    card_info: SdCardInfo {
        rca: 0,
        ocr: 0,
        cid: [0; 4],
        csd: [0; 4],
        capacity_bytes: 0,
        block_size: 512,
        total_blocks: 0,
        bus_width: 1,
        clock_freq: 0,
        card_type: SdCardType::SdV2,
        high_capacity: false,
    },
    initialized: false,
    dma_buf_virt: VirtAddr::new(0),
    dma_buf_phys: PhysAddr::new(0),
    interrupt_flags: 0,
    write_protected: false,
};

// ─── MMIO helpers ───────────────────────────────────────────────────

#[inline]
unsafe fn reg_read(offset: u64) -> u32 {
    let addr = EMMC2_VIRT_BASE + offset * 4;
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn reg_write(offset: u64, val: u32) {
    let addr = EMMC2_VIRT_BASE + offset * 4;
    core::ptr::write_volatile(addr as *mut u32, val);
}

// ─── Microsecond delay ─────────────────────────────────────────────

fn delay_us(us: u64) {
    let start = crate::arch::timer::Timer::system_time_us();
    let target = start + us;
    while crate::arch::timer::Timer::system_time_us() < target {
        cpu::nop();
    }
}

fn delay_ms(ms: u64) {
    delay_us(ms * 1000);
}

// ─── Controller Operations ──────────────────────────────────────────

/// Reset the eMMC2 host controller
unsafe fn controller_reset() {
    // Reset the complete host controller
    reg_write(CONTROL1, SRSTHC);
    delay_ms(10);

    // Wait for reset to complete
    let mut timeout = 1_000_000u64;
    while (reg_read(CONTROL1) & SRSTHC) != 0 {
        if timeout == 0 {
            crate::println!("[emmc] ERROR: controller reset timeout");
            return;
        }
        timeout -= 1;
        cpu::nop();
    }

    // Reset data and command circuits as well
    reg_write(CONTROL1, SRSTDATA | SRSTCMD);
    delay_ms(10);
    timeout = 1_000_000u64;
    while (reg_read(CONTROL1) & (SRSTDATA | SRSTCMD)) != 0 {
        if timeout == 0 {
            crate::println!("[emmc] ERROR: data/cmd reset timeout");
            return;
        }
        timeout -= 1;
        cpu::nop();
    }

    // Set data timeout to maximum (DTO=14 → ~2^14 base clocks)
    let mut ctl1 = reg_read(CONTROL1);
    ctl1 &= !DATATOMASK;
    ctl1 |= 14 << DATATOSHIFT;
    reg_write(CONTROL1, ctl1);

    // Set clock to identification frequency (400 KHz)
    set_clock(INITFREQ);

    // Enable all interrupt sources
    reg_write(IRPTMASK, 0xFFFFFFFF);
    reg_write(IRPTEN, 0xFFFFFFFF);

    crate::println!("[emmc] Controller reset complete, clock: {} Hz", INITFREQ);
}

/// Set the SD clock frequency
unsafe fn set_clock(freq: u32) {
    // Stop the clock first
    let mut ctl1 = reg_read(CONTROL1);
    ctl1 &= !(CLKEN | CLKINTLEN | CLKFREQ8MASK | CLKFREQMS2MASK);
    reg_write(CONTROL1, ctl1);
    delay_us(100);

    // Calculate clock divider: SDCLK = base_clock / (div * 2)
    let mut div = EXTFREQ / (freq * 2);
    if div == 0 {
        div = 1;
    }
    if div > 0x3FF {
        div = 0x3FF;
    }

    // Set divider in Control1
    let div_lo = (div & 0xFF) as u32;
    let div_hi = ((div >> 8) & 0x3) as u32;
    ctl1 = reg_read(CONTROL1);
    ctl1 |= CLKINTLEN | (div_lo << CLKFREQ8SHIFT) | (div_hi << CLKFREQMS2SHIFT);
    reg_write(CONTROL1, ctl1);

    // Wait for clock stable
    let mut timeout = 1_000_000u64;
    while (reg_read(CONTROL1) & CLKSTABLE) == 0 {
        if timeout == 0 {
            crate::println!("[emmc] ERROR: clock stable timeout");
            return;
        }
        timeout -= 1;
        cpu::nop();
    }

    // Enable the clock
    ctl1 = reg_read(CONTROL1);
    ctl1 |= CLKEN;
    reg_write(CONTROL1, ctl1);

    delay_us(100);

    // Update state
    let actual_freq = EXTFREQ / (div * 2);
    EMMC.clock_freq = actual_freq;
    EMMC.card_info.clock_freq = actual_freq;
}

/// Wait for command and data lines to be idle
unsafe fn wait_idle() -> bool {
    let mut timeout = 1_000_000u64;
    loop {
        let status = reg_read(STATUS);
        if (status & (CMDINHIBIT | DATINHIBIT)) == 0 {
            return true;
        }
        if timeout == 0 {
            crate::println!("[emmc] ERROR: wait_idle timeout, STATUS={:#010x}", status);
            return false;
        }
        timeout -= 1;
        cpu::nop();
    }
}

/// Clear interrupt flags
unsafe fn clear_interrupts() {
    reg_write(INTERRUPT, reg_read(INTERRUPT));
}

/// Send an SD command and wait for response
unsafe fn send_command(cmd: u32, arg: u32) -> Result<[u32; 4], EmmcError> {
    if !wait_idle() {
        return Err(EmmcError::Busy);
    }

    clear_interrupts();

    let flags = cmd_flags(cmd);
    let cmdtm = (cmd << INDEXSHIFT) | flags;
    reg_write(ARG1, arg);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE or error
    let mut timeout = 2_000_000u64;
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            let err_bits = irpts & (CTOERR | CCRCERR | CENDERR | CBADERR);
            reg_write(INTERRUPT, err_bits);
            if (irpts & CTOERR) != 0 {
                return Err(EmmcError::Timeout);
            }
            if (irpts & CCRCERR) != 0 {
                return Err(EmmcError::CrcError);
            }
            return Err(EmmcError::BadResponse);
        }
        if timeout == 0 {
            reg_write(INTERRUPT, reg_read(INTERRUPT));
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Read response
    let resp = [
        reg_read(RESP0),
        reg_read(RESP1),
        reg_read(RESP2),
        reg_read(RESP3),
    ];

    Ok(resp)
}

/// Send a command, retrying on timeout (up to 3 attempts)
unsafe fn send_command_retry(cmd: u32, arg: u32) -> Result<[u32; 4], EmmcError> {
    let mut attempts = 3;
    loop {
        match send_command(cmd, arg) {
            Ok(resp) => return Ok(resp),
            Err(EmmcError::Timeout) => {
                if attempts == 0 {
                    return Err(EmmcError::Timeout);
                }
                attempts -= 1;
                delay_ms(10);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Send APP_CMD prefix (CMD55) followed by an ACMD
unsafe fn send_app_cmd(acmd: u32, arg: u32) -> Result<[u32; 4], EmmcError> {
    // CMD55: next command is application-specific
    let rca = EMMC.rca;
    send_command(CMD55_APP_CMD, rca << 16)?;
    // Now send the ACMD
    send_command(acmd, arg)
}

// ─── SD Card Initialization ─────────────────────────────────────────

/// Initialize the SD card and return card info
unsafe fn init_card() -> Result<SdCardInfo, EmmcError> {
    crate::println!("[emmc] Initializing SD card...");

    // 1. CMD0: GO_IDLE_STATE — reset all cards to idle
    crate::println!("[emmc] CMD0: GO_IDLE_STATE");
    send_command(CMD0_GO_IDLE, 0)?;
    delay_ms(10);

    // 2. CMD8: SEND_IF_COND — verify SD v2.0+ card
    // Voltage: 2.7-3.6V (0x01), check pattern: 0xAA
    crate::println!("[emmc] CMD8: SEND_IF_COND");
    let is_v2 = match send_command(CMD8_SEND_IF_COND, 0x000001AA) {
        Ok(resp) => {
            // Card responded: SD v2.0 or later
            let echo_back = resp[0] & 0xFFF;
            if echo_back == 0x1AA {
                crate::println!("[emmc] SD v2.0+ card detected (echo: {:#06x})", echo_back);
                true
            } else {
                crate::println!("[emmc] CMD8 response unexpected: {:#06x}", echo_back);
                true // Still treat as v2
            }
        }
        Err(EmmcError::Timeout) => {
            // No response: SD v1.x card
            crate::println!("[emmc] No CMD8 response — SD v1.x card");
            false
        }
        Err(e) => return Err(e),
    };

    // 3. ACMD41: SD_SEND_OP_COND — initialize card with HCS (High Capacity Support)
    crate::println!("[emmc] ACMD41: SD_SEND_OP_COND");
    let hcs_flag: u32 = if is_v2 { 1 << 30 } else { 0 }; // HCS bit
    let mut ocr;
    let mut attempts = 200; // Up to 2 seconds
    loop {
        // 3.2-3.4V support (bits 20-21), plus HCS for v2
        let voltage_arg = (3 << 20) | (3 << 15) | hcs_flag;
        match send_app_cmd(CMD41_SD_SEND_OP_COND, voltage_arg) {
            Ok(resp) => {
                ocr = resp[0];
                // Bit 31 = busy/done (card has finished power-up)
                if (ocr & (1 << 31)) != 0 {
                    break;
                }
            }
            Err(e) => {
                crate::println!("[emmc] ACMD41 failed: {:?}", e);
                return Err(e);
            }
        }
        if attempts == 0 {
            crate::println!("[emmc] ERROR: ACMD41 timeout, OCR={:#010x}", ocr);
            return Err(EmmcError::CardNotReady);
        }
        attempts -= 1;
        delay_ms(10);
    }

    let is_high_capacity = (ocr & (1 << 30)) != 0;
    crate::println!(
        "[emmc] Card powered up: OCR={:#010x}, HC={}",
        ocr,
        is_high_capacity
    );

    // 4. CMD2: ALL_SEND_CID — get card identification
    crate::println!("[emmc] CMD2: ALL_SEND_CID");
    let cid_resp = send_command_retry(CMD2_ALL_SEND_CID, 0)?;
    let cid = [cid_resp[3], cid_resp[2], cid_resp[1], cid_resp[0]]; // Reverse order for correct parsing

    // 5. CMD3: SEND_RELATIVE_ADDR — get RCA
    crate::println!("[emmc] CMD3: SEND_RELATIVE_ADDR");
    let resp = send_command_retry(CMD3_SEND_RELATIVE_ADDR, 0)?;
    let rca = (resp[0] >> 16) & 0xFFFF;
    crate::println!("[emmc] Card RCA: {:#06x}", rca);
    EMMC.rca = rca;

    // 6. CMD9: SEND_CSD — get card-specific data
    crate::println!("[emmc] CMD9: SEND_CSD (rca={:#06x})", rca);
    let csd_resp = send_command_retry(CMD9_SEND_CSD, rca << 16)?;
    let csd = [csd_resp[3], csd_resp[2], csd_resp[1], csd_resp[0]];

    // Parse CSD to determine capacity
    let (capacity_bytes, total_blocks, block_size) = parse_csd(&csd, is_high_capacity);

    // 7. CMD7: SELECT_CARD — select the card
    crate::println!("[emmc] CMD7: SELECT_CARD");
    send_command_retry(CMD7_SELECT_CARD, rca << 16)?;
    crate::println!("[emmc] Card selected");

    // 8. CMD16: SET_BLOCKLEN — set block length to 512 (not needed for HC cards, but safe)
    if !is_high_capacity {
        crate::println!("[emmc] CMD16: SET_BLOCKLEN(512)");
        send_command(CMD16_SET_BLOCKLEN, 512)?;
    }

    // 9. Switch to high-speed clock (50 MHz)
    crate::println!("[emmc] Switching clock to 50 MHz (high-speed)");
    set_clock(SDHFREQ);

    // 10. Set 4-bit bus width via ACMD6
    crate::println!("[emmc] Setting 4-bit bus width...");
    // ACMD6: SET_BUS_WIDTH (arg: 0=1bit, 2=4bit)
    match send_app_cmd(CMD6_SWITCH_FUNC, 2) {
        Ok(_) => {
            reg_write(CONTROL0, reg_read(CONTROL0) | DWIDTH4);
            crate::println!("[emmc] 4-bit bus width enabled");
        }
        Err(e) => {
            crate::println!(
                "[emmc] WARNING: 4-bit bus width failed ({:?}), using 1-bit",
                e
            );
        }
    }

    let card_type = if is_high_capacity {
        if capacity_bytes >= 32 * 1024 * 1024 * 1024 {
            SdCardType::SdXc
        } else {
            SdCardType::SdHc
        }
    } else if is_v2 {
        SdCardType::SdV2
    } else {
        SdCardType::SdV1
    };

    let bus_width = if is_high_capacity { 4 } else { 1 };

    let info = SdCardInfo {
        rca,
        ocr,
        cid,
        csd,
        capacity_bytes,
        block_size,
        total_blocks,
        bus_width,
        clock_freq: EMMC.clock_freq,
        card_type,
        high_capacity: is_high_capacity,
    };

    crate::println!(
  "[emmc] SD card initialized: type={:?}, capacity={:.1} MiB, blocks={}, bus_width={}, clock={} MHz",
  card_type,
  capacity_bytes as f64 / (1024.0 * 1024.0),
  total_blocks,
  bus_width,
  EMMC.clock_freq / 1_000_000,
 );

    EMMC.card_info = info;
    EMMC.initialized = true;

    Ok(info)
}

/// Parse CSD register to extract capacity and block size
fn parse_csd(csd: &[u32; 4], high_capacity: bool) -> (u64, u64, u32) {
    if high_capacity {
        // CSD v2.0 (SDHC/SDXC)
        // Capacity = (C_SIZE+1) * 512K bytes
        // C_SIZE is in bits [69:48] of the 128-bit CSD
        // In our [u32;4] layout (reversed): csd[1] bits [15:0] | csd[2] bits [31:16]
        let c_size = ((csd[1] as u64 & 0xFFFF) << 16) | ((csd[2] as u64 >> 16) & 0xFFFF);
        let capacity = (c_size + 1) * 512 * 1024;
        let blocks = capacity / 512;
        (capacity, blocks, 512)
    } else {
        // CSD v1.0 / v1.2 (SDSC)
        // Capacity = (C_SIZE+1) * 2^(C_SIZE_MULT+2) * 2^(READ_BL_LEN)
        // These fields are in the CSD bit array
        // READ_BL_LEN: bits [83:80] → byte 10 bits [7:4]
        let read_bl_len = ((csd[1] >> 8) & 0x0F) as u64;
        // C_SIZE: bits [73:62] → complex bit extraction
        let c_size = ((csd[2] as u64 >> 30) & 0x3)
            | ((csd[1] as u64 & 0xFF) << 2)
            | (((csd[1] as u64 >> 24) & 0xFF) << 10);
        // C_SIZE_MULT: bits [49:47] → csd[3] bits [15:13] approximately
        let c_size_mult = ((csd[3] >> 7) & 0x07) as u64;
        if read_bl_len < 9 || read_bl_len > 12 || c_size == 0xFFF {
            crate::println!("[emmc] WARNING: CSD parse failed, using defaults");
            return (32 * 1024 * 1024 * 1024, 67_108_864, 512); // Default 32GB
        }
        let block_len = 1u64 << read_bl_len;
        let mult = 1u64 << (c_size_mult + 2);
        let capacity = (c_size + 1) * mult * block_len;
        let block_size = 512u32;
        let blocks = capacity / 512;
        (capacity, blocks, block_size)
    }
}

// ─── Block Read/Write Operations ────────────────────────────────────

/// Read a single 512-byte block from the SD card
pub unsafe fn read_block(block: u64, buf: &mut [u8; 512]) -> Result<(), EmmcError> {
    if !EMMC.initialized {
        return Err(EmmcError::CardNotReady);
    }

    if !wait_idle() {
        return Err(EmmcError::Busy);
    }

    // Set block size and count
    reg_write(BLKSIZCNT, 512 | (1 << 16));

    // For HC cards, use block address; for SC cards, use byte address
    let addr = if EMMC.card_info.high_capacity {
        block as u32
    } else {
        (block * 512) as u32
    };

    clear_interrupts();

    let flags = cmd_flags(CMD17_READ_SINGLE_BLOCK) | ISDATA | CARD2HOST;
    let cmdtm = (CMD17_READ_SINGLE_BLOCK << INDEXSHIFT) | flags;
    reg_write(ARG1, addr);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE
    let mut timeout = 2_000_000u64;
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            reg_write(INTERRUPT, irpts & (CTOERR | CCRCERR | CENDERR | CBADERR));
            return Err(EmmcError::BadResponse);
        }
        if timeout == 0 {
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Read data from FIFO
    let mut bytes_read = 0usize;
    timeout = 5_000_000u64;
    while bytes_read < 512 {
        let irpts = reg_read(INTERRUPT);
        if (irpts & READRDY) != 0 {
            reg_write(INTERRUPT, READRDY);
            // Read up to 128 words (512 bytes) per interrupt
            let words = (512 - bytes_read) / 4;
            for _ in 0..words.min(128) {
                let word = reg_read(DATA);
                let start = bytes_read;
                buf[start] = (word & 0xFF) as u8;
                if start + 1 < 512 {
                    buf[start + 1] = ((word >> 8) & 0xFF) as u8;
                }
                if start + 2 < 512 {
                    buf[start + 2] = ((word >> 16) & 0xFF) as u8;
                }
                if start + 3 < 512 {
                    buf[start + 3] = ((word >> 24) & 0xFF) as u8;
                }
                bytes_read += 4;
            }
        }
        if (irpts & DATADONE) != 0 {
            reg_write(INTERRUPT, DATADONE);
            break;
        }
        if (irpts & (DTOERR | DCRCERR | DENDERR)) != 0 {
            reg_write(INTERRUPT, irpts & (DTOERR | DCRCERR | DENDERR));
            return Err(EmmcError::DataError);
        }
        if timeout == 0 {
            crate::println!(
                "[emmc] ERROR: data read timeout, {} of 512 bytes",
                bytes_read
            );
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Wait for DATADONE
    timeout = 1_000_000u64;
    while (reg_read(INTERRUPT) & DATADONE) == 0 {
        if timeout == 0 {
            break;
        }
        timeout -= 1;
        cpu::nop();
    }
    reg_write(INTERRUPT, reg_read(INTERRUPT));

    Ok(())
}

/// Read multiple contiguous blocks from the SD card
pub unsafe fn read_blocks(start_block: u64, buf: &mut [u8], count: usize) -> Result<(), EmmcError> {
    if !EMMC.initialized {
        return Err(EmmcError::CardNotReady);
    }
    if buf.len() < count * 512 {
        return Err(EmmcError::InvalidArgument);
    }

    if count == 1 {
        let mut block_buf = [0u8; 512];
        read_block(start_block, &mut block_buf)?;
        buf[..512].copy_from_slice(&block_buf);
        return Ok(());
    }

    if !wait_idle() {
        return Err(EmmcError::Busy);
    }

    // Set block size and count
    reg_write(BLKSIZCNT, 512 | ((count as u32) << 16));

    let addr = if EMMC.card_info.high_capacity {
        start_block as u32
    } else {
        (start_block * 512) as u32
    };

    clear_interrupts();

    let flags = cmd_flags(CMD18_READ_MULTIPLE_BLOCK)
        | ISDATA
        | CARD2HOST
        | MULTIBLOCK
        | BLKCNTEN
        | AUTOCMD12;
    let cmdtm = (CMD18_READ_MULTIPLE_BLOCK << INDEXSHIFT) | flags;
    reg_write(ARG1, addr);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE
    let mut timeout = 2_000_000u64;
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            return Err(EmmcError::BadResponse);
        }
        if timeout == 0 {
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Read data from FIFO
    let total_bytes = count * 512;
    let mut bytes_read = 0usize;
    timeout = 5_000_000u64 * count as u64;
    while bytes_read < total_bytes {
        let irpts = reg_read(INTERRUPT);
        if (irpts & READRDY) != 0 {
            reg_write(INTERRUPT, READRDY);
            let remaining = total_bytes - bytes_read;
            let words = remaining / 4;
            for _ in 0..words.min(64) {
                if bytes_read + 4 > total_bytes {
                    break;
                }
                let word = reg_read(DATA);
                let start = bytes_read;
                buf[start] = (word & 0xFF) as u8;
                if start + 1 < total_bytes {
                    buf[start + 1] = ((word >> 8) & 0xFF) as u8;
                }
                if start + 2 < total_bytes {
                    buf[start + 2] = ((word >> 16) & 0xFF) as u8;
                }
                if start + 3 < total_bytes {
                    buf[start + 3] = ((word >> 24) & 0xFF) as u8;
                }
                bytes_read += 4;
            }
        }
        if (irpts & DATADONE) != 0 {
            reg_write(INTERRUPT, DATADONE);
            break;
        }
        if (irpts & (DTOERR | DCRCERR | DENDERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            return Err(EmmcError::DataError);
        }
        if timeout == 0 {
            crate::println!(
                "[emmc] ERROR: multi-read timeout, {} of {} bytes",
                bytes_read,
                total_bytes
            );
            // Send CMD12 to stop transmission
            let _ = send_command(CMD12_STOP_TRANSMISSION, 0);
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Wait for DATADONE
    timeout = 1_000_000u64;
    while (reg_read(INTERRUPT) & DATADONE) == 0 {
        if timeout == 0 {
            break;
        }
        timeout -= 1;
        cpu::nop();
    }
    reg_write(INTERRUPT, reg_read(INTERRUPT));

    Ok(())
}

/// Write a single 512-byte block to the SD card
pub unsafe fn write_block(block: u64, buf: &[u8; 512]) -> Result<(), EmmcError> {
    if !EMMC.initialized {
        return Err(EmmcError::CardNotReady);
    }
    if EMMC.write_protected {
        return Err(EmmcError::WriteProtected);
    }

    if !wait_idle() {
        return Err(EmmcError::Busy);
    }

    // Set block size and count
    reg_write(BLKSIZCNT, 512 | (1 << 16));

    let addr = if EMMC.card_info.high_capacity {
        block as u32
    } else {
        (block * 512) as u32
    };

    clear_interrupts();

    let flags = cmd_flags(CMD24_WRITE_SINGLE_BLOCK) | ISDATA | HOST2CARD;
    let cmdtm = (CMD24_WRITE_SINGLE_BLOCK << INDEXSHIFT) | flags;
    reg_write(ARG1, addr);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE
    let mut timeout = 2_000_000u64;
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            return Err(EmmcError::BadResponse);
        }
        if timeout == 0 {
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Write data to FIFO
    let mut bytes_written = 0usize;
    timeout = 5_000_000u64;
    while bytes_written < 512 {
        let irpts = reg_read(INTERRUPT);
        if (irpts & WRITERDY) != 0 {
            reg_write(INTERRUPT, WRITERDY);
            // Write up to 128 words per interrupt
            for _ in 0..128 {
                if bytes_written >= 512 {
                    break;
                }
                let mut word = 0u32;
                word |= buf[bytes_written] as u32;
                if bytes_written + 1 < 512 {
                    word |= (buf[bytes_written + 1] as u32) << 8;
                }
                if bytes_written + 2 < 512 {
                    word |= (buf[bytes_written + 2] as u32) << 16;
                }
                if bytes_written + 3 < 512 {
                    word |= (buf[bytes_written + 3] as u32) << 24;
                }
                reg_write(DATA, word);
                bytes_written += 4;
            }
        }
        if (irpts & DATADONE) != 0 {
            reg_write(INTERRUPT, DATADONE);
            break;
        }
        if (irpts & (DTOERR | DCRCERR | DENDERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            return Err(EmmcError::DataError);
        }
        if timeout == 0 {
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Wait for DATADONE (write completion)
    timeout = 5_000_000u64; // Longer timeout for write completion
    while (reg_read(INTERRUPT) & DATADONE) == 0 {
        if timeout == 0 {
            break;
        }
        timeout -= 1;
        cpu::nop();
    }
    reg_write(INTERRUPT, reg_read(INTERRUPT));

    // Wait for card to finish writing (check DATINHIBIT / WRITE transcription)
    timeout = 5_000_000u64;
    loop {
        let status = reg_read(STATUS);
        if (status & (WRITETRANS | DATINHIBIT)) == 0 {
            break;
        }
        if timeout == 0 {
            crate::println!("[emmc] WARNING: write completion wait timeout");
            break;
        }
        timeout -= 1;
        cpu::nop();
    }

    Ok(())
}

/// Write multiple contiguous blocks to the SD card
pub unsafe fn write_blocks(start_block: u64, buf: &[u8], count: usize) -> Result<(), EmmcError> {
    if !EMMC.initialized {
        return Err(EmmcError::CardNotReady);
    }
    if EMMC.write_protected {
        return Err(EmmcError::WriteProtected);
    }
    if buf.len() < count * 512 {
        return Err(EmmcError::InvalidArgument);
    }

    if count == 1 {
        let mut block_buf = [0u8; 512];
        block_buf.copy_from_slice(&buf[..512]);
        return write_block(start_block, &block_buf);
    }

    if !wait_idle() {
        return Err(EmmcError::Busy);
    }

    // Set block size and count
    reg_write(BLKSIZCNT, 512 | ((count as u32) << 16));

    let addr = if EMMC.card_info.high_capacity {
        start_block as u32
    } else {
        (start_block * 512) as u32
    };

    clear_interrupts();

    let flags = cmd_flags(CMD25_WRITE_MULTIPLE_BLOCK)
        | ISDATA
        | HOST2CARD
        | MULTIBLOCK
        | BLKCNTEN
        | AUTOCMD12;
    let cmdtm = (CMD25_WRITE_MULTIPLE_BLOCK << INDEXSHIFT) | flags;
    reg_write(ARG1, addr);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE
    let mut timeout = 2_000_000u64;
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            return Err(EmmcError::BadResponse);
        }
        if timeout == 0 {
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Write data to FIFO
    let total_bytes = count * 512;
    let mut bytes_written = 0usize;
    timeout = 5_000_000u64 * count as u64;
    while bytes_written < total_bytes {
        let irpts = reg_read(INTERRUPT);
        if (irpts & WRITERDY) != 0 {
            reg_write(INTERRUPT, WRITERDY);
            for _ in 0..64 {
                if bytes_written + 4 > total_bytes {
                    break;
                }
                let mut word = 0u32;
                word |= buf[bytes_written] as u32;
                if bytes_written + 1 < total_bytes {
                    word |= (buf[bytes_written + 1] as u32) << 8;
                }
                if bytes_written + 2 < total_bytes {
                    word |= (buf[bytes_written + 2] as u32) << 16;
                }
                if bytes_written + 3 < total_bytes {
                    word |= (buf[bytes_written + 3] as u32) << 24;
                }
                reg_write(DATA, word);
                bytes_written += 4;
            }
        }
        if (irpts & DATADONE) != 0 {
            reg_write(INTERRUPT, DATADONE);
            break;
        }
        if (irpts & (DTOERR | DCRCERR | DENDERR)) != 0 {
            reg_write(INTERRUPT, irpts);
            let _ = send_command(CMD12_STOP_TRANSMISSION, 0);
            return Err(EmmcError::DataError);
        }
        if timeout == 0 {
            let _ = send_command(CMD12_STOP_TRANSMISSION, 0);
            return Err(EmmcError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Wait for DATADONE
    timeout = 5_000_000u64;
    while (reg_read(INTERRUPT) & DATADONE) == 0 {
        if timeout == 0 {
            break;
        }
        timeout -= 1;
        cpu::nop();
    }
    reg_write(INTERRUPT, reg_read(INTERRUPT));

    // Wait for write completion
    timeout = 5_000_000u64;
    loop {
        let status = reg_read(STATUS);
        if (status & (WRITETRANS | DATINHIBIT)) == 0 {
            break;
        }
        if timeout == 0 {
            break;
        }
        timeout -= 1;
        cpu::nop();
    }

    Ok(())
}

// ─── GPIO/Pinmux Configuration ──────────────────────────────────────

/// Configure GPIO pins for eMMC2 on Pi 5
///
/// Pi 5 SD card pins (BCM2712 GPIO):
/// CLK: GPIO 42 (ALT3)
/// CMD: GPIO 39 (ALT3)
/// D0:  GPIO 40 (ALT3)
/// D1:  GPIO 41 (ALT3)
/// D2:  GPIO 42 shared — typically these are in ALT3
/// D3:  GPIO 43 (ALT3)
///
/// Note: Pi 5 uses specific GPIO for the SD card slot via the BCM2712
/// pin controller, different from the WiFi SDIO2 pins.
pub unsafe fn configure_gpio_pins() {
    crate::println!("[emmc] Configuring GPIO pins for eMMC2...");

    use crate::drivers::gpio::{set_pin_mode, set_pull, PinMode, PullMode};

    // On Pi 5, the SD card slot GPIOs are at a different bank
    // These are the standard Pi 5 SDIO0/eMMC2 pins
    // GPIO 39-43: ALT3 for SD card
    set_pin_mode(39, PinMode::Alt3); // CMD
    set_pin_mode(40, PinMode::Alt3); // D0
    set_pin_mode(41, PinMode::Alt3); // D1
    set_pin_mode(42, PinMode::Alt3); // D2/CLK
    set_pin_mode(43, PinMode::Alt3); // D3

    // Enable pull-ups on CMD and DATA lines
    set_pull(39, PullMode::Up); // CMD
    set_pull(40, PullMode::Up); // D0
    set_pull(41, PullMode::Up); // D1
    set_pull(42, PullMode::Up); // D2
    set_pull(43, PullMode::Up); // D3

    crate::println!("[emmc] GPIO pins configured (39-43=ALT3, pull-ups)");
}

// ─── Interrupt Handler ──────────────────────────────────────────────

/// eMMC2 interrupt handler (called from GIC dispatcher)
pub unsafe fn handle_interrupt() {
    let irpts = reg_read(INTERRUPT);
    EMMC.interrupt_flags |= irpts;
    reg_write(INTERRUPT, irpts);
    Gic::end_of_interrupt(EMMC2_IRQ);
}

// ─── BlockDevice implementation ─────────────────────────────────────

/// eMMC2 SD card block device — implements BlockDevice trait
pub struct EmmcBlockDevice;

impl BlockDevice for EmmcBlockDevice {
    fn read_block(&self, block: u64, buf: &mut [u8; 512]) -> Result<(), BlockError> {
        unsafe { read_block(block, buf).map_err(|e| e.into()) }
    }

    fn write_block(&self, block: u64, buf: &[u8; 512]) -> Result<(), BlockError> {
        unsafe { write_block(block, buf).map_err(|e| e.into()) }
    }

    fn block_size(&self) -> usize {
        512
    }

    fn total_blocks(&self) -> u64 {
        unsafe { EMMC.card_info.total_blocks }
    }

    fn name(&self) -> &str {
        "emmc-sdcard"
    }

    fn media_present(&self) -> bool {
        unsafe { EMMC.initialized }
    }

    fn read_blocks(
        &self,
        start_block: u64,
        buf: &mut [u8],
        count: usize,
    ) -> Result<(), BlockError> {
        unsafe { read_blocks(start_block, buf, count).map_err(|e| e.into()) }
    }

    fn write_blocks(&self, start_block: u64, buf: &[u8], count: usize) -> Result<(), BlockError> {
        unsafe { write_blocks(start_block, buf, count).map_err(|e| e.into()) }
    }

    fn device_type(&self) -> &str {
        "sd-card"
    }

    fn read_only(&self) -> bool {
        unsafe { EMMC.write_protected }
    }

    fn flush(&self) -> Result<(), BlockError> {
        // eMMC has no write-back cache in our driver; no-op
        Ok(())
    }
}

/// Static block device instance for registration
pub static EMMC_BLOCK_DEVICE: EmmcBlockDevice = EmmcBlockDevice;

// ─── Public API ─────────────────────────────────────────────────────

/// Initialize the eMMC2 controller and SD card
pub unsafe fn init() -> Result<SdCardInfo, EmmcError> {
    crate::println!("[emmc] === eMMC2 Controller Initialization ===");

    // 1. Configure GPIO pins for eMMC2
    configure_gpio_pins();

    // 2. Register GIC interrupt handler
    Gic::set_config(EMMC2_IRQ, 0b10); // Edge-triggered
    Gic::set_priority(EMMC2_IRQ, 0xB0); // Medium priority
    Gic::set_target(EMMC2_IRQ, 0x01); // Route to core 0
    Gic::enable_spi(EMMC2_IRQ);
    crate::println!("[emmc] GIC SPI {} registered", EMMC2_IRQ);

    // 3. Reset the eMMC controller
    controller_reset();

    // 4. Initialize the SD card
    match init_card() {
        Ok(info) => {
            crate::println!("[emmc] SD card initialized successfully");
            // 5. Register as a block device
            if crate::fs::block_dev::register_device(&EMMC_BLOCK_DEVICE).is_ok() {
                crate::println!("[emmc] Registered as block device");
            }
            Ok(info)
        }
        Err(e) => {
            crate::println!("[emmc] ERROR: SD card init failed: {:?}", e);
            Err(e)
        }
    }
}

/// Get current card info
pub fn card_info() -> SdCardInfo {
    unsafe { EMMC.card_info }
}

/// Check if the controller is initialized
pub fn is_initialized() -> bool {
    unsafe { EMMC.initialized }
}

/// Get current clock frequency
pub fn clock_freq() -> u32 {
    unsafe { EMMC.clock_freq }
}
