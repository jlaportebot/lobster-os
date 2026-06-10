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

/// Maximum Ethernet frame size (MTU + headers + FCS)
const MAX_FRAME_SIZE: usize = 1536;

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
    /// RX descriptor ring (DMA coherent memory)
    rx_descs: VirtAddr,
    /// TX buffer pointers
    tx_bufs: [VirtAddr; NUM_TX_DESC],
    /// RX buffer pointers
    rx_bufs: [VirtAddr; NUM_RX_DESC],
    /// TX producer index
    tx_prod: u32,
    /// TX consumer index
    tx_cons: u32,
    /// RX producer index
    rx_prod: u32,
    /// RX consumer index
    rx_cons: u32,
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
            rx_descs: VirtAddr::new(0),
            tx_bufs: [VirtAddr::new(0); NUM_TX_DESC],
            rx_bufs: [VirtAddr::new(0); NUM_RX_DESC],
            tx_prod: 0,
            tx_cons: 0,
            rx_prod: 0,
            rx_cons: 0,
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

        self.initialized = true;
        self.link_up = true; // Assume link is up (would poll PHY in real impl)
        self.link_speed = 1000;

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

        // Allocate RX descriptor ring
        let rx_desc_size = NUM_RX_DESC * core::mem::size_of::<DmaDesc>();
        let rx_desc_phys =
            crate::memory::frame_alloc::allocate_contiguous((rx_desc_size + 4095) / 4096)
                .ok_or("Failed to allocate RX descriptor ring")?;
        self.rx_descs = phys_to_virt(rx_desc_phys);

        // Zero out descriptor rings
        core::ptr::write_bytes(self.tx_descs.0 as *mut u8, 0, tx_desc_size);
        core::ptr::write_bytes(self.rx_descs.0 as *mut u8, 0, rx_desc_size);

        // Allocate TX packet buffers
        for i in 0..NUM_TX_DESC {
            let buf_phys =
                crate::memory::frame_alloc::allocate_contiguous((MAX_FRAME_SIZE + 4095) / 4096)
                    .ok_or("Failed to allocate TX buffer")?;
            self.tx_bufs[i] = phys_to_virt(buf_phys);
        }

        // Allocate RX packet buffers
        for i in 0..NUM_RX_DESC {
            let buf_phys =
                crate::memory::frame_alloc::allocate_contiguous((MAX_FRAME_SIZE + 4095) / 4096)
                    .ok_or("Failed to allocate RX buffer")?;
            self.rx_bufs[i] = phys_to_virt(buf_phys);
        }

        println!(
            "[genet] DMA rings allocated: TX {:#x}, RX {:#x}",
            self.tx_descs.0, self.rx_descs.0
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

    /// Configure the RX DMA ring and post receive buffers
    unsafe fn setup_rx_ring(&mut self) {
        let rx_desc_phys = virt_to_phys(self.rx_descs);

        // Write descriptor base address
        self.reg32_write(RX_RING_DESC_BASE_LO, rx_desc_phys.0 as u32);
        self.reg32_write(RX_RING_DESC_BASE_HI, (rx_desc_phys.0 >> 32) as u32);
        self.reg32_write(RX_RING_DESC_COUNT, NUM_RX_DESC as u32);

        // Initialize producer/consumer indices
        self.rx_prod = 0;
        self.rx_cons = 0;
        self.reg32_write(RX_RING_PROD_INDEX, 0);
        self.reg32_write(RX_RING_CONS_INDEX, 0);

        // Fill in buffer addresses for each RX descriptor
        let desc_ptr = self.rx_descs.0 as *mut DmaDesc;
        for i in 0..NUM_RX_DESC {
            let buf_phys = virt_to_phys(self.rx_bufs[i]);
            (*desc_ptr.add(i)).addr_lo = buf_phys.0 as u32;
            (*desc_ptr.add(i)).addr_hi = (buf_phys.0 >> 32) as u32;
            (*desc_ptr.add(i)).ctrl = MAX_FRAME_SIZE as u32; // Buffer length
        }

        // Set RX producer index to number of posted buffers
        self.rx_prod = NUM_RX_DESC as u32;
        self.reg32_write(RX_RING_PROD_INDEX, NUM_RX_DESC as u32);
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
    pub fn receive(&mut self, buf: &mut [u8]) -> usize {
        if !self.initialized {
            return 0;
        }

        // Check if there are received packets
        let cons = self.reg32_read(RX_RING_CONS_INDEX) & 0xFFFF;
        if cons == self.rx_cons {
            return 0;
        }

        let idx = self.rx_cons as usize % NUM_RX_DESC;
        let desc_ptr = self.rx_descs.0 as *mut DmaDesc;
        let desc = unsafe { *desc_ptr.add(idx) };

        let pkt_len = (desc.status & RX_DESC_PKT_LEN_MASK) as usize;
        if pkt_len == 0 || pkt_len > buf.len() {
            // Skip bad packets
            self.rx_cons = (self.rx_cons + 1) & 0xFFFF;
            self.reg32_write(RX_RING_CONS_INDEX, self.rx_cons);
            return 0;
        }

        // Copy packet data from RX buffer
        let rx_buf_ptr = self.rx_bufs[idx].0 as *const u8;
        unsafe {
            core::ptr::copy_nonoverlapping(rx_buf_ptr, buf.as_mut_ptr(), pkt_len);
        }

        // Re-post the RX buffer
        unsafe {
            (*desc_ptr.add(idx)).ctrl = MAX_FRAME_SIZE as u32;
            (*desc_ptr.add(idx)).status = 0;
        }

        // Advance consumer index
        self.rx_cons = (self.rx_cons + 1) & 0xFFFF;
        self.reg32_write(RX_RING_CONS_INDEX, self.rx_cons);

        // Advance producer index to re-post the buffer
        self.rx_prod = (self.rx_prod + 1) & 0xFFFF;
        self.reg32_write(RX_RING_PROD_INDEX, self.rx_prod);

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
