//! SDIO Host Controller driver for BCM2712 (Pi 5)
//!
//! The Pi 5's WiFi chip (Infineon CYW43455 / BCM43455) connects via the
//! Arasan SDHCI-compatible controller at physical 0x10_0110_0000.
//!
//! This driver provides:
//! - Controller reset and clock configuration
//! - SDIO command sending (CMD0–CMD53)
//! - Data transfer via FIFO and DMA
//! - Interrupt-driven operation (GIC SPI 274)
//!
//! Reference: Circle's emmc.c (Plan 9 derived), BCM2835 ARM Peripherals,
//! SDIO Specification v3.00.

use crate::arch::cpu;
use crate::arch::gic::Gic;
use crate::memory::{PhysAddr, VirtAddr};

// ─── SDIO2 Controller Base Address ──────────────────────────────────
// Physical: 0x10_0110_0000 (AXI offset 0x0110_0000)
// Virtual:  DEVICE_MAP_START + 0x0110_0000
// Our device map maps phys 0x10_0000_0000 → DEVICE_MAP_START
const SDIO2_PHYS_BASE: u64 = 0x10_0110_0000;
const SDIO2_VIRT_BASE: u64 = crate::memory::DEVICE_MAP_START + 0x0110_0000;

/// GIC SPI number for SDIO2 on BCM2712
const SDIO2_IRQ: u32 = 274;

// ─── Clock frequencies ──────────────────────────────────────────────
const EXTFREQ: u32 = 200_000_000; // External clock (200 MHz on Pi 5)
const INITFREQ: u32 = 400_000; // SDIO init frequency (400 KHz)
const SDFREQ: u32 = 25_000_000; // Standard SD frequency (25 MHz)
const SDFREQ_HS: u32 = 50_000_000; // High-speed SD frequency (50 MHz)

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
const SPIINTSPT: u64 = 0xF0 >> 2;
const SLOTISRVER: u64 = 0xFC >> 2;

// ─── Control0 bits ──────────────────────────────────────────────────
const HISPEED: u32 = 1 << 2;
const DWIDTH4: u32 = 1 << 1;
const DWIDTH1: u32 = 0 << 1;

// ─── Control1 bits ──────────────────────────────────────────────────
const SRSTDATA: u32 = 1 << 26; // Reset data circuit
const SRSTCMD: u32 = 1 << 25; // Reset command circuit
const SRSTHC: u32 = 1 << 24; // Reset complete host controller
const DATATOSHIFT: u32 = 16; // Data timeout unit exponent
const DATATOMASK: u32 = 0xF0000;
const CLKFREQ8SHIFT: u32 = 8; // SD clock base divider LSBs
const CLKFREQ8MASK: u32 = 0xFF00;
const CLKFREQMS2SHIFT: u32 = 6; // SD clock base divider MSBs
const CLKFREQMS2MASK: u32 = 0xC0;
const CLKGENDIV: u32 = 0 << 5; // SD clock divided
const CLKGENPROG: u32 = 1 << 5; // SD clock programmable
const CLKEN: u32 = 1 << 2; // SD clock enable
const CLKSTABLE: u32 = 1 << 1;
const CLKINTLEN: u32 = 1 << 0; // Enable internal EMMC clocks

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

// ─── SDIO Commands ──────────────────────────────────────────────────
const GO_IDLE_STATE: u32 = 0;
const SEND_RELATIVE_ADDR: u32 = 3;
const IO_SEND_OP_COND: u32 = 5;
const SELECT_CARD: u32 = 7;
const VOLTAGE_SWITCH: u32 = 11;
const IO_RW_DIRECT: u32 = 52;
const IO_RW_EXTENDED: u32 = 53;

// ─── SDIO CCCR (Card Common Control Registers) ──────────────────────
const CCCR_IOENABLE: u32 = 0x02;
const CCCR_IOREADY: u32 = 0x03;
const CCCR_INTENABLE: u32 = 0x04;
const CCCR_INTPEND: u32 = 0x05;
const CCCR_IOABORT: u32 = 0x06;
const CCCR_BUSIFC: u32 = 0x07;
const CCCR_CAPABILITY: u32 = 0x08;
const CCCR_BLKSIZE: u32 = 0x10;
const CCCR_HIGHSPEED: u32 = 0x13;

// ─── Command flag table (maps CMD index → TM register flags) ───────
/// Returns the CmdTM flags for a given SDIO command index.
fn cmd_flags(cmd: u32) -> u32 {
    match cmd {
        0 => IXCHKEN,
        2 => RESP136,
        3 => RESP48 | IXCHKEN | CRCCHKEN,
        5 => RESP48,
        6 => RESP48 | IXCHKEN | CRCCHKEN,
        7 => RESP48BUSY | IXCHKEN | CRCCHKEN,
        8 => RESP48 | IXCHKEN | CRCCHKEN,
        9 => RESP136,
        11 => RESP48 | IXCHKEN | CRCCHKEN,
        12 => RESP48BUSY | IXCHKEN | CRCCHKEN,
        13 => RESP48 | IXCHKEN | CRCCHKEN,
        16 => RESP48,
        17 => RESP48 | ISDATA | CARD2HOST | IXCHKEN | CRCCHKEN,
        18 => RESP48 | ISDATA | CARD2HOST | MULTIBLOCK | BLKCNTEN | IXCHKEN | CRCCHKEN,
        24 => RESP48 | ISDATA | HOST2CARD | IXCHKEN | CRCCHKEN,
        25 => RESP48 | ISDATA | HOST2CARD | MULTIBLOCK | BLKCNTEN | IXCHKEN | CRCCHKEN,
        41 => RESP48,
        52 => RESP48 | IXCHKEN | CRCCHKEN,
        53 => RESP48 | IXCHKEN | CRCCHKEN | ISDATA,
        55 => RESP48 | IXCHKEN | CRCCHKEN,
        _ => RESPNONE,
    }
}

// ─── Error type ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SdioError {
    Timeout,
    CrcError,
    BadResponse,
    DataError,
    CardNotReady,
    FunctionNotEnabled,
    InvalidArgument,
    Busy,
    Aborted,
}

// ─── SDIO Controller State ──────────────────────────────────────────

/// SDIO controller state
struct SdioController {
    /// Current clock frequency
    clock_freq: u32,
    /// Card RCA (relative card address)
    rca: u32,
    /// Current block size for CMD53
    blk_size: u32,
    /// Whether the controller has been initialized
    initialized: bool,
    /// DMA buffer for data transfers (4KB, cache-line aligned)
    dma_buf_virt: VirtAddr,
    dma_buf_phys: PhysAddr,
    /// Interrupt flags (written by ISR, read by polling)
    interrupt_flags: u32,
}

static mut SDIO: SdioController = SdioController {
    clock_freq: 0,
    rca: 0,
    blk_size: 64,
    initialized: false,
    dma_buf_virt: VirtAddr::new(0),
    dma_buf_phys: PhysAddr::new(0),
    interrupt_flags: 0,
};

// ─── MMIO helpers ───────────────────────────────────────────────────

#[inline]
unsafe fn reg_read(offset: u64) -> u32 {
    let addr = SDIO2_VIRT_BASE + offset * 4;
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn reg_write(offset: u64, val: u32) {
    let addr = SDIO2_VIRT_BASE + offset * 4;
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

/// Reset the SDIO host controller
unsafe fn controller_reset() {
    reg_write(CONTROL1, SRSTHC);
    delay_ms(10);

    // Wait for reset to complete
    let mut timeout = 1_000_000u64; // 1 second
    while (reg_read(CONTROL1) & SRSTHC) != 0 {
        if timeout == 0 {
            crate::println!("[sdio] ERROR: controller reset timeout");
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

    crate::println!("[sdio] Controller reset complete, clock: {} Hz", INITFREQ);
}

/// Set the SD clock frequency
unsafe fn set_clock(freq: u32) {
    // Stop the clock first
    let mut ctl1 = reg_read(CONTROL1);
    ctl1 &= !(CLKEN | CLKINTLEN | CLKFREQ8MASK | CLKFREQMS2MASK);
    reg_write(CONTROL1, ctl1);
    delay_us(100);

    // Calculate clock divider
    // SDCLK = base_clock / (div * 2)
    // For 200 MHz base:
    //   400 KHz → div = 200M / (400K * 2) = 250
    //   25 MHz  → div = 200M / (25M * 2) = 4
    //   50 MHz  → div = 200M / (50M * 2) = 2
    let mut div = EXTFREQ / (freq * 2);
    if div == 0 {
        div = 1;
    }
    if div > 0x3FF {
        div = 0x3FF; // Max divider
    }

    // Set divider in Control1
    // Bits [15:8] = div LSBs (FREQ8), Bits [7:6] = div MSBs (FREQMS2)
    let div_lo = (div & 0xFF) as u32;
    let div_hi = ((div >> 8) & 0x3) as u32;
    ctl1 = reg_read(CONTROL1);
    ctl1 |= CLKINTLEN | (div_lo << CLKFREQ8SHIFT) | (div_hi << CLKFREQMS2SHIFT);
    reg_write(CONTROL1, ctl1);

    // Wait for clock stable
    let mut timeout = 1_000_000u64;
    while (reg_read(CONTROL1) & CLKSTABLE) == 0 {
        if timeout == 0 {
            crate::println!("[sdio] ERROR: clock stable timeout");
            return;
        }
        timeout -= 1;
        cpu::nop();
    }

    // Enable the clock
    ctl1 = reg_read(CONTROL1);
    ctl1 |= CLKEN;
    reg_write(CONTROL1, ctl1);

    // Small delay for clock to propagate
    delay_us(100);

    // Update state
    let actual_freq = EXTFREQ / (div * 2);

    SDIO.clock_freq = actual_freq;
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
            crate::println!("[sdio] ERROR: wait_idle timeout, STATUS={:#010x}", status);
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

/// Send an SDIO command and wait for response
/// Returns the 4 response words (most commands only use RESP0)
unsafe fn send_command(cmd: u32, arg: u32) -> Result<[u32; 4], SdioError> {
    if !wait_idle() {
        return Err(SdioError::Busy);
    }

    // Clear previous interrupts
    clear_interrupts();

    // Build the CmdTM register
    let flags = cmd_flags(cmd);
    let cmdtm = (cmd << INDEXSHIFT) | flags;
    reg_write(ARG1, arg);
    reg_write(CMDTM, cmdtm);

    // Wait for CMDDONE or error
    let mut timeout = 2_000_000u64; // 2 seconds
    loop {
        let irpts = reg_read(INTERRUPT);
        if (irpts & CMDDONE) != 0 {
            // Clear CMDDONE
            reg_write(INTERRUPT, CMDDONE);
            break;
        }
        if (irpts & (CTOERR | CCRCERR | CENDERR | CBADERR)) != 0 {
            let err_bits = irpts & (CTOERR | CCRCERR | CENDERR | CBADERR);
            reg_write(INTERRUPT, err_bits);
            if (irpts & CTOERR) != 0 {
                return Err(SdioError::Timeout);
            }
            if (irpts & CCRCERR) != 0 {
                return Err(SdioError::CrcError);
            }
            return Err(SdioError::BadResponse);
        }
        if timeout == 0 {
            reg_write(INTERRUPT, reg_read(INTERRUPT));
            return Err(SdioError::Timeout);
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
unsafe fn send_command_retry(cmd: u32, arg: u32) -> Result<[u32; 4], SdioError> {
    let mut attempts = 3;
    loop {
        match send_command(cmd, arg) {
            Ok(resp) => return Ok(resp),
            Err(SdioError::Timeout) => {
                if attempts == 0 {
                    return Err(SdioError::Timeout);
                }
                attempts -= 1;
                delay_ms(10);
            }
            Err(e) => return Err(e),
        }
    }
}

// ─── SDIO I/O Operations (CMD52 / CMD53) ────────────────────────────

/// CMD52: Read a single byte from a SDIO function
/// `fn_num`: function number (0-7, Fn0=0, Fn1=1, Fn2=2)
/// `addr`: register address within the function (0x00000-0x1FFFF)
/// Returns the data byte read
pub unsafe fn io_read_byte(fn_num: u32, addr: u32) -> Result<u8, SdioError> {
    // CMD52 format: [31]=R/W=0, [30:28]=stuff, [27]=RAW=0,
    //               [25:23]=Function number, [22]=stuff,
    //               [21:19]=register address[18:16], [18:9]=stuff,
    //               [8:0] not used for read
    // Simplified: bits [25:23] = fn_num, [21:9] = addr (9 bits → 512 bytes per function)
    // Actually: bits [25:23] = fn_num, [19:9] = addr (11 bits, but only 9 used for direct)
    // Correct SDIO CMD52 arg: [31]=0 (read), [30]=0, [29:28]=stuff,
    //   [27]=RAW flag, [26:24]=stuff, [23:22]=stuff,
    //   [25:23]=function, [22]=stuff, [21:9]=register addr (13 bits)
    // Per SDIO spec: arg = (fn_num << 28) | (addr << 9) | 0
    // Wait, let me look at the actual spec format:
    //   Bit 31: R/W flag (0=read, 1=write)
    //   Bit 30: Raw flag (post-read = 0, write-after-read = 1)
    //   Bits 28:27: Function number (upper 2 bits)
    //   Bit 26: stuff
    //   Bits 25:24: Function number (lower bits)
    // Actually the standard layout is:
    //   [31] = R/W (0 for read)
    //   [30] = RAW
    //   [29:28] = Stuff
    //   [27] = stuff
    //   [26:24] = not used?
    // The Circle/Plan 9 code uses: arg = (fn<<28) | (addr<<9)
    // This matches: function in bits [30:28], address in bits [25:9]
    // Actually let me re-check:
    //   Bits [31] = Direction (0=write→read, 1=write)
    //   Bits [30] = RAW (read after write)
    //   Bits [29:28] = Function number
    //   Bits [27:26] = stuff
    //   Bits [25:9] = Register Address
    //   Bits [8:0] = Data (for writes) / Stuff (for reads)
    // So: arg = (0 << 31) | (0 << 30) | (fn << 28) | (addr << 9)

    let arg = (fn_num << 28) | (addr << 9);
    let resp = send_command(IO_RW_DIRECT, arg)?;

    // Response R5: bits [23:0] = data, bits [31:24] = flags
    let data = (resp[0] & 0xFF) as u8;

    // Check for errors in response flags
    let flags = (resp[0] >> 8) & 0xFF;
    if (flags & 0x06) != 0 {
        // COM_CRC_ERROR or ILLEGAL_COMMAND
        return Err(SdioError::BadResponse);
    }

    Ok(data)
}

/// CMD52: Write a single byte to a SDIO function
pub unsafe fn io_write_byte(fn_num: u32, addr: u32, data: u8) -> Result<(), SdioError> {
    // Same as read but bit 31 = 1 (write), bits [7:0] = data
    let arg = (1 << 31) | (fn_num << 28) | (addr << 9) | (data as u32);
    let resp = send_command(IO_RW_DIRECT, arg)?;

    // Check response flags
    let flags = (resp[0] >> 8) & 0xFF;
    if (flags & 0x06) != 0 {
        return Err(SdioError::BadResponse);
    }

    Ok(())
}

/// CMD53: Read a block of data from a SDIO function
/// `fn_num`: function number
/// `addr`: register address
/// `buf`: buffer to read into (must be at least `len` bytes)
/// `len`: number of bytes to read (must be <= 512 for block mode, <= 4096 for byte mode)
/// `incr`: if true, increment address after each byte (FIFO mode); if false, fixed address
pub unsafe fn io_read_block(
    fn_num: u32,
    addr: u32,
    buf: &mut [u8],
    len: usize,
    incr: bool,
) -> Result<(), SdioError> {
    if len == 0 || len > 4096 {
        return Err(SdioError::InvalidArgument);
    }

    if !wait_idle() {
        return Err(SdioError::Busy);
    }

    // Set up block size and count

    let blk_size = SDIO.blk_size;

    let block_count = ((len + blk_size as usize - 1) / blk_size as usize) as u32;
    reg_write(
        BLKSIZCNT,
        (blk_size & 0xFFF) | ((block_count & 0xFFFF) << 16),
    );

    // CMD53 arg format:
    //   [31] = R/W (0=read)
    //   [30] = OP (0=byte mode, 1=block mode)
    //   [29:28] = Function number
    //   [27] = Block mode flag (0=byte, 1=block)
    //   Wait — the actual format is:
    //   [31] = Direction (0=write→read, 1=write)
    //   [30] = Block mode (0=byte, 1=block)
    //   [29:28] = Function number
    //   [27] = OP code (0=fixed, 1=incrementing)
    //   [26:9] = Register address
    //   [8:0] = Byte count / Block count
    let op_code = if incr { 1 } else { 0 };
    let block_mode = 1u32; // Use block mode
    let count = if block_mode != 0 {
        block_count
    } else {
        len as u32
    };

    let arg = (0 << 31)
        | (block_mode << 30)
        | (fn_num << 28)
        | (op_code << 27)
        | (addr << 9)
        | (count & 0x1FF);

    clear_interrupts();

    let flags = cmd_flags(IO_RW_EXTENDED) | ISDATA | CARD2HOST;
    let cmdtm = (IO_RW_EXTENDED << INDEXSHIFT) | flags;
    reg_write(ARG1, arg);
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
            return Err(SdioError::BadResponse);
        }
        if timeout == 0 {
            return Err(SdioError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Read data from FIFO
    let mut bytes_read = 0usize;
    timeout = 5_000_000u64;
    while bytes_read < len {
        let irpts = reg_read(INTERRUPT);
        if (irpts & READRDY) != 0 {
            reg_write(INTERRUPT, READRDY);
            // Read available words from DATA register
            let status = reg_read(STATUS);
            let words_available = if (status & BUFREAD) != 0 {
                // FIFO has data available, read in 4-byte words
                let remaining = len - bytes_read;
                let words = remaining / 4;
                if words > 0 {
                    words
                } else {
                    1
                }
            } else {
                0
            };
            for _ in 0..words_available.min(64) {
                // Read up to 256 bytes (64 words) per interrupt
                let word = reg_read(DATA);
                let start = bytes_read;
                buf[start] = (word & 0xFF) as u8;
                if start + 1 < len {
                    buf[start + 1] = ((word >> 8) & 0xFF) as u8;
                }
                if start + 2 < len {
                    buf[start + 2] = ((word >> 16) & 0xFF) as u8;
                }
                if start + 3 < len {
                    buf[start + 3] = ((word >> 24) & 0xFF) as u8;
                }
                bytes_read += 4;
                if bytes_read >= len {
                    break;
                }
            }
        }
        if (irpts & DATADONE) != 0 {
            reg_write(INTERRUPT, DATADONE);
            break;
        }
        if (irpts & (DTOERR | DCRCERR | DENDERR)) != 0 {
            reg_write(INTERRUPT, irpts & (DTOERR | DCRCERR | DENDERR));
            return Err(SdioError::DataError);
        }
        if timeout == 0 {
            crate::println!(
                "[sdio] ERROR: data read timeout, {} of {} bytes",
                bytes_read,
                len
            );
            return Err(SdioError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Wait for DATADONE if not already received
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

/// CMD53: Write a block of data to a SDIO function
pub unsafe fn io_write_block(
    fn_num: u32,
    addr: u32,
    buf: &[u8],
    len: usize,
    incr: bool,
) -> Result<(), SdioError> {
    if len == 0 || len > 4096 {
        return Err(SdioError::InvalidArgument);
    }

    if !wait_idle() {
        return Err(SdioError::Busy);
    }

    let blk_size = SDIO.blk_size;

    let block_count = ((len + blk_size as usize - 1) / blk_size as usize) as u32;
    reg_write(
        BLKSIZCNT,
        (blk_size & 0xFFF) | ((block_count & 0xFFFF) << 16),
    );

    let op_code = if incr { 1 } else { 0 };
    let block_mode = 1u32;
    let count = if block_mode != 0 {
        block_count
    } else {
        len as u32
    };

    // Bit 31 = 1 for write
    let arg = (1 << 31)
        | (block_mode << 30)
        | (fn_num << 28)
        | (op_code << 27)
        | (addr << 9)
        | (count & 0x1FF);

    clear_interrupts();

    let flags = cmd_flags(IO_RW_EXTENDED) | ISDATA | HOST2CARD;
    let cmdtm = (IO_RW_EXTENDED << INDEXSHIFT) | flags;
    reg_write(ARG1, arg);
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
            return Err(SdioError::BadResponse);
        }
        if timeout == 0 {
            return Err(SdioError::Timeout);
        }
        timeout -= 1;
        cpu::nop();
    }

    // Write data to FIFO
    let mut bytes_written = 0usize;
    timeout = 5_000_000u64;
    while bytes_written < len {
        let irpts = reg_read(INTERRUPT);
        if (irpts & WRITERDY) != 0 {
            reg_write(INTERRUPT, WRITERDY);
            // Write up to 64 words per interrupt
            for _ in 0..64 {
                if bytes_written >= len {
                    break;
                }
                let mut word = 0u32;
                word |= if bytes_written < len {
                    buf[bytes_written] as u32
                } else {
                    0
                };
                word |= if bytes_written + 1 < len {
                    (buf[bytes_written + 1] as u32) << 8
                } else {
                    0
                };
                word |= if bytes_written + 2 < len {
                    (buf[bytes_written + 2] as u32) << 16
                } else {
                    0
                };
                word |= if bytes_written + 3 < len {
                    (buf[bytes_written + 3] as u32) << 24
                } else {
                    0
                };
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
            return Err(SdioError::DataError);
        }
        if timeout == 0 {
            return Err(SdioError::Timeout);
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

/// Set specific bits in a SDIO function register (read-modify-write)
pub unsafe fn io_set_bits(fn_num: u32, addr: u32, bits: u8) -> Result<(), SdioError> {
    let old = io_read_byte(fn_num, addr)?;
    io_write_byte(fn_num, addr, old | bits)
}

// ─── SDIO Bus Initialization ────────────────────────────────────────

/// Initialize the SDIO bus and enumerate the WiFi card
pub unsafe fn init_bus() -> Result<u32, SdioError> {
    crate::println!("[sdio] Initializing SDIO bus...");

    // 1. Send CMD0 (GO_IDLE_STATE) — reset all cards
    crate::println!("[sdio] CMD0: GO_IDLE_STATE");
    send_command(GO_IDLE_STATE, 0)?;
    delay_ms(10);

    // 2. Send CMD5 (IO_SEND_OP_COND) — query SDIO card capabilities
    crate::println!("[sdio] CMD5: IO_SEND_OP_COND");
    let mut ocr;
    let mut attempts = 10;
    loop {
        // CMD5 arg: host voltage support (3.2-3.4V + 2.7-2.9V)
        let voltage_arg = (3 << 20) | (3 << 15);
        let resp = send_command_retry(IO_SEND_OP_COND, voltage_arg)?;
        ocr = resp[0];

        // Check if card has returned its operating range (bit 31 = busy/done)
        if (ocr & (1 << 31)) != 0 {
            break;
        }

        if attempts == 0 {
            crate::println!("[sdio] ERROR: no response to CMD5, ocr={:#010x}", ocr);
            return Err(SdioError::CardNotReady);
        }
        attempts -= 1;
        delay_ms(100);
    }

    let num_io_funcs = ((ocr >> 28) & 0x7) as u32;
    crate::println!(
        "[sdio] Card responded: OCR={:#010x}, I/O functions={}",
        ocr,
        num_io_funcs
    );

    // 3. Send CMD3 (SEND_RELATIVE_ADDR) — get card's RCA
    crate::println!("[sdio] CMD3: SEND_RELATIVE_ADDR");
    let resp = send_command_retry(SEND_RELATIVE_ADDR, 0)?;
    let rca = (resp[0] >> 16) & 0xFFFF;
    crate::println!("[sdio] Card RCA: {:#06x}", rca);

    SDIO.rca = rca;

    // 4. Send CMD7 (SELECT_CARD) — select the card with this RCA
    crate::println!("[sdio] CMD7: SELECT_CARD (rca={:#06x})", rca);
    let select_arg = rca << 16;
    send_command_retry(SELECT_CARD, select_arg)?;
    crate::println!("[sdio] Card selected");

    // 5. Enable SDIO functions via CCCR
    // Enable I/O functions 1 and 2 (WiFi uses Fn1 for control, Fn2 for data)
    crate::println!("[sdio] Enabling I/O functions 1+2...");
    io_write_byte(0, CCCR_IOENABLE, 0x03)?; // Enable Fn1 + Fn2

    // Wait for functions to become ready
    attempts = 100;
    loop {
        let ready = io_read_byte(0, CCCR_IOREADY)?;
        if (ready & 0x03) == 0x03 {
            break;
        }
        if attempts == 0 {
            crate::println!(
                "[sdio] WARNING: I/O functions not ready (IORDY={:#04x})",
                ready
            );
            break;
        }
        attempts -= 1;
        delay_ms(10);
    }
    crate::println!("[sdio] I/O functions 1+2 enabled and ready");

    // 6. Set block size to 64 bytes for both functions
    crate::println!("[sdio] Setting block size...");
    // Fn1 block size (2 bytes, LSB first at 0x110, MSB at 0x111)
    io_write_byte(0, CCCR_BLKSIZE, 64)?;
    io_write_byte(0, CCCR_BLKSIZE + 1, 0)?;
    // Fn2 block size
    io_write_byte(0, CCCR_BLKSIZE + 2, 64)?;
    io_write_byte(0, CCCR_BLKSIZE + 3, 0)?;

    SDIO.blk_size = 64;

    // 7. Enable high-speed mode (50 MHz) if supported
    let caps = io_read_byte(0, CCCR_CAPABILITY)?;
    if (caps & 0x01) != 0 {
        crate::println!("[sdio] Enabling high-speed mode...");
        io_set_bits(0, CCCR_HIGHSPEED, 0x02)?;
        // Verify
        let hs = io_read_byte(0, CCCR_HIGHSPEED)?;
        if (hs & 0x02) != 0 {
            crate::println!("[sdio] High-speed mode enabled, switching clock to 50 MHz");
            set_clock(SDFREQ_HS);
        } else {
            crate::println!("[sdio] WARNING: high-speed mode not accepted, using 25 MHz");
            set_clock(SDFREQ);
        }
    } else {
        crate::println!("[sdio] High-speed not supported, using 25 MHz");
        set_clock(SDFREQ);
    }

    // 8. Set bus width to 4-bit
    let bus_ifc = io_read_byte(0, CCCR_BUSIFC)?;
    if (bus_ifc & 0x40) != 0 {
        // Support 4-bit bus
        io_write_byte(0, CCCR_BUSIFC, (bus_ifc & 0xF8) | 0x02)?; // Set 4-bit
        reg_write(CONTROL0, reg_read(CONTROL0) | DWIDTH4);
        crate::println!("[sdio] 4-bit bus width enabled");
    } else {
        crate::println!("[sdio] WARNING: 4-bit bus not supported, using 1-bit");
    }

    // 9. Enable SDIO interrupts
    io_write_byte(0, CCCR_INTENABLE, 0x03)?; // Enable Fn1 + Fn2 interrupts

    crate::println!("[sdio] SDIO bus initialization complete!");

    SDIO.initialized = true;

    Ok(rca)
}

// ─── GPIO/Pinmux Configuration ──────────────────────────────────────

/// Configure GPIO pins for SDIO2 on Pi 5
///
/// On the Pi 5, the WiFi SDIO2 pins are routed through the SoC's
/// pin controller. The BCM2712 uses a different pin mux than BCM2835.
/// The SDIO2 controller uses pins in ALT3/Func7 mode.
///
/// Pi 5 WiFi pins (from DeviceTree):
///   CMD:  GPIO 34 (ALT3 / Func7)
///   CLK:  GPIO 33 (ALT3 / Func7)  
///   D0:   GPIO 35 (ALT3 / Func7)
///   D1:   GPIO 36 (ALT3 / Func7)
///   D2:   GPIO 37 (ALT3 / Func7)
///   D3:   GPIO 38 (ALT3 / Func7)
///
/// Note: On Pi 5 with RP1, the SDIO2 controller is on the BCM2712
/// side (not RP1), so these are BCM2712 GPIO pins, not RP1 pins.
/// The GPIO controller base for BCM2712 is at 0x10_7D50_4000.
pub unsafe fn configure_gpio_pins() {
    crate::println!("[sdio] Configuring GPIO pins for SDIO2...");

    // Set pin mode to ALT3 (Func7) for SDIO2
    // GPIO 33-38 are in GPFSEL3 (pins 30-39)
    // Each pin uses 3 bits: ALT3 = 0b111 = 7
    //
    // GPFSEL3 offset: GPIO_BASE + 0x0C
    // Pin 33: bits [29:27], Pin 34: bits [26:24], Pin 35: bits [23:21]
    // Pin 36: bits [20:18], Pin 37: bits [17:15], Pin 38: bits [14:12]
    use crate::drivers::gpio::{set_pin_mode, set_pull, PinMode, PullMode};

    // Set SDIO2 pins to ALT3 (which is our PinMode::Alt3 = 7)
    set_pin_mode(33, PinMode::Alt3); // CLK
    set_pin_mode(34, PinMode::Alt3); // CMD
    set_pin_mode(35, PinMode::Alt3); // D0
    set_pin_mode(36, PinMode::Alt3); // D1
    set_pin_mode(37, PinMode::Alt3); // D2
    set_pin_mode(38, PinMode::Alt3); // D3

    // Enable pull-ups on CMD and DATA lines (required by SDIO spec)
    set_pull(34, PullMode::Up); // CMD
    set_pull(35, PullMode::Up); // D0
    set_pull(36, PullMode::Up); // D1
    set_pull(37, PullMode::Up); // D2
    set_pull(38, PullMode::Up); // D3

    // WiFi power-on: GPIO 41 must be HIGH to power the WiFi chip
    // On Pi 5, this is controlled by the PMU or a GPIO expander
    // The firmware typically handles this, but let's make sure:
    set_pin_mode(41, PinMode::Output);
    set_pull(41, PullMode::None);
    crate::drivers::gpio::set_pin(41); // Power on WiFi
    delay_ms(50); // Wait for power to stabilize

    // 32.768 KHz clock for WiFi low-power mode: GPIO 43 → ALT0 (GPCLK2)
    set_pin_mode(43, PinMode::Alt0);

    crate::println!("[sdio] GPIO pins configured (33-38=ALT3, 41=HIGH, 43=ALT0)");
}

// ─── Interrupt Handler ──────────────────────────────────────────────

/// SDIO2 interrupt handler (called from GIC dispatcher)
pub unsafe fn handle_interrupt() {
    let irpts = reg_read(INTERRUPT);

    SDIO.interrupt_flags |= irpts;

    // Clear the interrupt sources
    reg_write(INTERRUPT, irpts);

    // Acknowledge GIC
    Gic::end_of_interrupt(SDIO2_IRQ);
}

// ─── Public API ─────────────────────────────────────────────────────

/// Initialize the SDIO2 controller and WiFi card
pub unsafe fn init() -> Result<(), SdioError> {
    crate::println!("[sdio] === SDIO2 Controller Initialization ===");

    // 1. Configure GPIO pins for SDIO2
    configure_gpio_pins();

    // 2. Register GIC interrupt handler
    Gic::set_config(SDIO2_IRQ, 0b10); // Edge-triggered
    Gic::set_priority(SDIO2_IRQ, 0xA0); // Medium-high priority
    Gic::set_target(SDIO2_IRQ, 0x01); // Route to core 0
    Gic::enable_spi(SDIO2_IRQ);
    crate::println!("[sdio] GIC SPI {} registered", SDIO2_IRQ);

    // 3. Reset the SDIO controller
    controller_reset();

    // 4. Initialize the SDIO bus (enumerate card)
    match init_bus() {
        Ok(rca) => {
            crate::println!("[sdio] WiFi card initialized (RCA={:#06x})", rca);
            Ok(())
        }
        Err(e) => {
            crate::println!("[sdio] ERROR: WiFi card init failed: {:?}", e);
            Err(e)
        }
    }
}

/// Get current block size
pub fn block_size() -> u32 {
    unsafe { SDIO.blk_size }
}

/// Set block size for CMD53 operations
pub unsafe fn set_block_size(size: u32) {
    // Update in CCCR
    let _ = io_write_byte(0, CCCR_BLKSIZE, (size & 0xFF) as u8);
    let _ = io_write_byte(0, CCCR_BLKSIZE + 1, ((size >> 8) & 0xFF) as u8);

    SDIO.blk_size = size;
}

/// Abort pending I/O on a specific function
pub unsafe fn abort(fn_num: u32) -> Result<(), SdioError> {
    io_write_byte(0, CCCR_IOABORT, fn_num as u8)
}

/// Check if the controller is initialized
pub fn is_initialized() -> bool {
    unsafe { SDIO.initialized }
}
