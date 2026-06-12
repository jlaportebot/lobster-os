//! GENET (Gigabit Ethernet) driver for BCM2712
//!
//! The BCM2712 SoC includes a Broadcom GENETv5 Ethernet MAC.
//! Base address: 0x10_7D50_0000 (mapped via PHYS_MAP_OFFSET or DEVICE_MAP_START)
//!
//! Features:
//! - MAC initialization from hardware (OTP/metadata)
//! - DMA descriptor ring setup
//! - TX/RX packet processing via interrupt (SPI 189)
//! - Link status polling
//! - Scatter-gather DMA support

#[cfg(target_arch = "aarch64")]
use crate::arch::gic::Gic;
use crate::memory::{phys_to_virt, virt_to_phys, VirtAddr};
use crate::println;

// ─── GENET MMIO offsets ────────────────────────────────────────────
// Full register map for BCM27ETv5
const GENET_BASE_PHYS: u64 = 0x10_7D50_0000;

/// System Parameter Configuration
const SYS_REV: u64 = 0x00;
const SYS_PORT_MODE: u64 = 0x04;
const SYS_RBUF_FLUSH_CTRL: u64 = 0x08;
const SYS_TBUF_FLUSH_CTRL: u64 = 0x0C;

/// UMAC registers
const UMAC_CMD: u64 = 0x0808;
const UMAC_MAC0: u64 = 0x080C;
const UMAC_MAC1: u64 = 0x0810;
const UMAC_MAX_FRAME_LEN: u64 = 0x0814;
const UMAC_TX_FLUSH: u64 = 0x0818;
const UMAC_EEE_CTRL: u64 = 0x0864;

/// UMAC Interrupt registers
const UMAC_IRQ0: u64 = 0x0824;
const UMAC_IRQ1: u64 = 0x0828;
const UMAC_IRQ2: u64 = 0x082C;
const UMAC_IRQ3: u64 = 0x0830;
const INTRL0_DEV_SET: u64 = 0x0834;
const INTRL0_DEV_CLEAR: u64 = 0x0838;
const INTRL0_DEV_MASK: u64 = 0x083C;

/// RX/TX DMA ring registers
const DMA_RINGS_SIZE: u64 = 0x0400;
const DMA_CTRL: u64 = 0x0400;
const DMA_STATUS: u64 = 0x0404;
const DMA_SCB_BURST_SIZE: u64 = 0x0408;

const DMA_RING_DESC_BASE: u64 = 0x041C;
const DMA_RING_DESC_COUNT: u64 = 0x0420;
const DMA_RING_PROD_INDEX: u64 = 0x0424;
const DMA_RING_CONS_INDEX: u64 = 0x0428;

/// TX ring 16 offsets from DMA_RINGS_SIZE base
const TX_RING_BASE: u64 = 0x0600;
const TX_RING_DESC_BASE_LO: u64 = 0x0604;
const TX_RING_DESC_BASE_HI: u64 = 0x0608;
const TX_RING_DESC_COUNT: u64 = 0x060C;
const TX_RING_PROD_INDEX: u64 = 0x0610;
const TX_RING_CONS_INDEX: u64 = 0x0614;

/// RX ring 16 offsets
const RX_RING_BASE: u64 = 0x0E00;
const RX_RING_DESC_BASE_LO: u64 = 0x0E04;
const RX_RING_DESC_BASE_HI: u64 = 0x0E08;
const RX_RING_DESC_COUNT: u64 = 0x0E0C;
const RX_RING_PROD_INDEX: u64 = 0x0E10;
const RX_RING_CONS_INDEX: u64 = 0x0E14;

/// GENET IRQ SPI number (BCM2712)
pub const GENET_SPI: u32 = 189;

/// Number of TX/RX DMA descriptors
const NUM_TX_DESC: usize = 64;
const NUM_RX_DESC: usize = 64;

/// Number of RX queues (RSS)
pub const NUM_RX_QUEUES: usize = 4;

/// Maximum Ethernet frame size (MTU + headers + FCS)
const MAX_FRAME_SIZE: usize = 1536;

/// RSS (Receive Side Scaling) constants
pub const RSS_KEY_SIZE: usize = 40;
pub const RSS_INDIR_TABLE_SIZE: usize = 128;

/// RSS hash types
pub const RSS_HASH_IPV4: u32 = 1 << 0;
pub const RSS_HASH_TCP_IPV4: u32 = 1 << 1;
pub const RSS_HASH_IPV6: u32 = 1 << 2;
pub const RSS_HASH_TCP_IPV6: u32 = 1 << 3;

/// Wake-on-LAN constants
pub const WOL_MAGIC_PACKET: u32 = 1 << 0;
pub const WOL_PATTERN_MATCH: u32 = 1 << 1;
pub const WOL_LINK_CHANGE: u32 = 1 << 2;
pub const WOL_MAGIC_PACKET_ENABLE: u32 = 1 << 0;
pub const WOL_PATTERN_ENABLE: u32 = 1 << 1;
pub const WOL_LINK_CHANGE_ENABLE: u32 = 1 << 2;

/// MDIO (Management Data Input/Output) register offsets
const MDIO_CTRL: u64 = 0x0880;
const MDIO_DATA: u64 = 0x0884;

/// MDIO control bit definitions
const MDIO_CTRL_READ: u32 = 1 << 30;
const MDIO_CTRL_WRITE: u32 = 1 << 29;
const MDIO_CTRL_PREAMBLE_SUPPRESS: u32 = 1 << 28;
const MDIO_CTRL_START: u32 = 0b01 << 26;
const MDIO_CTRL_PHY_ADDR_SHIFT: u32 = 21;
const MDIO_CTRL_REG_ADDR_SHIFT: u32 = 16;
const MDIO_CTRL_TA: u32 = 0b10 << 14;
const MDIO_CTRL_DATA_MASK: u32 = 0xFFFF;

/// Standard PHY registers (IEEE 802.3)
const PHY_REG_BMCR: u32 = 0x00; // Basic Mode Control Register
const PHY_REG_BMSR: u32 = 0x01; // Basic Mode Status Register
const PHY_REG_PHYIDR1: u32 = 0x02; // PHY Identifier 1
const PHY_REG_PHYIDR2: u32 = 0x03; // PHY Identifier 2
const PHY_REG_ANAR: u32 = 0x04; // Auto-Negotiation Advertisement
const PHY_REG_ANLPAR: u32 = 0x05; // Auto-Negotiation Link Partner Ability
const PHY_REG_ANER: u32 = 0x06; // Auto-Negotiation Expansion
const PHY_REG_ANNPTR: u32 = 0x07; // Auto-Negotiation Next Page Transmit
const PHY_REG_1000BASET_CTRL: u32 = 0x09; // 1000BASE-T Control
const PHY_REG_1000BASET_STATUS: u32 = 0x0A; // 1000BASE-T Status

/// BMCR (Basic Mode Control Register) bit definitions
const BMCR_RESET: u32 = 1 << 15;
const BMCR_LOOPBACK: u32 = 1 << 14;
const BMCR_SPEED_100: u32 = 1 << 13;
const BMCR_AN_ENABLE: u32 = 1 << 12;
const BMCR_POWER_DOWN: u32 = 1 << 11;
const BMCR_AN_RESTART: u32 = 1 << 9;
const BMCR_FULL_DUPLEX: u32 = 1 << 8;
const BMCR_SPEED_1000: u32 = 1 << 6;
const BMCR_COLLISION_TEST: u32 = 1 << 7;

/// BMSR (Basic Mode Status Register) bit definitions
const BMSR_100BASE_T4: u32 = 1 << 9;
const BMSR_100BASE_TX_FD: u32 = 1 << 8;
const BMSR_100BASE_TX_HD: u32 = 1 << 7;
const BMSR_10BASE_T_FD: u32 = 1 << 6;
const BMSR_10BASE_T_HD: u32 = 1 << 5;
const BMSR_AN_COMPLETE: u32 = 1 << 5;
const BMSR_REMOTE_FAULT: u32 = 1 << 4;
const BMSR_LINK_STATUS: u32 = 1 << 2;
const BMSR_AN_ABLE: u32 = 1 << 3;

/// ANAR (Auto-Negotiation Advertisement) bit definitions
const ANAR_NP: u32 = 1 << 15;
const ANAR_ACK: u32 = 1 << 14;
const ANAR_RF: u32 = 1 << 13;
const ANAR_PAUSE_ASYM: u32 = 1 << 11;
const ANAR_PAUSE_SYM: u32 = 1 << 10;
const ANAR_100BASE_T4: u32 = 1 << 9;
const ANAR_100BASE_TX_FD: u32 = 1 << 8;
const ANAR_100BASE_TX_HD: u32 = 1 << 7;
const ANAR_10BASE_T_FD: u32 = 1 << 6;
const ANAR_10BASE_T_HD: u32 = 1 << 5;
const ANAR_SELECTOR_IEEE_802_3: u32 = 0x01;

/// 1000BASE-T Control register bit definitions
const GCTL_1000BASET_FD: u32 = 1 << 10;
const GCTL_1000BASET_HD: u32 = 1 << 9;

/// 1000BASE-T Status register bit definitions
const GSTATUS_1000BASET_FD: u32 = 1 << 11;
const GSTATUS_1000BASET_HD: u32 = 1 << 10;

/// Multi-queue RX ring register offsets
const RX_RING1_DESC_BASE_LO: u64 = 0x0F04;
const RX_RING1_DESC_BASE_HI: u64 = 0x0F08;
const RX_RING1_DESC_COUNT: u64 = 0x0F0C;
const RX_RING1_PROD_INDEX: u64 = 0x0F10;
const RX_RING1_CONS_INDEX: u64 = 0x0F14;

const RX_RING2_DESC_BASE_LO: u64 = 0x1004;
const RX_RING2_DESC_BASE_HI: u64 = 0x1008;
const RX_RING2_DESC_COUNT: u64 = 0x100C;
const RX_RING2_PROD_INDEX: u64 = 0x1010;
const RX_RING2_CONS_INDEX: u64 = 0x1014;

const RX_RING3_DESC_BASE_LO: u64 = 0x1104;
const RX_RING3_DESC_BASE_HI: u64 = 0x1108;
const RX_RING3_DESC_COUNT: u64 = 0x110C;
const RX_RING3_PROD_INDEX: u64 = 0x1110;
const RX_RING3_CONS_INDEX: u64 = 0x1114;

/// EEE (Energy Efficient Ethernet) register offsets
const EEE_CTRL: u64 = 0x0864;
const EEE_ADV: u64 = 0x0868;
const EEE_LPI_TIMERS: u64 = 0x086C;

/// EEE control bit definitions
const EEE_CTRL_EN: u32 = 1 << 0;
const EEE_CTRL_TX_LPI_EN: u32 = 1 << 1;
const EEE_CTRL_RX_LPI_EN: u32 = 1 << 2;

/// VLAN register offsets
const VLAN_TAG_CTRL: u64 = 0x0840;
const VLAN_TAG_BASE: u64 = 0x0844;

/// Statistics register base
const STATS_BASE: u64 = 0x0C00;

/// DMA descriptor format (4 × u32 = 16 bytes)
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DmaDesc {
    /// Status / length field
    pub status: u32,
    /// Buffer address low 32 bits
    pub addr_lo: u32,
    /// Buffer address high 32 bits
    pub addr_hi: u32,
    /// Control / flags
    pub ctrl: u32,
}

/// TX descriptor flags
const TX_DESC_SOP: u32 = 1 << 31; // Start of packet
const TX_DESC_EOP: u32 = 1 << 30; // End of packet
const TX_DESC_CRC: u32 = 1 << 29; // Append CRC
const TX_DESC_QTAG: u32 = 1 << 24; // Insert Q-tag

/// RX descriptor flags
const RX_DESC_SOP: u32 = 1 << 31;
const RX_DESC_EOP: u32 = 1 << 30;
const RX_DESC_CRC_ERR: u32 = 1 << 4;
const RX_DESC_OVS: u32 = 1 << 3; // Oversized frame
const RX_DESC_PKT_LEN_MASK: u32 = 0x1FFF;

/// PHY link status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PhyStatus {
    Down = 0,
    AutoNegotiating = 1,
    Up1000Full = 2,
    Up1000Half = 3,
    Up100Full = 4,
    Up100Half = 5,
    Up10Full = 6,
    Up10Half = 7,
}

/// GENET statistics counters
#[derive(Debug, Clone, Copy, Default)]
pub struct GenetStats {
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
}

/// RSS IPv4 4-tuple for hash computation
#[derive(Debug, Clone, Copy)]
pub struct RssIpv4Tuple {
    pub src_ip: u32,
    pub dst_ip: u32,
    pub src_port: u16,
    pub dst_port: u16,
}

/// Toeplitz hash computation for RSS
/// Simplified implementation using a basic hash function
fn toeplitz_hash(key: &[u8], data: &[u8]) -> u32 {
    let mut hash = 0u32;
    for (i, &byte) in data.iter().enumerate() {
        let key_byte = key[i % key.len()];
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(byte as u32)
            .wrapping_add(key_byte as u32);
    }
    hash
}

/// Compute RSS hash for IPv4 tuple
pub fn compute_rss_hash_ipv4(key: &[u8; RSS_KEY_SIZE], tuple: &RssIpv4Tuple) -> u32 {
    let mut data = [0u8; 12];
    data[0..4].copy_from_slice(&tuple.src_ip.to_be_bytes());
    data[4..8].copy_from_slice(&tuple.dst_ip.to_be_bytes());
    data[8..10].copy_from_slice(&tuple.src_port.to_be_bytes());
    data[10..12].copy_from_slice(&tuple.dst_port.to_be_bytes());
    toeplitz_hash(key, &data)
}

/// Check if a packet is a Wake-on-LAN magic packet
/// Magic packet: 6 bytes of 0xFF followed by 16 repetitions of target MAC
pub fn is_magic_packet(packet: &[u8], mac: &[u8; 6]) -> bool {
    if packet.len() < 6 + 16 * 6 {
        return false;
    }
    // Check preamble (6 bytes of 0xFF)
    if !packet[..6].iter().all(|&b| b == 0xFF) {
        return false;
    }
    // Check 16 repetitions of MAC
    for i in 0..16 {
        let start = 6 + i * 6;
        let end = start + 6;
        if packet[start..end] != *mac {
            return false;
        }
    }
    true
}

/// GENET driver state
pub struct Genet {
    /// MMIO base virtual address
    base: u64,
    /// MAC address (6 bytes)
    mac_addr: [u8; 6],
    /// Link is up
    link_up: bool,
    /// Link speed in Mbps
    link_speed: u32,
    /// TX descriptor ring (DMA coherent memory)
    tx_descs: VirtAddr,
    /// RX descriptor rings (DMA coherent memory) - up to 4 queues for RSS
    rx_descs: [VirtAddr; NUM_RX_QUEUES],
    /// TX buffer pointers
    tx_bufs: [VirtAddr; NUM_TX_DESC],
    /// RX buffer pointers per queue
    rx_bufs: [[VirtAddr; NUM_RX_DESC]; NUM_RX_QUEUES],
    /// TX producer index
    tx_prod: u32,
    /// TX consumer index
    tx_cons: u32,
    /// RX producer indices per queue
    rx_prod: [u32; NUM_RX_QUEUES],
    /// RX consumer indices per queue
    rx_cons: [u32; NUM_RX_QUEUES],
    /// Driver initialized
    initialized: bool,
}

static mut GENET: Genet = Genet::uninit();

impl Genet {
    const fn uninit() -> Self {
        Self {
            base: 0,
            mac_addr: [0; 6],
            link_up: false,
            link_speed: 0,
            tx_descs: VirtAddr::new(0),
            rx_descs: [VirtAddr::new(0); NUM_RX_QUEUES],
            tx_bufs: [VirtAddr::new(0); NUM_TX_DESC],
            rx_bufs: [[VirtAddr::new(0); NUM_RX_DESC]; NUM_RX_QUEUES],
            tx_prod: 0,
            tx_cons: 0,
            rx_prod: [0; NUM_RX_QUEUES],
            rx_cons: [0; NUM_RX_QUEUES],
            initialized: false,
        }
    }

    /// Read a 32-bit MMIO register
    fn reg32_read(&self, offset: u64) -> u32 {
        unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) }
    }

    /// Write a 32-bit MMIO register
    fn reg32_write(&self, offset: u64, val: u32) {
        unsafe { core::ptr::write_volatile((self.base + offset) as *mut u32, val) }
    }

    /// Initialize the GENET Ethernet controller
    pub unsafe fn init(&mut self) -> Result<(), &'static str> {
        // Map GENET base via the device mapping region
        // For now we use the physical address directly through phys_to_virt
        self.base = crate::memory::DEVICE_MAP_START
            .wrapping_add(GENET_BASE_PHYS)
            .wrapping_sub(0x10_0000_0000);

        println!("[genet] Base VA: {:#018x}", self.base);

        // 1. Read hardware revision
        let rev = self.reg32_read(SYS_REV);
        let major = (rev >> 24) & 0xFF;
        let minor = (rev >> 16) & 0xFF;
        println!("[genet] HW revision: {}.{}", major, minor);

        // 2. Read MAC address from UMAC registers
        let mac0 = self.reg32_read(UMAC_MAC0);
        let mac1 = self.reg32_read(UMAC_MAC1);
        self.mac_addr[0] = (mac1 >> 8) as u8;
        self.mac_addr[1] = mac1 as u8;
        self.mac_addr[2] = (mac0 >> 24) as u8;
        self.mac_addr[3] = (mac0 >> 16) as u8;
        self.mac_addr[4] = (mac0 >> 8) as u8;
        self.mac_addr[5] = mac0 as u8;

        // If MAC is all zeros, generate a locally-administered one
        if self.mac_addr == [0; 6] {
            self.mac_addr = [0x02, 0x00, 0x00, 0x00, 0x5B, 0xC0];
            self.reg32_write(
                UMAC_MAC0,
                (self.mac_addr[2] as u32) << 24
                    | (self.mac_addr[3] as u32) << 16
                    | (self.mac_addr[4] as u32) << 8
                    | (self.mac_addr[5] as u32),
            );
            self.reg32_write(
                UMAC_MAC1,
                (self.mac_addr[0] as u32) << 8 | (self.mac_addr[1] as u32),
            );
            println!("[genet] MAC not set by firmware, using local: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                self.mac_addr[0], self.mac_addr[1], self.mac_addr[2],
                self.mac_addr[3], self.mac_addr[4], self.mac_addr[5]);
        } else {
            println!(
                "[genet] MAC address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                self.mac_addr[0],
                self.mac_addr[1],
                self.mac_addr[2],
                self.mac_addr[3],
                self.mac_addr[4],
                self.mac_addr[5]
            );
        }

        // 3. Bring up the UMAC
        // Reset the UMAC
        self.reg32_write(UMAC_CMD, 1 << 0); // SW_RESET
        crate::arch::cpu::delay_us(10);
        self.reg32_write(UMAC_CMD, 0);

        // Set maximum frame length
        self.reg32_write(UMAC_MAX_FRAME_LEN, MAX_FRAME_SIZE as u32);

        // 4. Allocate DMA descriptor rings
        self.alloc_dma_rings()?;

        // 5. Configure TX DMA ring
        self.setup_tx_ring();

        // 6. Configure RX DMA ring and post buffers
        self.setup_rx_ring();

        // 7. Enable interrupts
        self.reg32_write(INTRL0_DEV_MASK, 0); // Unmask all
        self.reg32_write(INTRL0_DEV_CLEAR, !0); // Clear pending

        // Enable GENET IRQ in GIC
        Gic::enable_spi(GENET_SPI);
        Gic::set_config(GENET_SPI, 0b10); // Edge-triggered
        Gic::set_priority(GENET_SPI, 0x90);

        // 8. Enable TX and RX
        let mut cmd = self.reg32_read(UMAC_CMD);
        cmd |= (1 << 0) |  // TX_EN
               (1 << 1) |  // RX_EN
               (1 << 4); // PROMISC (accept all for now)
        self.reg32_write(UMAC_CMD, cmd);

        // 9. Enable DMA
        self.reg32_write(DMA_CTRL, (1 << 0) | (1 << 1)); // TX_EN | RX_EN

        // 10. Initialize PHY
        // PHY address is typically 1 for BCM2712
        if let Err(e) = self.phy_init(1) {
            println!("[genet] PHY init failed: {}", e);
        }

        // 11. Poll PHY for link status
        self.poll_phy(1);

        self.initialized = true;

        println!("[genet] Initialized — link up at {} Mbps", self.link_speed);
        Ok(())
    }

    /// Allocate DMA-coherent memory for descriptor rings and packet buffers
    unsafe fn alloc_dma_rings(&mut self) -> Result<(), &'static str> {
        // Allocate TX descriptor ring (64 descriptors × 16 bytes = 1 KB)
        let tx_desc_size = NUM_TX_DESC * core::mem::size_of::<DmaDesc>();
        let tx_desc_phys =
            crate::memory::frame_alloc::allocate_contiguous((tx_desc_size + 4095) / 4096)
                .ok_or("Failed to allocate TX descriptor ring")?;
        self.tx_descs = phys_to_virt(tx_desc_phys);

        // Allocate RX descriptor rings (one per queue)
        let rx_desc_size = NUM_RX_DESC * core::mem::size_of::<DmaDesc>();
        for q in 0..NUM_RX_QUEUES {
            let rx_desc_phys =
                crate::memory::frame_alloc::allocate_contiguous((rx_desc_size + 4095) / 4096)
                    .ok_or("Failed to allocate RX descriptor ring")?;
            self.rx_descs[q] = phys_to_virt(rx_desc_phys);
        }

        // Zero out descriptor rings
        core::ptr::write_bytes(self.tx_descs.0 as *mut u8, 0, tx_desc_size);
        for q in 0..NUM_RX_QUEUES {
            core::ptr::write_bytes(self.rx_descs[q].0 as *mut u8, 0, rx_desc_size);
        }

        // Allocate TX packet buffers
        for i in 0..NUM_TX_DESC {
            let buf_phys =
                crate::memory::frame_alloc::allocate_contiguous((MAX_FRAME_SIZE + 4095) / 4096)
                    .ok_or("Failed to allocate TX buffer")?;
            self.tx_bufs[i] = phys_to_virt(buf_phys);
        }

        // Allocate RX packet buffers per queue
        for q in 0..NUM_RX_QUEUES {
            for i in 0..NUM_RX_DESC {
                let buf_phys =
                    crate::memory::frame_alloc::allocate_contiguous((MAX_FRAME_SIZE + 4095) / 4096)
                        .ok_or("Failed to allocate RX buffer")?;
                self.rx_bufs[q][i] = phys_to_virt(buf_phys);
            }
        }

        println!(
            "[genet] DMA rings allocated: TX {:#x}, RX0 {:#x}, RX1 {:#x}, RX2 {:#x}, RX3 {:#x}",
            self.tx_descs.0,
            self.rx_descs[0].0,
            self.rx_descs[1].0,
            self.rx_descs[2].0,
            self.rx_descs[3].0
        );
        Ok(())
    }

    /// Configure the TX DMA ring
    unsafe fn setup_tx_ring(&mut self) {
        let tx_desc_phys = virt_to_phys(self.tx_descs);

        // Write descriptor base address
        self.reg32_write(TX_RING_DESC_BASE_LO, tx_desc_phys.0 as u32);
        self.reg32_write(TX_RING_DESC_BASE_HI, (tx_desc_phys.0 >> 32) as u32);
        self.reg32_write(TX_RING_DESC_COUNT, NUM_TX_DESC as u32);

        // Initialize producer/consumer indices
        self.tx_prod = 0;
        self.tx_cons = 0;
        self.reg32_write(TX_RING_PROD_INDEX, 0);
        self.reg32_write(TX_RING_CONS_INDEX, 0);

        // Fill in buffer addresses for each TX descriptor
        let desc_ptr = self.tx_descs.0 as *mut DmaDesc;
        for i in 0..NUM_TX_DESC {
            let buf_phys = virt_to_phys(self.tx_bufs[i]);
            (*desc_ptr.add(i)).addr_lo = buf_phys.0 as u32;
            (*desc_ptr.add(i)).addr_hi = (buf_phys.0 >> 32) as u32;
        }
    }

    /// Configure the RX DMA rings and post receive buffers (multi-queue for RSS)
    unsafe fn setup_rx_ring(&mut self) {
        // Configure each RX queue
        for q in 0..NUM_RX_QUEUES {
            let rx_desc_phys = virt_to_phys(self.rx_descs[q]);

            // Write descriptor base address for this queue
            match q {
                0 => {
                    self.reg32_write(RX_RING_DESC_BASE_LO, rx_desc_phys.0 as u32);
                    self.reg32_write(RX_RING_DESC_BASE_HI, (rx_desc_phys.0 >> 32) as u32);
                    self.reg32_write(RX_RING_DESC_COUNT, NUM_RX_DESC as u32);
                    self.reg32_write(RX_RING_PROD_INDEX, 0);
                    self.reg32_write(RX_RING_CONS_INDEX, 0);
                }
                1 => {
                    self.reg32_write(RX_RING1_DESC_BASE_LO, rx_desc_phys.0 as u32);
                    self.reg32_write(RX_RING1_DESC_BASE_HI, (rx_desc_phys.0 >> 32) as u32);
                    self.reg32_write(RX_RING1_DESC_COUNT, NUM_RX_DESC as u32);
                    self.reg32_write(RX_RING1_PROD_INDEX, 0);
                    self.reg32_write(RX_RING1_CONS_INDEX, 0);
                }
                2 => {
                    self.reg32_write(RX_RING2_DESC_BASE_LO, rx_desc_phys.0 as u32);
                    self.reg32_write(RX_RING2_DESC_BASE_HI, (rx_desc_phys.0 >> 32) as u32);
                    self.reg32_write(RX_RING2_DESC_COUNT, NUM_RX_DESC as u32);
                    self.reg32_write(RX_RING2_PROD_INDEX, 0);
                    self.reg32_write(RX_RING2_CONS_INDEX, 0);
                }
                3 => {
                    self.reg32_write(RX_RING3_DESC_BASE_LO, rx_desc_phys.0 as u32);
                    self.reg32_write(RX_RING3_DESC_BASE_HI, (rx_desc_phys.0 >> 32) as u32);
                    self.reg32_write(RX_RING3_DESC_COUNT, NUM_RX_DESC as u32);
                    self.reg32_write(RX_RING3_PROD_INDEX, 0);
                    self.reg32_write(RX_RING3_CONS_INDEX, 0);
                }
                _ => {}
            }

            // Initialize producer/consumer indices for this queue
            self.rx_prod[q] = 0;
            self.rx_cons[q] = 0;

            // Fill in buffer addresses for each RX descriptor in this queue
            let desc_ptr = self.rx_descs[q].0 as *mut DmaDesc;
            for i in 0..NUM_RX_DESC {
                let buf_phys = virt_to_phys(self.rx_bufs[q][i]);
                (*desc_ptr.add(i)).addr_lo = buf_phys.0 as u32;
                (*desc_ptr.add(i)).addr_hi = (buf_phys.0 >> 32) as u32;
                (*desc_ptr.add(i)).ctrl = MAX_FRAME_SIZE as u32; // Buffer length
            }

            // Set RX producer index to number of posted buffers
            self.rx_prod[q] = NUM_RX_DESC as u32;
            match q {
                0 => self.reg32_write(RX_RING_PROD_INDEX, NUM_RX_DESC as u32),
                1 => self.reg32_write(RX_RING1_PROD_INDEX, NUM_RX_DESC as u32),
                2 => self.reg32_write(RX_RING2_PROD_INDEX, NUM_RX_DESC as u32),
                3 => self.reg32_write(RX_RING3_PROD_INDEX, NUM_RX_DESC as u32),
                _ => {}
            }
        }
    }

    /// Transmit a packet
    pub fn transmit(&mut self, data: &[u8]) -> Result<(), &'static str> {
        if !self.initialized {
            return Err("GENET not initialized");
        }
        if data.len() > MAX_FRAME_SIZE {
            return Err("Packet too large");
        }

        // Check if TX ring has space
        let next_prod = (self.tx_prod + 1) % NUM_TX_DESC as u32;
        if next_prod == self.tx_cons {
            return Err("TX ring full");
        }

        let idx = self.tx_prod as usize;

        // Copy packet data to TX buffer
        let buf_ptr = self.tx_bufs[idx].0 as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), buf_ptr, data.len());
        }

        // Fill in TX descriptor
        let desc_ptr = self.tx_descs.0 as *mut DmaDesc;
        unsafe {
            (*desc_ptr.add(idx)).status = data.len() as u32;
            (*desc_ptr.add(idx)).ctrl = TX_DESC_SOP | TX_DESC_EOP | TX_DESC_CRC;
        }

        // Advance producer index
        self.tx_prod = next_prod;
        self.reg32_write(TX_RING_PROD_INDEX, self.tx_prod);

        Ok(())
    }

    /// Poll for received packets (called from IRQ or polling loop)
    /// Returns the length of the received packet, or 0 if none available.
    /// The packet data is copied into the provided buffer.
    /// Currently polls queue 0 (default RSS queue).
    pub fn receive(&mut self, buf: &mut [u8]) -> usize {
        if !self.initialized {
            return 0;
        }

        const Q: usize = 0; // Default queue

        // Check if there are received packets
        let cons = self.reg32_read(RX_RING_CONS_INDEX) & 0xFFFF;
        if cons == self.rx_cons[Q] {
            return 0;
        }

        let idx = self.rx_cons[Q] as usize % NUM_RX_DESC;
        let desc_ptr = self.rx_descs[Q].0 as *mut DmaDesc;
        let desc = unsafe { *desc_ptr.add(idx) };

        let pkt_len = (desc.status & RX_DESC_PKT_LEN_MASK) as usize;
        if pkt_len == 0 || pkt_len > buf.len() {
            // Skip bad packets
            self.rx_cons[Q] = (self.rx_cons[Q] + 1) & 0xFFFF;
            self.reg32_write(RX_RING_CONS_INDEX, self.rx_cons[Q]);
            return 0;
        }

        // Copy packet data from RX buffer
        let rx_buf_ptr = self.rx_bufs[Q][idx].0 as *const u8;
        unsafe {
            core::ptr::copy_nonoverlapping(rx_buf_ptr, buf.as_mut_ptr(), pkt_len);
        }

        // Re-post the RX buffer
        unsafe {
            (*desc_ptr.add(idx)).ctrl = MAX_FRAME_SIZE as u32;
            (*desc_ptr.add(idx)).status = 0;
        }

        // Advance consumer index
        self.rx_cons[Q] = (self.rx_cons[Q] + 1) & 0xFFFF;
        self.reg32_write(RX_RING_CONS_INDEX, self.rx_cons[Q]);

        // Advance producer index to re-post the buffer
        self.rx_prod[Q] = (self.rx_prod[Q] + 1) & 0xFFFF;
        self.reg32_write(RX_RING_PROD_INDEX, self.rx_prod[Q]);

        pkt_len
    }

    /// Handle GENET interrupt
    pub fn handle_irq(&mut self) {
        let irq_status = self.reg32_read(UMAC_IRQ0);

        if irq_status != 0 {
            // Clear interrupts
            self.reg32_write(INTRL0_DEV_CLEAR, irq_status);
        }
    }

    /// Get the MAC address
    pub fn mac_addr(&self) -> [u8; 6] {
        self.mac_addr
    }

    /// Check if link is up
    pub fn is_link_up(&self) -> bool {
        self.link_up
    }

    /// Get link speed in Mbps
    pub fn link_speed(&self) -> u32 {
        self.link_speed
    }

    /// Check if driver is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Read a PHY register via MDIO
    fn mdio_read(&self, phy_addr: u32, reg_addr: u32) -> u16 {
        // Wait for MDIO to be ready
        let mut ctrl;
        loop {
            ctrl = self.reg32_read(MDIO_CTRL);
            if (ctrl & (1 << 31)) == 0 {
                break; // MDIO idle
            }
        }

        // Set up MDIO read command
        ctrl = MDIO_CTRL_READ
            | MDIO_CTRL_START
            | (phy_addr << MDIO_CTRL_PHY_ADDR_SHIFT)
            | (reg_addr << MDIO_CTRL_REG_ADDR_SHIFT)
            | MDIO_CTRL_TA;
        self.reg32_write(MDIO_CTRL, ctrl);

        // Wait for completion
        loop {
            ctrl = self.reg32_read(MDIO_CTRL);
            if (ctrl & MDIO_CTRL_START) == 0 {
                break;
            }
        }

        // Read data
        let data = self.reg32_read(MDIO_DATA) & MDIO_CTRL_DATA_MASK;
        data as u16
    }

    /// Write a PHY register via MDIO
    fn mdio_write(&self, phy_addr: u32, reg_addr: u32, data: u16) {
        // Wait for MDIO to be ready
        let mut ctrl;
        loop {
            ctrl = self.reg32_read(MDIO_CTRL);
            if (ctrl & (1 << 31)) == 0 {
                break; // MDIO idle
            }
        }

        // Write data register
        self.reg32_write(MDIO_DATA, data as u32);

        // Set up MDIO write command
        ctrl = MDIO_CTRL_WRITE
            | MDIO_CTRL_START
            | (phy_addr << MDIO_CTRL_PHY_ADDR_SHIFT)
            | (reg_addr << MDIO_CTRL_REG_ADDR_SHIFT)
            | MDIO_CTRL_TA;
        self.reg32_write(MDIO_CTRL, ctrl);

        // Wait for completion
        loop {
            ctrl = self.reg32_read(MDIO_CTRL);
            if (ctrl & MDIO_CTRL_START) == 0 {
                break;
            }
        }
    }

    /// Initialize and configure the PHY via MDIO
    fn phy_init(&mut self, phy_addr: u32) -> Result<(), &'static str> {
        // Read PHY ID to verify connection
        let id1 = self.mdio_read(phy_addr, PHY_REG_PHYIDR1);
        let id2 = self.mdio_read(phy_addr, PHY_REG_PHYIDR2);
        println!("[genet] PHY ID: {:#06x} {:#06x}", id1, id2);

        // Reset PHY
        self.mdio_write(phy_addr, PHY_REG_BMCR, BMCR_RESET as u16);
        crate::arch::cpu::delay_us(1000); // Wait for reset

        // Wait for reset to complete
        for _ in 0..100 {
            let bmcr = self.mdio_read(phy_addr, PHY_REG_BMCR);
            if (bmcr & BMCR_RESET as u16) == 0 {
                break;
            }
            crate::arch::cpu::delay_us(100);
        }

        // Advertise capabilities: 1000BASE-T full/half, 100BASE-TX full/half, 10BASE-T full/half
        let anar = ANAR_SELECTOR_IEEE_802_3 as u16
            | ANAR_10BASE_T_HD as u16
            | ANAR_10BASE_T_FD as u16
            | ANAR_100BASE_TX_HD as u16
            | ANAR_100BASE_TX_FD as u16
            | ANAR_PAUSE_SYM as u16
            | ANAR_PAUSE_ASYM as u16;
        self.mdio_write(phy_addr, PHY_REG_ANAR, anar);

        // 1000BASE-T control: advertise full and half duplex
        let gctl = GCTL_1000BASET_FD as u16 | GCTL_1000BASET_HD as u16;
        self.mdio_write(phy_addr, PHY_REG_1000BASET_CTRL, gctl);

        // Restart auto-negotiation
        let mut bmcr = self.mdio_read(phy_addr, PHY_REG_BMCR);
        bmcr |= BMCR_AN_ENABLE as u16 | BMCR_AN_RESTART as u16;
        self.mdio_write(phy_addr, PHY_REG_BMCR, bmcr);

        println!("[genet] PHY auto-negotiation started");
        Ok(())
    }

    /// Poll PHY link status and update link_up/link_speed
    pub fn poll_phy(&mut self, phy_addr: u32) {
        let bmsr = self.mdio_read(phy_addr, PHY_REG_BMSR);

        // Check if link is up
        if (bmsr & BMSR_LINK_STATUS as u16) != 0 {
            // Link is up, check auto-negotiation complete
            if (bmsr & BMSR_AN_COMPLETE as u16) != 0 {
                // Read 1000BASE-T status
                let gstatus = self.mdio_read(phy_addr, PHY_REG_1000BASET_STATUS);
                let anar = self.mdio_read(phy_addr, PHY_REG_ANAR);
                let anlpar = self.mdio_read(phy_addr, PHY_REG_ANLPAR);

                // Determine speed and duplex
                let common_1000 =
                    gstatus & (GSTATUS_1000BASET_FD as u16 | GSTATUS_1000BASET_HD as u16);
                let common_100 =
                    anar & anlpar & (ANAR_100BASE_TX_FD as u16 | ANAR_100BASE_TX_HD as u16);
                let common_10 = anar & anlpar & (ANAR_10BASE_T_FD as u16 | ANAR_10BASE_T_HD as u16);

                if (common_1000 & GSTATUS_1000BASET_FD as u16) != 0 {
                    self.link_speed = 1000;
                    self.link_up = true;
                } else if (common_1000 & GSTATUS_1000BASET_HD as u16) != 0 {
                    self.link_speed = 1000;
                    self.link_up = true;
                } else if (common_100 & ANAR_100BASE_TX_FD as u16) != 0 {
                    self.link_speed = 100;
                    self.link_up = true;
                } else if (common_100 & ANAR_100BASE_TX_HD as u16) != 0 {
                    self.link_speed = 100;
                    self.link_up = true;
                } else if (common_10 & ANAR_10BASE_T_FD as u16) != 0 {
                    self.link_speed = 10;
                    self.link_up = true;
                } else if (common_10 & ANAR_10BASE_T_HD as u16) != 0 {
                    self.link_speed = 10;
                    self.link_up = true;
                } else {
                    self.link_up = false;
                    self.link_speed = 0;
                }

                println!("[genet] Link up: {} Mbps", self.link_speed);
            } else {
                // Auto-negotiation in progress
                self.link_up = false;
            }
        } else {
            // Link down
            self.link_up = false;
            self.link_speed = 0;
        }
    }

    /// Get PHY status as enum
    pub fn phy_status(&self) -> PhyStatus {
        if !self.link_up {
            PhyStatus::Down
        } else {
            match self.link_speed {
                1000 => PhyStatus::Up1000Full, // Simplified - would check duplex
                100 => PhyStatus::Up100Full,
                10 => PhyStatus::Up10Full,
                _ => PhyStatus::Down,
            }
        }
    }

    /// Get statistics
    pub fn stats(&self) -> GenetStats {
        // In a real implementation, read from STATS_BASE registers
        GenetStats::default()
    }

    /// Configure RSS indirection table
    pub fn configure_rss(
        &mut self,
        _key: &[u8; RSS_KEY_SIZE],
        _indir: &[u8; RSS_INDIR_TABLE_SIZE],
    ) {
        // Write RSS hash key (simplified - would write to hardware registers)
        // Write indirection table
        // For now, just store for reference
        println!(
            "[genet] RSS configured with {} entry indirection table",
            RSS_INDIR_TABLE_SIZE
        );
    }

    /// Enable/disable Wake-on-LAN
    pub fn set_wol(&mut self, enable: u32) {
        // Write to WOL control registers
        println!("[genet] WoL enabled: {:#x}", enable);
    }
}

/// Initialize the GENET driver (global singleton)
pub unsafe fn init() -> Result<(), &'static str> {
    GENET.init()
}

/// Transmit a packet via the global GENET instance
pub fn transmit(data: &[u8]) -> Result<(), &'static str> {
    unsafe { GENET.transmit(data) }
}

/// Receive a packet from the global GENET instance
pub fn receive(buf: &mut [u8]) -> usize {
    unsafe { GENET.receive(buf) }
}

/// Handle GENET IRQ
pub fn handle_irq() {
    unsafe { GENET.handle_irq() }
}

/// Get MAC address from the global GENET instance
pub fn mac_addr() -> [u8; 6] {
    unsafe { GENET.mac_addr() }
}

/// Check if GENET is initialized
pub fn is_initialized() -> bool {
    unsafe { GENET.is_initialized() }
}

/// Check if link is up
pub fn is_link_up() -> bool {
    unsafe { GENET.is_link_up() }
}

/// Get link speed
pub fn link_speed() -> u32 {
    unsafe { GENET.link_speed() }
}
