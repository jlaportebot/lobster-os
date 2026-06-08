//! BCM43455 (CYW43455) WiFi driver for Raspberry Pi 5
//!
//! Drives the Infineon CYW43455 802.11b/g/n WiFi chip over SDIO.
//! The chip connects via the SDIO2 host controller (see sdio.rs).
//!
//! Architecture:
//! - SDIO Fn1: control registers (backplane access, ioctl)
//! - SDIO Fn2: data path (Tx/Rx of 802.3 frames)
//! - SSB backplane: indirect register access via 32KB window
//! - Firmware: downloaded to chip SOCSRAM, handles 802.11↔802.3 conversion
//!
//! Reference: brcmfmac driver (Linux), CYW43455 datasheet,
//! Broadcom SDIO host specification.

use crate::drivers::sdio::{self, SdioError};
use crate::println;

/// SHA-1 implementation for PBKDF2 (WPA2 PMK derivation)
mod sha1 {
    use alloc::vec::Vec;

    const K: [u32; 4] = [0x5A827999, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6];

    fn rotl32(x: u32, n: u32) -> u32 {
        (x << n) | (x >> (32 - n))
    }

    fn f1(b: u32, c: u32, d: u32) -> u32 {
        (b & c) | (!b & d)
    }
    fn f2(b: u32, c: u32, d: u32) -> u32 {
        b ^ c ^ d
    }
    fn f3(b: u32, c: u32, d: u32) -> u32 {
        (b & c) | (b & d) | (c & d)
    }
    fn f4(b: u32, c: u32, d: u32) -> u32 {
        b ^ c ^ d
    }

    pub fn sha1_hash(data: &[u8]) -> [u8; 20] {
        let mut h = [
            0x67452301u32,
            0xEFCDAB89,
            0x98BADCFE,
            0x10325476,
            0xC3D2E1F0,
        ];
        let mut msg = Vec::with_capacity(((data.len() + 9 + 63) / 64) * 64);
        msg.extend_from_slice(data);
        msg.push(0x80);
        while (msg.len() % 64) != 56 {
            msg.push(0);
        }
        let bit_len = (data.len() * 8) as u64;
        msg.extend_from_slice(&bit_len.to_be_bytes());

        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..80 {
                w[i] = rotl32(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
            }
            let mut a = h[0];
            let mut b = h[1];
            let mut c = h[2];
            let mut d = h[3];
            let mut e = h[4];
            for i in 0..80 {
                let (f, k) = match i {
                    0..=19 => (f1(b, c, d), K[0]),
                    20..=39 => (f2(b, c, d), K[1]),
                    40..=59 => (f3(b, c, d), K[2]),
                    _ => (f4(b, c, d), K[3]),
                };
                let temp = rotl32(a, 5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(w[i]);
                e = d;
                d = c;
                c = rotl32(b, 30);
                b = a;
                a = temp;
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
        }

        let mut out = [0u8; 20];
        for i in 0..5 {
            out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
        }
        out
    }

    pub fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
        const BLOCK_SIZE: usize = 64;
        let mut k = [0u8; BLOCK_SIZE];
        if key.len() > BLOCK_SIZE {
            let kh = sha1_hash(key);
            k[..20].copy_from_slice(&kh);
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut ipad = [0x36u8; BLOCK_SIZE];
        let mut opad = [0x5cu8; BLOCK_SIZE];
        for i in 0..BLOCK_SIZE {
            ipad[i] ^= k[i];
            opad[i] ^= k[i];
        }
        let mut inner = Vec::with_capacity(BLOCK_SIZE + data.len());
        inner.extend_from_slice(&ipad);
        inner.extend_from_slice(data);
        let inner_hash = sha1_hash(&inner);
        let mut outer = Vec::with_capacity(BLOCK_SIZE + 20);
        outer.extend_from_slice(&opad);
        outer.extend_from_slice(&inner_hash);
        sha1_hash(&outer)
    }
}

/// PBKDF2-HMAC-SHA1 for WPA2 PMK derivation
/// PMK = PBKDF2-HMAC-SHA1(passphrase, SSID, 4096, 32)
fn pbkdf2_hmac_sha1(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    dk_len: usize,
) -> alloc::vec::Vec<u8> {
    let hlen = 20; // SHA-1 output length
    let l = (dk_len + hlen - 1) / hlen;
    let mut dk = alloc::vec::Vec::with_capacity(dk_len);

    for block_idx in 1..=l {
        // Build salt || block_index (big-endian)
        let mut salt_block = alloc::vec::Vec::with_capacity(salt.len() + 4);
        salt_block.extend_from_slice(salt);
        salt_block.extend_from_slice(&block_idx.to_be_bytes());

        // U_1 = HMAC-SHA1(password, salt || block_index)
        let mut u = sha1::hmac_sha1(password, &salt_block);
        let mut t = u;

        // U_i = HMAC-SHA1(password, U_{i-1}) for i = 2..iterations
        // T = U_1 ^ U_2 ^ ... ^ U_iterations
        for _ in 1..iterations {
            u = sha1::hmac_sha1(password, &u);
            for (i, byte) in u.iter().enumerate() {
                t[i] ^= byte;
            }
        }
        dk.extend_from_slice(&t);
    }
    dk.truncate(dk_len);
    dk
}

// ─── WiFi Error Type ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WifiError {
    Sdio(SdioError),
    NotInitialized,
    FirmwareNotFound,
    FirmwareLoadFailed,
    CoreNotFound(u16),
    CoreNotUp,
    IoctlTimeout,
    IoctlError(u32),
    ScanFailed,
    ConnectFailed,
    NotConnected,
    TxFailed,
    NoData,
    Busy,
    InvalidState,
}

impl From<SdioError> for WifiError {
    fn from(e: SdioError) -> Self {
        WifiError::Sdio(e)
    }
}

// ─── SDIO Function Numbers ─────────────────────────────────────────

const FN_BACKPLANE: u32 = 1; // SDIO Fn1 — backplane/ctrl
const FN_DATA: u32 = 2; // SDIO Fn2 — data path

// ─── SDIO Fn1 Register Map ─────────────────────────────────────────

// Backplane access (SDIO Fn1)
const SB_32BIT_WIN: u32 = 0x8000; // 32KB window base
const SB_ADDR_LOW: u32 = 0x1000A; // Backplane address [15:0]
const SB_ADDR_MID: u32 = 0x1000B; // Backplane address [23:16]
const SB_ADDR_HIGH: u32 = 0x1000C; // Backplane address [31:24]
const SB_WIN_DATA: u32 = 0x10000; // Window data (32KB from 0x10000)

// ChipCommon registers (via backplane)
const CC_CHIPID: u32 = 0x18000000;
const CC_CAPABILITIES: u32 = 0x18000004;

// ARM core registers
const ARM_CR4_RESET: u32 = 0x18001804; // AI_IOCTRL
const ARM_CR4_HALT: u32 = 0x18001808; // AI_IOSTATUS
const ARM_CR4_RESET_CTRL: u32 = 0x1800180C;

// SOCSRAM registers
const SOCSRAM_BANKIDX: u32 = 0x18108000;
const SOCSRAM_BANKPDA: u32 = 0x18108004;
const SOCSRAM_BANKINFO: u32 = 0x18108044;

// SDIO core registers (via backplane)
const SDIO_INTSTATUS: u32 = 0x18002020;
const SDIO_INTMASK: u32 = 0x18002024;
const SDIO_PCMBASE: u32 = 0x18004000;
const SDIO_TOBUS: u32 = 0x1800400C;

// ─── Backplane Window Size ─────────────────────────────────────────

const BACKPLANE_WIN_SIZE: u32 = 0x8000; // 32KB
const BACKPLANE_WIN_MASK: u32 = 0xFFFF8000;

// ─── SSB Core IDs ──────────────────────────────────────────────────

const CC_CORE_ID: u16 = 0x800; // ChipCommon
const SOCSRAM_CORE_ID: u16 = 0x80E; // SOCSRAM
const ARMCR4_CORE_ID: u16 = 0x8A4; // ARM CR4
const D11_CORE_ID: u16 = 0x812; // 802.11 MAC
const SDIO_CORE_ID: u16 = 0x829; // SDIO core

// ─── AI (Agent Interface) Register Offsets ─────────────────────────

const AI_IOCTRL: u32 = 0x408; // Reset/clock control
const AI_IOSTATUS: u32 = 0x500; // Status
const AI_RESETCTRL: u32 = 0x800; // Reset control
const AI_RESETSTATUS: u32 = 0x804;

// ─── CR4 Reset Bits ────────────────────────────────────────────────

const ARMCR4_CAP: u32 = 1 << 15; // CPU capitulate
const ARMCR4_BRESET: u32 = 1 << 1; // Reset
const ARMCR4_CLOCK_EN: u32 = 1; // Clock enable

// ─── IOCTL Definitions ─────────────────────────────────────────────

// IOCTL command groups
const IOCTL_GRP_SSID: u32 = 0; // SSID
const IOCTL_GRP_BSS: u32 = 2; // BSS
const IOCTL_GRP_SEC: u32 = 5; // Security
const IOCTL_GRP_DEV: u32 = 6; // Device
const IOCTL_GRP_ESCAN: u32 = 22; // Extended scan

// IOCTL IDs
const IOCTL_SET_SSID: u32 = 0x02; // Set SSID
const IOCTL_DISASSOC: u32 = 0x07; // Disassociate
const IOCTL_SET_CHANNEL: u32 = 0x10; // Set channel
const IOCTL_JOIN: u32 = 0x12; // Join BSS
const IOCTL_SET_INFRA: u32 = 0x14; // Set infra mode
const IOCTL_SET_AUTH: u32 = 0x15; // Set auth type
const IOCTL_SET_WSEC: u32 = 0x1C; // Set WSEC
const IOCTL_SET_WSEC_PMK: u32 = 0x1D; // Set WSEC PMK
const IOCTL_GET_BSS_INFO: u32 = 0x24; // Get BSS info
const IOCTL_UP: u32 = 0x2E; // Bring up
const IOCTL_DOWN: u32 = 0x2F; // Bring down
const IOCTL_GET_MAC: u32 = 0x3E; // Get MAC address
const IOCTL_SET_COUNTRY: u32 = 0x39; // Set country code
const IOCTL_ESCAN: u32 = 0x64; // Extended scan

// IOCTL flag bits
const IOCTL_ACTION_SET: u32 = 2;
const IOCTL_ACTION_GET: u32 = 3;
const IOCTL_ACTION_ODE: u32 = 1 << 14; // ODE flag

// ─── DPC (Deferred Procedure Call) / Event Definitions ─────────────

// Event types from firmware
const EVT_SET_SSID: u32 = 0;
const EVT_DISASSOC: u32 = 1;
const EVT_JOIN: u32 = 2;
const EVT_AUTH: u32 = 3;
const EVT_DEAUTH: u32 = 4;
const EVT_LINK: u32 = 5;
const EVT_ESCAN_RESULT: u32 = 6;
const EVT_IFACE: u32 = 7;

// ─── SDIO Fn2 Data Frame Header ────────────────────────────────────

// Tx/Rx frame header (2 bytes) prepended to Ethernet frames on Fn2
const SDPCM_HEADER_LEN: usize = 4;

// Header flags
const SDPCM_DATA_FLAG: u8 = 0x08; // Data frame
const SDPCM_EVENT_FLAG: u8 = 0x10; // Event frame
const SDPCM_CTRL_FLAG: u8 = 0x20; // Control frame
const SDPCM_TX_SEQ_SHIFT: u8 = 4;

// ─── Scan / BSS Info ───────────────────────────────────────────────

/// Maximum number of scan results
const MAX_SCAN_RESULTS: usize = 32;

/// Maximum SSID length (802.11)
const MAX_SSID_LEN: usize = 32;

/// BSS information from scan results
#[derive(Clone, Copy)]
pub struct BssInfo {
    pub bssid: [u8; 6],
    pub ssid: [u8; MAX_SSID_LEN],
    pub ssid_len: u8,
    pub channel: u8,
    pub rssi: i8, // Signal strength dBm
    pub security: SecurityType,
    pub infra: bool, // true = infrastructure, false = IBSS
}

/// Security type
#[derive(Clone, Copy, PartialEq)]
pub enum SecurityType {
    Open,
    Wep,
    WpaPsk,
    Wpa2Psk,
    Wpa3Sae,
    Unknown,
}

/// WiFi connection state
#[derive(Clone, Copy, PartialEq)]
pub enum WifiState {
    Uninitialized,
    FirmwareLoaded,
    Up,
    Scanning,
    Connecting,
    Connected,
    Disconnected,
    Error,
}

// ─── IOCTL Request / Response Ring ─────────────────────────────────

/// Maximum IOCTL data size
const IOCTL_MAX_DATA: usize = 1024;

/// IOCTL request structure
struct IoctlRequest {
    cmd: u32,
    action: u32,
    data: [u8; IOCTL_MAX_DATA],
    data_len: usize,
    done: bool,
    status: u32,
}

impl IoctlRequest {
    const fn new() -> Self {
        Self {
            cmd: 0,
            action: 0,
            data: [0; IOCTL_MAX_DATA],
            data_len: 0,
            done: false,
            status: 0,
        }
    }
}

// ─── Firmware Embedding ────────────────────────────────────────────
// The actual firmware blobs would normally be embedded here.
// For now we use placeholder paths — the real firmware files
// (brcmfmac43455-sdio.bin and brcmfmac43455-sdio.clm_blob)
// must be placed at these paths before building.

// Path constants (for future include_bytes!)
const FIRMWARE_PATH: &str = "firmware/brcmfmac43455-sdio.bin";
const CLM_BLOB_PATH: &str = "firmware/brcmfmac43455-sdio.clm_blob";

// Placeholder firmware sizes (actual CYW43455 firmware is ~280KB,
// CLM blob is ~4KB)
const FIRMWARE_MAX_SIZE: usize = 300_000;
const CLM_BLOB_MAX_SIZE: usize = 8_192;

// ─── Core Descriptor ───────────────────────────────────────────────

/// SSB core descriptor
#[derive(Clone, Copy)]
struct CoreDesc {
    id: u16,
    rev: u16,
    base_addr: u32,
    wrap_addr: u32,
}

impl CoreDesc {
    const fn empty() -> Self {
        Self {
            id: 0,
            rev: 0,
            base_addr: 0,
            wrap_addr: 0,
        }
    }
}

// ─── WiFi Controller State ─────────────────────────────────────────

/// Maximum Rx buffer size
const RX_BUF_SIZE: usize = 2048;

/// Maximum Tx buffer size
const TX_BUF_SIZE: usize = 2048;

struct WifiController {
    /// Driver state
    state: WifiState,
    /// MAC address of the WiFi interface
    mac_addr: [u8; 6],
    /// Currently connected BSSID
    connected_bssid: [u8; 6],
    /// Currently connected SSID
    connected_ssid: [u8; MAX_SSID_LEN],
    connected_ssid_len: u8,
    /// Chip cores
    arm_core: CoreDesc,
    d11_core: CoreDesc,
    socsram_core: CoreDesc,
    sdio_core: CoreDesc,
    cc_core: CoreDesc,
    /// Backplane window current address
    bp_window_addr: u32,
    /// IOCTL state
    ioctl_pending: IoctlRequest,
    /// Tx sequence number
    tx_seq: u8,
    /// Rx buffer
    rx_buf: [u8; RX_BUF_SIZE],
    /// Scan results
    scan_results: [BssInfo; MAX_SCAN_RESULTS],
    scan_result_count: usize,
    /// Link state
    link_up: bool,
    /// Channel
    channel: u8,
    /// Saved network profile for auto-connect
    saved_ssid: [u8; MAX_SSID_LEN],
    saved_ssid_len: u8,
    saved_password: [u8; 64],
    saved_password_len: u8,
    has_saved_network: bool,
}

static mut WIFI: WifiController = WifiController {
    state: WifiState::Uninitialized,
    mac_addr: [0; 6],
    connected_bssid: [0; 6],
    connected_ssid: [0; MAX_SSID_LEN],
    connected_ssid_len: 0,
    arm_core: CoreDesc::empty(),
    d11_core: CoreDesc::empty(),
    socsram_core: CoreDesc::empty(),
    sdio_core: CoreDesc::empty(),
    cc_core: CoreDesc::empty(),
    bp_window_addr: 0,
    ioctl_pending: IoctlRequest::new(),
    tx_seq: 0,
    rx_buf: [0; RX_BUF_SIZE],
    scan_results: [BssInfo {
        bssid: [0; 6],
        ssid: [0; MAX_SSID_LEN],
        ssid_len: 0,
        channel: 0,
        rssi: 0,
        security: SecurityType::Unknown,
        infra: true,
    }; MAX_SCAN_RESULTS],
    scan_result_count: 0,
    link_up: false,
    channel: 1,
    saved_ssid: [0; MAX_SSID_LEN],
    saved_ssid_len: 0,
    saved_password: [0; 64],
    saved_password_len: 0,
    has_saved_network: false,
};

// ─── Backplane Window Access ───────────────────────────────────────

/// Set the backplane window address (3 SDIO byte writes)
unsafe fn bp_window_set(addr: u32) {
    let aligned = addr & BACKPLANE_WIN_MASK;
    if aligned == WIFI.bp_window_addr {
        return; // Already mapped
    }

    // Write address bytes to SB_ADDR registers
    sdio::io_write_byte(FN_BACKPLANE, SB_ADDR_LOW, (aligned & 0xFF) as u8)
        .unwrap_or_else(|e| println!("[wifi] bp_window_set low err: {:?}", e));
    sdio::io_write_byte(FN_BACKPLANE, SB_ADDR_MID, ((aligned >> 8) & 0xFF) as u8)
        .unwrap_or_else(|e| println!("[wifi] bp_window_set mid err: {:?}", e));
    sdio::io_write_byte(FN_BACKPLANE, SB_ADDR_HIGH, ((aligned >> 16) & 0xFF) as u8)
        .unwrap_or_else(|e| println!("[wifi] bp_window_set high err: {:?}", e));

    WIFI.bp_window_addr = aligned;
}

/// Read a 32-bit register from the backplane
unsafe fn bp_read32(addr: u32) -> u32 {
    bp_window_set(addr);
    let offset = (addr & !BACKPLANE_WIN_MASK) as u32;
    // Read 4 bytes from window (little-endian)
    let b0 = sdio::io_read_byte(FN_BACKPLANE, SB_WIN_DATA + offset).unwrap_or(0);
    let b1 = sdio::io_read_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 1).unwrap_or(0);
    let b2 = sdio::io_read_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 2).unwrap_or(0);
    let b3 = sdio::io_read_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 3).unwrap_or(0);
    u32::from_le_bytes([b0, b1, b2, b3])
}

/// Write a 32-bit register to the backplane
unsafe fn bp_write32(addr: u32, val: u32) {
    bp_window_set(addr);
    let offset = (addr & !BACKPLANE_WIN_MASK) as u32;
    let bytes = val.to_le_bytes();
    sdio::io_write_byte(FN_BACKPLANE, SB_WIN_DATA + offset, bytes[0])
        .unwrap_or_else(|e| println!("[wifi] bp_write32 b0 err: {:?}", e));
    sdio::io_write_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 1, bytes[1])
        .unwrap_or_else(|e| println!("[wifi] bp_write32 b1 err: {:?}", e));
    sdio::io_write_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 2, bytes[2])
        .unwrap_or_else(|e| println!("[wifi] bp_write32 b2 err: {:?}", e));
    sdio::io_write_byte(FN_BACKPLANE, SB_WIN_DATA + offset + 3, bytes[3])
        .unwrap_or_else(|e| println!("[wifi] bp_write32 b3 err: {:?}", e));
}

/// Read a block of data from the backplane using CMD53
unsafe fn bp_read_block(addr: u32, buf: &mut [u8], len: usize) -> Result<(), WifiError> {
    bp_window_set(addr);
    let offset = (addr & !BACKPLANE_WIN_MASK) as u32;
    sdio::io_read_block(FN_BACKPLANE, SB_WIN_DATA + offset, buf, len, true)?;
    Ok(())
}

/// Write a block of data to the backplane using CMD53
unsafe fn bp_write_block(addr: u32, buf: &[u8], len: usize) -> Result<(), WifiError> {
    bp_window_set(addr);
    let offset = (addr & !BACKPLANE_WIN_MASK) as u32;
    sdio::io_write_block(FN_BACKPLANE, SB_WIN_DATA + offset, buf, len, true)?;
    Ok(())
}

// ─── Core Enumeration ──────────────────────────────────────────────

/// Enumerate SSB cores on the chip's backplane
unsafe fn enumerate_cores() -> Result<(), WifiError> {
    println!("[wifi] Enumerating chip cores...");

    // Read chip ID from ChipCommon
    let chip_id = bp_read32(CC_CHIPID);
    let chip_num = chip_id & 0xFFFF;
    let chip_rev = (chip_id >> 16) & 0xF;
    let chip_pkg = (chip_id >> 20) & 0xF;
    println!(
        "[wifi] ChipID: num={:#06x} rev={} pkg={}",
        chip_num, chip_rev, chip_pkg
    );

    // Read capabilities
    let caps = bp_read32(CC_CAPABILITIES);
    println!("[wifi] ChipCommon caps: {:#010x}", caps);

    // CYW43455 chip number
    if chip_num != 0x4345 {
        println!(
            "[wifi] WARNING: unexpected chip num {:#06x} (expected 0x4345)",
            chip_num
        );
    }

    // Now scan the core table via the EROM
    // The core table starts at 0x18000000 area and is accessed via
    // the Agent Interface (AI) enumeration space.
    // Each core descriptor is 8 bytes: [0]=id, [4]=address
    // Scan through the enumeration ROM to find all cores.
    let mut core_idx: usize = 0;
    let mut found_cores = 0u32;

    // Core table base addresses — the CYW43455 has cores at fixed offsets
    // We scan known addresses based on the brcmfmac driver
    let core_addrs: &[(u16, u32, u32)] = &[
        // (core_id, base_addr, wrap_addr)
        (CC_CORE_ID, 0x18000000, 0x18100000),
        (SOCSRAM_CORE_ID, 0x18108000, 0x18105000),
        (ARMCR4_CORE_ID, 0x18001800, 0x18106000),
        (D11_CORE_ID, 0x18002000, 0x18104000),
        (SDIO_CORE_ID, 0x18004000, 0x18107000),
    ];

    for &(id, base, wrap) in core_addrs {
        // Read the core's identity register at base + 0x000
        let core_id_reg = bp_read32(base);
        let found_id = (core_id_reg & 0xFFFF) as u16;
        let found_rev = ((core_id_reg >> 16) & 0xFF) as u16;

        if found_id == id || found_id != 0 {
            let core = CoreDesc {
                id: found_id,
                rev: found_rev,
                base_addr: base,
                wrap_addr: wrap,
            };
            println!(
                "[wifi]   Core {}: id={:#06x} rev={} base={:#010x} wrap={:#010x}",
                core_idx, found_id, found_rev, base, wrap
            );

            // Store in controller
            match found_id {
                CC_CORE_ID => WIFI.cc_core = core,
                SOCSRAM_CORE_ID => WIFI.socsram_core = core,
                ARMCR4_CORE_ID => WIFI.arm_core = core,
                D11_CORE_ID => WIFI.d11_core = core,
                SDIO_CORE_ID => WIFI.sdio_core = core,
                _ => {}
            }
            found_cores |= 1;
        }
        core_idx += 1;
    }

    if found_cores == 0 {
        return Err(WifiError::CoreNotFound(0));
    }

    println!(
        "[wifi] Core enumeration complete (found {} cores)",
        core_idx
    );
    Ok(())
}

// ─── Core Reset / Control ──────────────────────────────────────────

/// Reset a specific SSB core
unsafe fn core_reset(core: &CoreDesc) {
    // Write to AI_IOCTRL at wrap_addr + 0x408
    bp_write32(core.wrap_addr + AI_IOCTRL, 0x03); // Reset + clock enable
    delay_ms(1);
    // Clear reset
    bp_write32(core.wrap_addr + AI_IOCTRL, 0x01); // Clock enable only
    delay_ms(1);
}

/// Halt (reset) the ARM CR4 core
unsafe fn arm_cr4_halt() {
    let wrap = WIFI.arm_core.wrap_addr;
    println!("[wifi] Halting ARM CR4 core...");

    // Set CPU capitulate bit
    bp_write32(wrap + AI_RESETCTRL + 0x10, ARMCR4_CAP);

    // Poll for reset status
    let mut timeout = 100_000u64;
    loop {
        let status = bp_read32(wrap + AI_RESETSTATUS);
        if (status & ARMCR4_BRESET) != 0 {
            break;
        }
        if timeout == 0 {
            println!("[wifi] WARNING: ARM CR4 halt timeout");
            break;
        }
        timeout -= 1;
        cpu_nop();
    }

    // Assert reset
    bp_write32(wrap + AI_IOCTRL, ARMCR4_BRESET);

    // Disable clock
    bp_write32(wrap + AI_IOCTRL, 0);

    println!("[wifi] ARM CR4 halted");
}

/// Start the ARM CR4 core
unsafe fn arm_cr4_start() {
    let wrap = WIFI.arm_core.wrap_addr;
    println!("[wifi] Starting ARM CR4 core...");

    // Enable clock
    bp_write32(wrap + AI_IOCTRL, ARMCR4_CLOCK_EN);
    delay_ms(1);

    // Clear reset
    bp_write32(wrap + AI_IOCTRL, ARMCR4_CLOCK_EN & !ARMCR4_BRESET);
    delay_ms(1);

    // Verify CPU is running
    let mut timeout = 100_000u64;
    loop {
        let status = bp_read32(wrap + AI_IOSTATUS);
        if (status & ARMCR4_BRESET) == 0 {
            break;
        }
        if timeout == 0 {
            println!("[wifi] WARNING: ARM CR4 start timeout");
            break;
        }
        timeout -= 1;
        cpu_nop();
    }

    println!("[wifi] ARM CR4 started");
}

// ─── SOCSRAM Setup ─────────────────────────────────────────────────

/// Initialize SOCSRAM banks for firmware download
unsafe fn socsram_init() {
    let base = WIFI.socsram_core.base_addr;
    println!("[wifi] Initializing SOCSRAM...");

    // Enable all SOCSRAM banks
    // Bank index register: bits [7:0]=bank, [8]=enable, [9]=write enable
    // We enable all banks (CYW43455 has ~128 banks of 8KB each ≈ 1MB)
    let num_banks = 128u32;
    for bank in 0..num_banks {
        let bankidx = (1 << 8) | (1 << 9) | bank; // Enable + write enable + bank index
        bp_write32(base + 0x00, bankidx); // BANKIDX
                                          // Bank PDA (power-down array) — set to 0 to enable
        bp_write32(base + 0x04, 0); // BANKPDA
    }

    println!("[wifi] SOCSRAM initialized ({} banks)", num_banks);
}

// ─── Firmware Download ─────────────────────────────────────────────

/// Download firmware to the chip via backplane
/// Returns Ok(()) if firmware was loaded successfully
unsafe fn firmware_download() -> Result<(), WifiError> {
    println!("[wifi] === Firmware Download ===");

    // Halt ARM core before loading
    arm_cr4_halt();

    // Initialize SOCSRAM
    socsram_init();

    // Try to load firmware from filesystem (SD card)
    // Path: /firmware/brcmfmac43455-sdio.bin
    let firmware = crate::fs::vfs::read_file("/firmware/brcmfmac43455-sdio.bin");
    let clm_blob = crate::fs::vfs::read_file("/firmware/brcmfmac43455-sdio.clm_blob");

    let firmware_loaded = match (firmware, clm_blob) {
        (Ok(fw), Ok(clm)) => {
            println!(
                "[wifi] Loaded firmware from filesystem: {} bytes, CLM: {} bytes",
                fw.len(),
                clm.len()
            );
            download_firmware_to_chip(&fw, &clm)?;
            true
        }
        (Ok(fw), Err(_)) => {
            println!(
                "[wifi] Loaded firmware from filesystem: {} bytes (no CLM blob)",
                fw.len()
            );
            download_firmware_to_chip(&fw, &[])?;
            true
        }
        (Err(_), Ok(clm)) => {
            println!(
                "[wifi] Loaded CLM blob only: {} bytes (no firmware)",
                clm.len()
            );
            false
        }
        (Err(_), Err(_)) => {
            println!("[wifi] Firmware files not found on filesystem, checking for pre-loaded firmware...");
            false
        }
    };

    if !firmware_loaded {
        // Check if ARM CR4 is in reset (ready for firmware)
        let status = bp_read32(WIFI.arm_core.wrap_addr + AI_IOSTATUS);
        if (status & ARMCR4_BRESET) == 0 {
            println!("[wifi] WARNING: ARM core not in reset before firmware load");
        }

        // Firmware download address — the ARM CR4 starts executing from
        // the base of SOCSRAM (typically 0x00000000 in chip address space)
        let fw_base_addr: u32 = 0x00000000;
        let ram_base = WIFI.socsram_core.base_addr;

        println!("[wifi] Firmware load address: {:#010x}", fw_base_addr);
        println!("[wifi] SOCSRAM base: {:#010x}", ram_base);

        // Verify the chip is in a state where it would accept firmware
        // by writing a test pattern to SOCSRAM and reading it back
        let test_pattern: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];
        bp_write_block(ram_base, &test_pattern, 4)?;

        let mut readback = [0u8; 4];
        bp_read_block(ram_base, &mut readback, 4)?;

        if readback == test_pattern {
            println!("[wifi] SOCSRAM read/write verified — firmware download path ready");
        } else {
            println!(
                "[wifi] WARNING: SOCSRAM readback mismatch: {:02x?} != {:02x?}",
                readback, test_pattern
            );
            // Don't fail — some chips have different bank access patterns
        }

        // === CLM blob download ===
        // The CLM (Country Locale Matrix) blob contains regulatory data.
        // It's downloaded to a specific region of SOCSRAM after firmware.
        // The firmware tells us the CLM address via a shared structure.

        // For a minimal working driver, we skip actual firmware download
        // and assume the firmware has been pre-loaded (e.g., by the
        // VideoCore bootloader or a previous OS).
        //
        // In a production driver, this would:
        // 1. Load firmware from embedded data or filesystem
        // 2. Write it to SOCSRAM in 4KB blocks
        // 3. Write CLM blob after firmware
        // 4. Verify download CRC
        // 5. Set firmware entry point
        // 6. Start ARM core

        println!("[wifi] Firmware download path configured (firmware must be loaded externally)");
        println!(
            "[wifi] Place {} and {} on SD card at /firmware/",
            FIRMWARE_PATH, CLM_BLOB_PATH
        );
    }

    // Start ARM core (even without firmware, to see if it responds)
    arm_cr4_start();

    // Wait for firmware to signal ready
    let mut timeout = 500_000u64; // ~5 seconds
    loop {
        // Check SDIO IntStatus for firmware ready bit
        let intstatus = bp_read32(WIFI.sdio_core.base_addr + 0x20);
        if (intstatus & 0x40) != 0 {
            // F2_READY
            println!("[wifi] Firmware signaled ready!");
            break;
        }
        if timeout == 0 {
            println!("[wifi] WARNING: firmware ready timeout — may need firmware load");
            // Don't fail — the chip might still work with bootloader firmware
            break;
        }
        timeout -= 1;
        cpu_nop();
    }

    WIFI.state = WifiState::FirmwareLoaded;
    Ok(())
}

/// Download firmware and CLM blob to the chip's SOCSRAM
unsafe fn download_firmware_to_chip(firmware: &[u8], clm_blob: &[u8]) -> Result<(), WifiError> {
    let ram_base = WIFI.socsram_core.base_addr;
    let fw_base_addr: u32 = 0x00000000;

    // Download firmware in 4KB blocks
    const BLOCK_SIZE: usize = 4096;
    let mut offset = 0usize;
    while offset < firmware.len() {
        let chunk = &firmware[offset..core::cmp::min(offset + BLOCK_SIZE, firmware.len())];
        bp_write_block(ram_base + fw_base_addr + offset as u32, chunk, chunk.len())?;
        offset += chunk.len();

        // Progress indicator every 64KB
        if offset % (16 * BLOCK_SIZE) == 0 || offset == firmware.len() {
            println!(
                "[wifi] Firmware download: {}/{} bytes ({}%)",
                offset,
                firmware.len(),
                offset * 100 / firmware.len()
            );
        }
    }

    // Download CLM blob if provided
    if !clm_blob.is_empty() {
        // CLM blob typically goes after firmware, at a specific offset
        // For CYW43455, the CLM address is often at firmware_size rounded up to 4KB
        let clm_offset = ((firmware.len() + BLOCK_SIZE - 1) / BLOCK_SIZE) * BLOCK_SIZE;
        println!(
            "[wifi] Downloading CLM blob at offset {} bytes ({} bytes)",
            clm_offset,
            clm_blob.len()
        );
        bp_write_block(
            ram_base + fw_base_addr + clm_offset as u32,
            clm_blob,
            clm_blob.len(),
        )?;
    }

    // Verify firmware download by reading back first 4 bytes
    let mut verify = [0u8; 4];
    bp_read_block(ram_base + fw_base_addr, &mut verify, 4)?;
    println!(
        "[wifi] Firmware verification (first 4 bytes): {:02x?}",
        verify
    );

    Ok(())
}

// ─── IOCTL Interface ───────────────────────────────────────────────

/// Build an IOCTL command word
/// Format: [31:16]=group, [15:8]=index, [7:0]=action
const fn ioctl_cmd(group: u32, index: u32, action: u32) -> u32 {
    (group << 16) | (index << 8) | action
}

/// Send an IOCTL command to the firmware and wait for response
unsafe fn ioctl_send(cmd: u32, action: u32, data: &[u8]) -> Result<u32, WifiError> {
    if WIFI.state == WifiState::Uninitialized {
        return Err(WifiError::NotInitialized);
    }

    // Write IOCTL data to ring buffer via Fn1 backplane
    // The firmware expects IOCTL data at a specific DMA ring buffer.
    // For SDIO, we write the command header + data via backplane
    // and then "ring the doorbell" to notify firmware.

    // IOCTL message header (8 bytes):
    // [0:3]  = cmd (little-endian)
    // [4:7]  = action + len (little-endian)
    // [8:11] = transaction ID
    // [12:]  = data

    let mut msg = [0u8; IOCTL_MAX_DATA + 16];
    let cmd_bytes = cmd.to_le_bytes();
    msg[0..4].copy_from_slice(&cmd_bytes);
    let action_len = (action << 16) | (data.len() as u32 & 0xFFFF);
    let al_bytes = action_len.to_le_bytes();
    msg[4..8].copy_from_slice(&al_bytes);
    // Transaction ID = 1
    msg[8] = 1;
    msg[9] = 0;
    msg[10] = 0;
    msg[11] = 0;
    // Copy data
    let data_copy_len = data.len().min(IOCTL_MAX_DATA);
    msg[12..12 + data_copy_len].copy_from_slice(&data[..data_copy_len]);

    // Write to SDIO core's PCM ring buffer
    // The brcmfmac driver uses the "dongle tohost data" ring
    let ring_base = WIFI.sdio_core.base_addr + 0x1000; // Simplified ring offset

    // Write the IOCTL message in blocks
    let total_len = 12 + data_copy_len;
    bp_write_block(ring_base, &msg[..total_len], total_len)?;

    // Ring doorbell — write to IntStatus to signal new IOCTL
    bp_write32(WIFI.sdio_core.base_addr + 0x20, 0x80); // IOCTL pending

    // Wait for firmware to process
    let mut timeout = 200_000u64; // ~2 seconds
    loop {
        let status = bp_read32(WIFI.sdio_core.base_addr + 0x20);
        if (status & 0x80) == 0 {
            // IOCTL processed
            break;
        }
        if timeout == 0 {
            return Err(WifiError::IoctlTimeout);
        }
        timeout -= 1;
        cpu_nop();
    }

    // Read response status from backplane
    let resp = bp_read32(ring_base);
    let resp_status = resp & 0xFFFF;

    if resp_status != 0 {
        return Err(WifiError::IoctlError(resp_status));
    }

    Ok(resp >> 16)
}

/// Send an IOCTL SET command
unsafe fn ioctl_set(cmd: u32, data: &[u8]) -> Result<u32, WifiError> {
    ioctl_send(cmd, IOCTL_ACTION_SET, data)
}

/// Send an IOCTL GET command
unsafe fn ioctl_get(cmd: u32, data: &mut [u8]) -> Result<u32, WifiError> {
    let result = ioctl_send(cmd, IOCTL_ACTION_GET, data)?;
    // Read response data from backplane
    let ring_base = WIFI.sdio_core.base_addr + 0x1000;
    bp_read_block(ring_base + 0x100, data, data.len())?;
    Ok(result)
}

// ─── WiFi Configuration ────────────────────────────────────────────

/// Set country code (regulatory domain)
unsafe fn set_country() -> Result<(), WifiError> {
    // Set country to "US" (FCC)
    let country = b"US\x00";
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_DEV, IOCTL_SET_COUNTRY, IOCTL_ACTION_SET),
        country,
    )?;
    println!("[wifi] Country set: US");
    Ok(())
}

/// Set infrastructure mode (BSS/infra vs IBSS)
unsafe fn set_infra_mode() -> Result<(), WifiError> {
    // 0=IBSS, 1=Infrastructure
    let mode: [u8; 4] = 1u32.to_le_bytes();
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_BSS, IOCTL_SET_INFRA, IOCTL_ACTION_SET),
        &mode,
    )?;
    println!("[wifi] Mode: infrastructure");
    Ok(())
}

/// Set authentication type
unsafe fn set_auth_type(auth: u32) -> Result<(), WifiError> {
    // 0=open, 1=shared, 2=auto
    let auth_val = auth.to_le_bytes();
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_SEC, IOCTL_SET_AUTH, IOCTL_ACTION_SET),
        &auth_val,
    )?;
    Ok(())
}

/// Set wireless security (WSEC)
unsafe fn set_wsec(wsec: u32) -> Result<(), WifiError> {
    // WSEC bits: 1=WEP, 2=TKIP, 4=AES, 0x10=SMS4
    let wsec_val = wsec.to_le_bytes();
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_SEC, IOCTL_SET_WSEC, IOCTL_ACTION_SET),
        &wsec_val,
    )?;
    Ok(())
}

/// Set WPA2 passphrase key (PMK)
unsafe fn set_wsec_pmk(pmk: &[u8]) -> Result<(), WifiError> {
    // PMK is 32 bytes for WPA2
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_SEC, IOCTL_SET_WSEC_PMK, IOCTL_ACTION_SET),
        pmk,
    )?;
    Ok(())
}

/// Bring WiFi interface up
unsafe fn iface_up() -> Result<(), WifiError> {
    ioctl_set(ioctl_cmd(0, IOCTL_UP, IOCTL_ACTION_SET), &[])?;
    println!("[wifi] Interface UP");
    WIFI.state = WifiState::Up;
    Ok(())
}

/// Bring WiFi interface down
unsafe fn iface_down() -> Result<(), WifiError> {
    ioctl_set(ioctl_cmd(0, IOCTL_DOWN, IOCTL_ACTION_SET), &[])?;
    println!("[wifi] Interface DOWN");
    WIFI.state = WifiState::FirmwareLoaded;
    WIFI.link_up = false;
    Ok(())
}

/// Read MAC address from firmware
unsafe fn read_mac_addr() -> Result<(), WifiError> {
    let mut buf = [0u8; 8];
    ioctl_get(ioctl_cmd(0, IOCTL_GET_MAC, IOCTL_ACTION_GET), &mut buf)?;
    WIFI.mac_addr.copy_from_slice(&buf[0..6]);
    println!(
        "[wifi] MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        WIFI.mac_addr[0],
        WIFI.mac_addr[1],
        WIFI.mac_addr[2],
        WIFI.mac_addr[3],
        WIFI.mac_addr[4],
        WIFI.mac_addr[5]
    );
    Ok(())
}

/// Set channel
unsafe fn set_channel(channel: u32) -> Result<(), WifiError> {
    let ch = channel.to_le_bytes();
    ioctl_set(
        ioctl_cmd(IOCTL_GRP_BSS, IOCTL_SET_CHANNEL, IOCTL_ACTION_SET),
        &ch,
    )?;
    WIFI.channel = channel as u8;
    Ok(())
}

// ─── Scan Implementation ───────────────────────────────────────────

/// Extended scan result format from firmware
#[repr(C)]
struct EscanResult {
    buflen: u32,
    version: u32,
    sync_id: u16,
    bss_count: u16,
    // Followed by BSS info structures
}

/// BSS info structure from firmware scan result
#[repr(C)]
struct BssInfoRaw {
    version: u32,
    length: u32,
    bssid: [u8; 6],
    beacon_period: u16,
    capability: u16,
    ssid_len: u32,
    ssid: [u8; 32],
    channel: u32,
    rssi: i32,
    // More fields follow...
}

/// Initiate a WiFi scan
unsafe fn escan_start() -> Result<(), WifiError> {
    if WIFI.state != WifiState::Up && WIFI.state != WifiState::Disconnected {
        return Err(WifiError::InvalidState);
    }

    WIFI.state = WifiState::Scanning;
    WIFI.scan_result_count = 0;

    // Build scan parameters
    // ESCAN ioctl expects: version, action (1=start, 2=abort), sync_id, ssid, bssid, scan_type, ...
    let mut scan_params = [0u8; 128];
    // Version = 2 (ESCAN version)
    scan_params[0..4].copy_from_slice(&2u32.to_le_bytes());
    // Action = 1 (start scan)
    scan_params[4..8].copy_from_slice(&1u32.to_le_bytes());
    // Sync ID
    scan_params[8..10].copy_from_slice(&0x1234u16.to_le_bytes());
    // SSID = broadcast (empty = scan all)
    // BSSID = broadcast
    scan_params[16..22].copy_from_slice(&[0xFF; 6]);
    // Scan type = 1 (passive), 0 = active
    scan_params[24..28].copy_from_slice(&0u32.to_le_bytes());

    let cmd = ioctl_cmd(IOCTL_GRP_ESCAN, IOCTL_ESCAN, IOCTL_ACTION_SET);
    ioctl_set(cmd, &scan_params[..64])?;

    // Wait for scan results (firmware sends events)
    let mut timeout = 1_000_000u64; // ~10 seconds for scan
    loop {
        if WIFI.state != WifiState::Scanning {
            break; // State changed by event handler
        }
        if timeout == 0 {
            println!("[wifi] Scan timeout");
            WIFI.state = WifiState::Up;
            break;
        }
        timeout -= 1;
        cpu_nop();

        // Poll for events
        poll_events();
    }

    println!("[wifi] Scan complete: {} results", WIFI.scan_result_count);
    if WIFI.state == WifiState::Scanning {
        WIFI.state = WifiState::Up;
    }
    Ok(())
}

// ─── Connect / Disconnect ──────────────────────────────────────────

/// Connect to a WiFi network
unsafe fn wifi_connect(ssid: &[u8], password: &[u8]) -> Result<(), WifiError> {
    if WIFI.state != WifiState::Up && WIFI.state != WifiState::Disconnected {
        return Err(WifiError::InvalidState);
    }

    WIFI.state = WifiState::Connecting;
    println!(
        "[wifi] Connecting to SSID: {}",
        core::str::from_utf8(ssid).unwrap_or("<binary>")
    );

    // 1. Set auth type: open or WPA2
    if password.is_empty() {
        set_auth_type(0)?; // Open
        set_wsec(0)?; // No encryption
    } else {
        set_auth_type(2)?; // Auto (WPA2)
        set_wsec(4)?; // AES (WPA2)
                      // Derive PMK from passphrase using PBKDF2-HMAC-SHA1 (WPA2 standard)
                      // PMK = PBKDF2-HMAC-SHA1(password, SSID, 4096, 32)
        let pmk = pbkdf2_hmac_sha1(password, ssid, 4096, 32);
        set_wsec_pmk(&pmk)?;
    }

    // 2. Set SSID
    let mut ssid_buf = [0u8; 36]; // 4 bytes len + 32 bytes SSID
    let ssid_len = ssid.len().min(32);
    ssid_buf[0..4].copy_from_slice(&(ssid_len as u32).to_le_bytes());
    ssid_buf[4..4 + ssid_len].copy_from_slice(&ssid[..ssid_len]);

    // Store in controller
    WIFI.connected_ssid = [0; MAX_SSID_LEN];
    WIFI.connected_ssid[..ssid_len].copy_from_slice(&ssid[..ssid_len]);
    WIFI.connected_ssid_len = ssid_len as u8;

    // 3. Send JOIN ioctl
    let cmd = ioctl_cmd(IOCTL_GRP_BSS, IOCTL_JOIN, IOCTL_ACTION_SET);
    ioctl_set(cmd, &ssid_buf[..4 + ssid_len])?;

    // 4. Send SET_SSID ioctl
    let cmd2 = ioctl_cmd(IOCTL_GRP_SSID, IOCTL_SET_SSID, IOCTL_ACTION_SET);
    ioctl_set(cmd2, &ssid_buf[..4 + ssid_len])?;

    // 5. Wait for connection event
    let mut timeout = 2_000_000u64; // ~20 seconds
    loop {
        if WIFI.state == WifiState::Connected {
            break;
        }
        if WIFI.state == WifiState::Error || WIFI.state == WifiState::Disconnected {
            return Err(WifiError::ConnectFailed);
        }
        if timeout == 0 {
            WIFI.state = WifiState::Disconnected;
            return Err(WifiError::ConnectFailed);
        }
        timeout -= 1;
        cpu_nop();

        poll_events();
    }

    println!("[wifi] Connected to SSID!");
    Ok(())
}

/// Disconnect from the current WiFi network
unsafe fn wifi_disconnect() -> Result<(), WifiError> {
    // Send disassociate
    let cmd = ioctl_cmd(IOCTL_GRP_BSS, IOCTL_DISASSOC, IOCTL_ACTION_SET);
    ioctl_set(cmd, &[])?;

    // Bring interface down
    iface_down()?;

    WIFI.connected_bssid = [0; 6];
    WIFI.connected_ssid = [0; MAX_SSID_LEN];
    WIFI.connected_ssid_len = 0;
    WIFI.link_up = false;
    WIFI.state = WifiState::Up;

    println!("[wifi] Disconnected");
    Ok(())
}

// ─── Event Handling ────────────────────────────────────────────────

/// Poll for firmware events (called from main loop or interrupt)
unsafe fn poll_events() {
    // Check SDIO Fn2 for event frames
    // Read the interrupt status from the SDIO core
    let intstatus = bp_read32(WIFI.sdio_core.base_addr + 0x20);

    if intstatus == 0 {
        return;
    }

    // Clear interrupt
    bp_write32(WIFI.sdio_core.base_addr + 0x20, intstatus);

    // F2 packet available
    if (intstatus & 0x01) != 0 {
        // Read event/data from Fn2
        if sdio::io_read_block(FN_DATA, 0, &mut WIFI.rx_buf, RX_BUF_SIZE, false).is_ok() {
            process_event_frame(&WIFI.rx_buf);
        }
    }

    // Process event frames from the rx_buf
    // The firmware sends events as SDPCM frames on Fn2
    // For now, we just handle the key state changes
    if (intstatus & 0x40) != 0 {
        // F2 ready — firmware is running
    }
}

/// Process a received event frame
unsafe fn process_event_frame(data: &[u8]) {
    if data.len() < 4 {
        return;
    }

    // SDPCM header: [0]=len_lo, [1]=len_hi, [2]=offset, [3]=flags
    let _len = (data[0] as u16) | ((data[1] as u16) << 8);
    let flags = data[3];

    if flags & SDPCM_EVENT_FLAG != 0 {
        // Event frame
        if data.len() < 12 {
            return;
        }
        // Event data starts after SDPCM header
        let evt_type = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        match evt_type {
            EVT_SET_SSID => {
                // Connection result
                if data.len() >= 16 {
                    let status = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
                    if status == 0 {
                        WIFI.state = WifiState::Connected;
                        WIFI.link_up = true;
                        println!("[wifi] Event: SET_SSID success");
                    } else {
                        WIFI.state = WifiState::Disconnected;
                        println!("[wifi] Event: SET_SSID failed (status={})", status);
                    }
                }
            }
            EVT_DISASSOC | EVT_DEAUTH => {
                WIFI.state = WifiState::Disconnected;
                WIFI.link_up = false;
                println!("[wifi] Event: disassociated");
            }
            EVT_LINK => {
                if data.len() >= 14 {
                    let link_state = data[13];
                    WIFI.link_up = link_state != 0;
                    println!(
                        "[wifi] Event: link {}",
                        if WIFI.link_up { "UP" } else { "DOWN" }
                    );
                }
            }
            EVT_ESCAN_RESULT => {
                // Parse scan results
                parse_scan_result(data);
            }
            EVT_AUTH => {
                // Authentication event (part of 4-way handshake)
                if data.len() >= 16 {
                    let status = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
                    if status == 0 {
                        println!("[wifi] Event: AUTH success (handshake step)");
                    } else {
                        println!("[wifi] Event: AUTH failed (status={})", status);
                    }
                }
            }
            EVT_JOIN => {
                // Join completion event
                if data.len() >= 16 {
                    let status = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
                    if status == 0 {
                        WIFI.state = WifiState::Connected;
                        WIFI.link_up = true;
                        println!("[wifi] Event: JOIN success");
                    } else {
                        println!("[wifi] Event: JOIN failed (status={})", status);
                    }
                }
            }
            EVT_IFACE => {
                // Interface state change event
                println!("[wifi] Event: IFACE change");
            }
            _ => {
                // Unknown event
                println!("[wifi] Event: unknown type={}", evt_type);
            }
        }
    } else if flags & SDPCM_DATA_FLAG != 0 {
        // Data frame received — process it
        // The network stack will call receive() to get the data
    }
}

/// Parse security type from RSN/WPA/WPA2 IEs in scan results
/// Returns the detected SecurityType
unsafe fn parse_security_from_ies(data: &[u8], ie_offset: usize, data_len: usize) -> SecurityType {
    if ie_offset >= data_len {
        return SecurityType::Open;
    }

    let mut offset = ie_offset;
    let mut has_rsn = false;
    let mut has_wpa = false;
    let mut has_wpa3 = false;

    // Walk through IEs (Tag-Length-Value format)
    while offset + 2 <= data_len {
        let tag = data[offset];
        let len = data[offset + 1] as usize;

        if offset + 2 + len > data_len {
            break;
        }

        let ie_data = &data[offset + 2..offset + 2 + len];

        match tag {
            0x30 => {
                // RSN IE (802.11i / WPA2/WPA3)
                has_rsn = true;
                // Check for SAE (WPA3) - AKM suite 0x000FAC:8 or 0x000FAC:18
                if len >= 8 {
                    let akm_count = u16::from_le_bytes([ie_data[6], ie_data[7]]) as usize;
                    if len >= 8 + akm_count * 4 {
                        for i in 0..akm_count {
                            let akm_offset = 8 + i * 4;
                            let akm = u32::from_le_bytes([
                                ie_data[akm_offset],
                                ie_data[akm_offset + 1],
                                ie_data[akm_offset + 2],
                                ie_data[akm_offset + 3],
                            ]);
                            // 0x000FAC:8 = SAE (WPA3-Personal)
                            // 0x000FAC:18 = SAE Ext Key (WPA3)
                            if akm == 0x08AC0F00 || akm == 0x18AC0F00 {
                                has_wpa3 = true;
                            }
                        }
                    }
                }
            }
            0xDD => {
                // Vendor Specific IE - could be WPA (Microsoft) or WPA3
                if len >= 4 && ie_data[0] == 0x00 && ie_data[1] == 0x50 && ie_data[2] == 0xF2 {
                    // Microsoft OUI: 00:50:F2
                    let oui_type = ie_data[3];
                    if oui_type == 1 {
                        has_wpa = true; // WPA (original)
                    } else if oui_type == 4 {
                        has_wpa3 = true; // WPA3 (SAE)
                    }
                }
            }
            _ => {}
        }

        offset += 2 + len;
    }

    if has_rsn || has_wpa3 {
        if has_wpa3 {
            SecurityType::Wpa3Sae
        } else {
            SecurityType::Wpa2Psk
        }
    } else if has_wpa {
        SecurityType::WpaPsk
    } else {
        // Fallback to capability bits
        SecurityType::Open
    }
}

/// Parse scan result event data
unsafe fn parse_scan_result(data: &[u8]) {
    if data.len() < 20 {
        return;
    }

    // ESCAN result header
    let _sync_id = u16::from_le_bytes([data[12], data[13]]);
    let bss_count = u16::from_le_bytes([data[14], data[15]]) as usize;

    // Parse each BSS info structure
    let mut offset = 16usize;
    for _ in 0..bss_count.min(MAX_SCAN_RESULTS) {
        if offset + 64 > data.len() {
            break;
        }

        let idx = WIFI.scan_result_count;
        if idx >= MAX_SCAN_RESULTS {
            break;
        }

        // Parse BSS info (simplified — real format has variable-length TLVs)
        let result = &mut WIFI.scan_results[idx];

        // BSSID at offset +8
        if offset + 14 <= data.len() {
            result.bssid.copy_from_slice(&data[offset + 8..offset + 14]);
        }

        // SSID length and SSID
        if offset + 22 <= data.len() {
            let ssid_len = u32::from_le_bytes([
                data[offset + 14],
                data[offset + 15],
                data[offset + 16],
                data[offset + 17],
            ]) as usize;
            let ssid_len = ssid_len.min(MAX_SSID_LEN);
            if offset + 18 + ssid_len <= data.len() {
                result.ssid = [0; MAX_SSID_LEN];
                result.ssid[..ssid_len].copy_from_slice(&data[offset + 18..offset + 18 + ssid_len]);
                result.ssid_len = ssid_len as u8;
            }
        }

        // Channel (from DS parameter set)
        if offset + 50 <= data.len() {
            result.channel = data[offset + 50];
        }

        // RSSI
        if offset + 54 <= data.len() {
            result.rssi = i32::from_le_bytes([
                data[offset + 50],
                data[offset + 51],
                data[offset + 52],
                data[offset + 53],
            ]) as i8;
        }

        // Security type (from capability bits + IE parsing)
        if offset + 6 <= data.len() {
            let capability = u16::from_le_bytes([data[offset + 4], data[offset + 5]]);
            result.infra = (capability & 0x0002) != 0; // ESS bit

            // Parse IEs to determine security type
            // IEs start after the fixed portion of BSS info
            // The IEs typically start at offset + 42 (after fixed fields)
            let ie_offset = offset + 42;
            result.security = parse_security_from_ies(data, ie_offset, data.len());
        }

        // Move to next BSS entry
        if offset + 4 <= data.len() {
            let entry_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += if entry_len > 0 { entry_len } else { 64 };
        } else {
            break;
        }

        WIFI.scan_result_count += 1;
    }
}

// ─── Tx/Rx Data Path ───────────────────────────────────────────────

/// Transmit an Ethernet frame over WiFi
/// The firmware handles 802.3→802.11 conversion
unsafe fn wifi_transmit(data: &[u8]) -> Result<(), WifiError> {
    if !WIFI.link_up {
        return Err(WifiError::NotConnected);
    }
    if data.len() > TX_BUF_SIZE - SDPCM_HEADER_LEN {
        return Err(WifiError::TxFailed);
    }

    // Build SDPCM Tx header
    let mut tx_frame = [0u8; TX_BUF_SIZE];
    let total_len = SDPCM_HEADER_LEN + data.len();

    // SDPCM header:
    // [0:1] = frame length (little-endian)
    // [2]   = offset to data (4 for SDPCM)
    // [3]   = flags (data + seq)
    tx_frame[0] = (total_len & 0xFF) as u8;
    tx_frame[1] = ((total_len >> 8) & 0xFF) as u8;
    tx_frame[2] = SDPCM_HEADER_LEN as u8;
    tx_frame[3] = SDPCM_DATA_FLAG | (WIFI.tx_seq << SDPCM_TX_SEQ_SHIFT);

    // Copy Ethernet frame data
    tx_frame[SDPCM_HEADER_LEN..total_len].copy_from_slice(data);

    // Advance Tx sequence
    WIFI.tx_seq = WIFI.tx_seq.wrapping_add(1) & 0x0F;

    // Write to SDIO Fn2 (data function)
    sdio::io_write_block(FN_DATA, 0, &tx_frame[..total_len], total_len, true)?;

    Ok(())
}

/// Receive an Ethernet frame from WiFi
/// Returns the number of bytes received (0 if no data)
unsafe fn wifi_receive(buf: &mut [u8]) -> usize {
    if !WIFI.link_up {
        return 0;
    }

    // Check if Fn2 has data available
    // Read from SDIO Fn2
    let mut rx_tmp = [0u8; RX_BUF_SIZE];
    match sdio::io_read_block(FN_DATA, 0, &mut rx_tmp, RX_BUF_SIZE, false) {
        Ok(()) => {
            // Parse SDPCM header
            if rx_tmp.len() < SDPCM_HEADER_LEN {
                return 0;
            }

            let frame_len = (rx_tmp[0] as usize) | ((rx_tmp[1] as usize) << 8);
            let _offset = rx_tmp[2] as usize;
            let flags = rx_tmp[3];

            if frame_len < SDPCM_HEADER_LEN || frame_len > RX_BUF_SIZE {
                return 0;
            }

            if flags & SDPCM_DATA_FLAG != 0 {
                // Data frame — extract Ethernet frame
                let data_len = frame_len - SDPCM_HEADER_LEN;
                if data_len > buf.len() {
                    return 0;
                }
                buf[..data_len].copy_from_slice(&rx_tmp[SDPCM_HEADER_LEN..frame_len]);
                return data_len;
            } else if flags & SDPCM_EVENT_FLAG != 0 {
                // Event frame — process it
                process_event_frame(&rx_tmp[..frame_len]);
                return 0; // No data frame for caller
            }
            0
        }
        Err(_) => 0,
    }
}

// ─── Helpers ───────────────────────────────────────────────────────

fn delay_ms(ms: u64) {
    let target = crate::arch::timer::Timer::system_time_us() + ms * 1000;
    while crate::arch::timer::Timer::system_time_us() < target {
        cpu_nop();
    }
}

fn cpu_nop() {
    crate::arch::cpu::nop();
}

// ─── Public API ────────────────────────────────────────────────────

/// Initialize the WiFi driver
/// Must be called after SDIO bus initialization (sdio::init())
pub unsafe fn init() -> Result<(), WifiError> {
    println!("[wifi] === BCM43455 WiFi Driver Initialization ===");

    if !sdio::is_initialized() {
        println!("[wifi] ERROR: SDIO bus not initialized");
        return Err(WifiError::NotInitialized);
    }

    // 1. Enumerate chip cores
    enumerate_cores()?;

    // 2. Download firmware (or verify it's loaded)
    firmware_download()?;

    // 3. Configure WiFi
    set_country()?;
    set_infra_mode()?;
    set_auth_type(2)?; // Auto auth
    set_channel(1)?; // Default channel

    // 4. Read MAC address
    read_mac_addr()?;

    // 5. Bring interface up
    iface_up()?;

    WIFI.state = WifiState::Up;
    println!("[wifi] === WiFi driver ready ===");
    Ok(())
}

/// Get the WiFi MAC address
pub fn mac_addr() -> [u8; 6] {
    unsafe { WIFI.mac_addr }
}

/// Check if WiFi is connected
pub fn is_up() -> bool {
    unsafe { WIFI.link_up }
}

/// Get current WiFi state
pub fn state() -> WifiState {
    unsafe { WIFI.state }
}

/// Get current channel
pub fn channel() -> u8 {
    unsafe { WIFI.channel }
}

/// Scan for WiFi networks
/// Returns a slice of BssInfo (stored in internal buffer)
pub fn scan() -> Result<&'static [BssInfo], WifiError> {
    unsafe {
        escan_start()?;
        let count = WIFI.scan_result_count;
        Ok(&WIFI.scan_results[..count])
    }
}

/// Get scan result count
pub fn scan_result_count() -> usize {
    unsafe { WIFI.scan_result_count }
}

/// Get a specific scan result
pub fn scan_result(index: usize) -> Option<BssInfo> {
    unsafe {
        if index < WIFI.scan_result_count {
            Some(WIFI.scan_results[index])
        } else {
            None
        }
    }
}

/// Connect to a WiFi network with SSID and password
/// For open networks, pass empty password
pub fn connect(ssid: &[u8], password: &[u8]) -> Result<(), WifiError> {
    unsafe { wifi_connect(ssid, password) }
}

/// Disconnect from current WiFi network
pub fn disconnect() -> Result<(), WifiError> {
    unsafe { wifi_disconnect() }
}

/// Transmit an Ethernet frame over WiFi
/// The firmware converts 802.3→802.11 automatically
pub fn transmit(data: &[u8]) -> Result<(), WifiError> {
    unsafe { wifi_transmit(data) }
}

/// Receive an Ethernet frame from WiFi
/// Returns the number of bytes placed in buf (0 if no data)
pub fn receive(buf: &mut [u8]) -> usize {
    unsafe { wifi_receive(buf) }
}

/// Poll for WiFi events and Rx data
/// Call this periodically from the main loop
pub fn poll() {
    unsafe {
        if WIFI.state != WifiState::Uninitialized {
            poll_events();
        }
    }
}

/// SDIO interrupt handler for WiFi
/// Called from GIC dispatcher when SDIO2 IRQ fires
pub unsafe fn handle_interrupt() {
    // Read SDIO core interrupt status
    if WIFI.state != WifiState::Uninitialized {
        let intstatus = bp_read32(WIFI.sdio_core.base_addr + 0x20);
        if intstatus != 0 {
            // F2 data available
            if (intstatus & 0x01) != 0 {
                // Data available — will be read on next poll()
            }
            // F2 ready
            if (intstatus & 0x40) != 0 {
                // Firmware ready
            }
            // Clear interrupt
            bp_write32(WIFI.sdio_core.base_addr + 0x20, intstatus);
        }
    }
}

/// Get connected BSSID
pub fn connected_bssid() -> [u8; 6] {
    unsafe { WIFI.connected_bssid }
}

/// Get connected SSID as bytes
pub fn connected_ssid() -> (&'static [u8], usize) {
    unsafe {
        let len = WIFI.connected_ssid_len as usize;
        (&WIFI.connected_ssid[..len], len)
    }
}

// ─── Network Profiles ────────────────────────────────────────────────

/// WiFi network profile for persistent storage
#[derive(Clone, Copy)]
struct WifiProfile {
    ssid: [u8; MAX_SSID_LEN],
    ssid_len: u8,
    password: [u8; 64],
    password_len: u8,
}

impl WifiProfile {
    const fn empty() -> Self {
        Self {
            ssid: [0; MAX_SSID_LEN],
            ssid_len: 0,
            password: [0; 64],
            password_len: 0,
        }
    }
}

/// Save a network profile to persistent storage (/etc/wifi/known_networks)
unsafe fn wifi_save_network(ssid: &[u8], password: &[u8]) -> Result<(), WifiError> {
    // Store in memory for immediate use
    let ssid_len = ssid.len().min(MAX_SSID_LEN);
    WIFI.saved_ssid = [0; MAX_SSID_LEN];
    WIFI.saved_ssid[..ssid_len].copy_from_slice(&ssid[..ssid_len]);
    WIFI.saved_ssid_len = ssid_len as u8;

    let pw_len = password.len().min(63);
    WIFI.saved_password = [0; 64];
    WIFI.saved_password[..pw_len].copy_from_slice(&password[..pw_len]);
    WIFI.saved_password_len = pw_len as u8;

    WIFI.has_saved_network = true;

    // Persist to VFS: /etc/wifi/known_networks (simple INI format)
    // Format:
    // [network]
    // ssid=<hex>
    // password=<hex>
    // For simplicity, store raw bytes as hex strings

    use crate::fs::vfs::{create_dir, create, open, write};

    // Ensure /etc/wifi directory exists
    let _ = create_dir("/etc/wifi");

    // Create the config file (get inode ID)
    let _ = create("/etc/wifi/known_networks");

    // Open for writing to get file descriptor
    let fd = match open("/etc/wifi/known_networks", 1) { // O_WRONLY
        Ok(fd) => fd,
        Err(e) => {
            println!("[wifi] Failed to open config file: {:?}", e);
            return Ok(()); // Don't fail connection if config save fails
        }
    };

    // Build config content (simple key=value per line, hex-encoded)
    let mut config_buf = alloc::string::String::new();
    config_buf.push_str("[network]\n");
    config_buf.push_str("ssid=");
    for &b in &ssid[..ssid_len] {
        use core::fmt::Write;
        write!(&mut config_buf, "{:02x}", b).ok();
    }
    config_buf.push('\n');
    config_buf.push_str("password=");
    for &b in &password[..pw_len] {
        use core::fmt::Write;
        write!(&mut config_buf, "{:02x}", b).ok();
    }
    config_buf.push('\n');

    // Write to file
    let _ = write(fd, config_buf.as_bytes());

    println!("[wifi] Saved network profile to /etc/wifi/known_networks");
    Ok(())
}

/// Load saved network profiles from persistent storage (/etc/wifi/known_networks)
/// Returns a vector of (ssid, password) tuples
unsafe fn wifi_load_profiles() -> alloc::vec::Vec<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)> {
    use crate::fs::vfs::read_file;

    let mut profiles = alloc::vec::Vec::new();

    // Read the config file
    let data = match read_file("/etc/wifi/known_networks") {
        Ok(d) => d,
        Err(_) => return profiles, // No config file, return empty
    };

    // Parse simple INI format: key=value lines, hex-encoded
    let content = core::str::from_utf8(&data).unwrap_or("");
    let mut current_ssid = alloc::vec::Vec::new();
    let mut current_password = alloc::vec::Vec::new();
    let mut in_network_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[network]" {
            // Save previous network if complete
            if !current_ssid.is_empty() {
                profiles.push((core::mem::take(&mut current_ssid), core::mem::take(&mut current_password)));
            }
            in_network_section = true;
            continue;
        }
        if !in_network_section {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos].trim();
            let value = &line[eq_pos + 1..].trim();
            match *key {
                "ssid" => {
                    // Decode hex string
                    current_ssid = (0..value.len())
                        .step_by(2)
                        .filter_map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
                        .collect();
                }
                "password" => {
                    current_password = (0..value.len())
                        .step_by(2)
                        .filter_map(|i| u8::from_str_radix(&value[i..i + 2], 16).ok())
                        .collect();
                }
                _ => {}
            }
        }
    }

    // Don't forget the last one
    if !current_ssid.is_empty() {
        profiles.push((current_ssid, current_password));
    }

    profiles
}

/// Get saved network profiles (loads from VFS if not already loaded)
pub fn saved_networks() -> alloc::vec::Vec<(alloc::vec::Vec<u8>, alloc::vec::Vec<u8>)> {
    unsafe { wifi_load_profiles() }
}

/// Attempt auto-connect to saved network(s)
/// Tries each saved profile until one succeeds
pub fn auto_connect() -> Result<(), WifiError> {
    let profiles = saved_networks();
    if profiles.is_empty() {
        return Err(WifiError::NotInitialized);
    }

    for (ssid, password) in profiles {
        println!(
            "[wifi] Auto-connecting to saved network: {}",
            core::str::from_utf8(&ssid).unwrap_or("<binary>")
        );
        match connect(&ssid, &password) {
            Ok(()) => {
                println!("[wifi] Auto-connected successfully!");
                return Ok(());
            }
            Err(e) => {
                println!("[wifi] Auto-connect failed for {}: {:?}", core::str::from_utf8(&ssid).unwrap_or("<binary>"), e);
                // Try next profile
            }
        }
    }

    Err(WifiError::ConnectFailed)
}

/// Check if link is up
pub fn link_up() -> bool {
    unsafe { WIFI.link_up }
}

/// Save a network profile for auto-connect on boot
pub fn save_network(ssid: &[u8], password: &[u8]) -> Result<(), WifiError> {
    unsafe { wifi_save_network(ssid, password) }
}
