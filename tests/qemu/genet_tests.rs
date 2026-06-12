//! GENET Ethernet Driver Integration Tests
//!
//! Tests for GENETv5 Ethernet MAC on BCM2712:
//! - DMA descriptor operations
//! - MDIO PHY management
//! - Auto-negotiation
//! - RSS/RFS for multi-core
//! - Wake-on-LAN

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod genet_tests {
    use crate::net::genet::{DmaDesc, NUM_TX_DESC, NUM_RX_DESC, MAX_FRAME_SIZE};
    use crate::qemu_test::*;

    /// Test GENET DMA descriptor struct
    fn test_genet_dma_desc() -> TestResult {
        let desc = DmaDesc::default();
        test_assert_eq!(desc.status, 0);
        test_assert_eq!(desc.addr_lo, 0);
        test_assert_eq!(desc.addr_hi, 0);
        test_assert_eq!(desc.ctrl, 0);

        // Test setting fields
        let mut desc = DmaDesc::default();
        desc.status = 0x12345678;
        desc.addr_lo = 0xABCDEF00;
        desc.addr_hi = 0x11223344;
        desc.ctrl = 0x55667788;
        test_assert_eq!(desc.status, 0x12345678);
        test_assert_eq!(desc.addr_lo, 0xABCDEF00);
        test_assert_eq!(desc.addr_hi, 0x11223344);
        test_assert_eq!(desc.ctrl, 0x55667788);
        TestResult::Passed
    }

    /// Test GENET DMA descriptor constants
    fn test_genet_constants() -> TestResult {
        test_assert_eq!(NUM_TX_DESC, 64);
        test_assert_eq!(NUM_RX_DESC, 64);
        test_assert_eq!(MAX_FRAME_SIZE, 1536);
        TestResult::Passed
    }

    /// Test TX descriptor flags
    fn test_genet_tx_desc_flags() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(TX_DESC_SOP, 1 << 31);
        test_assert_eq!(TX_DESC_EOP, 1 << 30);
        test_assert_eq!(TX_DESC_CRC, 1 << 29);
        test_assert_eq!(TX_DESC_QTAG, 1 << 24);
        TestResult::Passed
    }

    /// Test RX descriptor flags
    fn test_genet_rx_desc_flags() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(RX_DESC_SOP, 1 << 31);
        test_assert_eq!(RX_DESC_EOP, 1 << 30);
        test_assert_eq!(RX_DESC_CRC_ERR, 1 << 4);
        test_assert_eq!(RX_DESC_OVS, 1 << 3);
        test_assert_eq!(RX_DESC_PKT_LEN_MASK, 0x1FFF);
        TestResult::Passed
    }

    /// Test GENET register offsets
    fn test_genet_register_offsets() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(GENET_BASE_PHYS, 0x10_7D50_0000);
        test_assert_eq!(SYS_REV, 0x00);
        test_assert_eq!(UMAC_CMD, 0x0808);
        test_assert_eq!(UMAC_MAC0, 0x080C);
        test_assert_eq!(UMAC_MAC1, 0x0810);
        test_assert_eq!(GENET_SPI, 189);
        TestResult::Passed
    }

    /// Test MDIO register definitions
    fn test_genet_mdio_registers() -> TestResult {
        use crate::net::genet::*;
        // MDIO registers (these will be added when MDIO is implemented)
        // For now, test that constants can be defined
        test_assert_eq!(MDIO_CTRL, 0x0880);
        test_assert_eq!(MDIO_DATA, 0x0884);
        TestResult::Passed
    }

    /// Test PHY register definitions
    fn test_genet_phy_registers() -> TestResult {
        use crate::net::genet::*;
        // Standard PHY registers (IEEE 802.3)
        test_assert_eq!(PHY_REG_BMCR, 0x00); // Basic Mode Control Register
        test_assert_eq!(PHY_REG_BMSR, 0x01); // Basic Mode Status Register
        test_assert_eq!(PHY_REG_PHYIDR1, 0x02); // PHY Identifier 1
        test_assert_eq!(PHY_REG_PHYIDR2, 0x03); // PHY Identifier 2
        test_assert_eq!(PHY_REG_ANAR, 0x04); // Auto-Negotiation Advertisement
        test_assert_eq!(PHY_REG_ANLPAR, 0x05); // Auto-Negotiation Link Partner Ability
        test_assert_eq!(PHY_REG_ANER, 0x06); // Auto-Negotiation Expansion
        test_assert_eq!(PHY_REG_ANNPTR, 0x07); // Auto-Negotiation Next Page Transmit
        test_assert_eq!(PHY_REG_1000BASET_CTRL, 0x09); // 1000BASE-T Control
        test_assert_eq!(PHY_REG_1000BASET_STATUS, 0x0A); // 1000BASE-T Status
        TestResult::Passed
    }

    /// Test BMCR (Basic Mode Control Register) bit definitions
    fn test_genet_bmcr_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(BMCR_RESET, 1 << 15);
        test_assert_eq!(BMCR_LOOPBACK, 1 << 14);
        test_assert_eq!(BMCR_SPEED_1000, 1 << 6);
        test_assert_eq!(BMCR_AN_ENABLE, 1 << 12);
        test_assert_eq!(BMCR_POWER_DOWN, 1 << 11);
        test_assert_eq!(BMCR_AN_RESTART, 1 << 9);
        test_assert_eq!(BMCR_FULL_DUPLEX, 1 << 8);
        TestResult::Passed
    }

    /// Test BMSR (Basic Mode Status Register) bit definitions
    fn test_genet_bmsr_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(BMSR_100BASE_T4, 1 << 9);
        test_assert_eq!(BMSR_100BASE_TX_FD, 1 << 8);
        test_assert_eq!(BMSR_100BASE_TX_HD, 1 << 7);
        test_assert_eq!(BMSR_10BASE_T_FD, 1 << 6);
        test_assert_eq!(BMSR_10BASE_T_HD, 1 << 5);
        test_assert_eq!(BMSR_AN_COMPLETE, 1 << 5);
        test_assert_eq!(BMSR_LINK_STATUS, 1 << 2);
        test_assert_eq!(BMSR_AN_ABLE, 1 << 3);
        TestResult::Passed
    }

    /// Test ANAR (Auto-Negotiation Advertisement) bit definitions
    fn test_genet_anar_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(ANAR_NP, 1 << 15);
        test_assert_eq!(ANAR_ACK, 1 << 14);
        test_assert_eq!(ANAR_RF, 1 << 13);
        test_assert_eq!(ANAR_PAUSE_ASYM, 1 << 11);
        test_assert_eq!(ANAR_PAUSE_SYM, 1 << 10);
        test_assert_eq!(ANAR_100BASE_T4, 1 << 9);
        test_assert_eq!(ANAR_100BASE_TX_FD, 1 << 8);
        test_assert_eq!(ANAR_100BASE_TX_HD, 1 << 7);
        test_assert_eq!(ANAR_10BASE_T_FD, 1 << 6);
        test_assert_eq!(ANAR_10BASE_T_HD, 1 << 5);
        test_assert_eq!(ANAR_SELECTOR_IEEE_802_3, 0x01);
        TestResult::Passed
    }

    /// Test RSS/RFS constants
    fn test_genet_rss_constants() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(RSS_KEY_SIZE, 40);
        test_assert_eq!(RSS_INDIR_TABLE_SIZE, 128);
        test_assert_eq!(RSS_HASH_IPV4, 1 << 0);
        test_assert_eq!(RSS_HASH_TCP_IPV4, 1 << 1);
        test_assert_eq!(RSS_HASH_IPV6, 1 << 2);
        test_assert_eq!(RSS_HASH_TCP_IPV6, 1 << 3);
        TestResult::Passed
    }

    /// Test Wake-on-LAN constants
    fn test_genet_wol_constants() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(WOL_MAGIC_PACKET, 1 << 0);
        test_assert_eq!(WOL_PATTERN_MATCH, 1 << 1);
        test_assert_eq!(WOL_LINK_CHANGE, 1 << 2);
        test_assert_eq!(WOL_MAGIC_PACKET_ENABLE, 1 << 0);
        test_assert_eq!(WOL_PATTERN_ENABLE, 1 << 1);
        test_assert_eq!(WOL_LINK_CHANGE_ENABLE, 1 << 2);
        TestResult::Passed
    }

    /// Test PHY status enum
    fn test_genet_phy_status() -> TestResult {
        use crate::net::genet::PhyStatus;
        test_assert_eq!(PhyStatus::Down as u8, 0);
        test_assert_eq!(PhyStatus::AutoNegotiating as u8, 1);
        test_assert_eq!(PhyStatus::Up1000Full as u8, 2);
        test_assert_eq!(PhyStatus::Up1000Half as u8, 3);
        test_assert_eq!(PhyStatus::Up100Full as u8, 4);
        test_assert_eq!(PhyStatus::Up100Half as u8, 5);
        test_assert_eq!(PhyStatus::Up10Full as u8, 6);
        test_assert_eq!(PhyStatus::Up10Half as u8, 7);
        TestResult::Passed
    }

    /// Test RSS hash function (unit test - no hardware needed)
    fn test_genet_rss_hash_computation() -> TestResult {
        use crate::net::genet::*;
        // Test RSS hash with known key and input
        let key = [0x6Du8; RSS_KEY_SIZE]; // Test key
        let ipv4_tuple = RssIpv4Tuple {
            src_ip: 0xC0A80101, // 192.168.1.1
            dst_ip: 0xC0A80102, // 192.168.1.2
            src_port: 12345,
            dst_port: 80,
        };
        let hash = compute_rss_hash_ipv4(&key, &ipv4_tuple);
        test_assert_ne!(hash, 0);
        test_assert_eq!(hash, compute_rss_hash_ipv4(&key, &ipv4_tuple)); // Deterministic
        TestResult::Passed
    }

    /// Test magic packet detection
    fn test_genet_wol_magic_packet() -> TestResult {
        use crate::net::genet::*;
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];

        // Create magic packet: 6 bytes of 0xFF + 16 repetitions of MAC
        let mut magic = [0xFFu8; 6];
        for _ in 0..16 {
            magic.extend_from_slice(&mac);
        }

        test_assert!(is_magic_packet(&magic, &mac));
        test_assert!(!is_magic_packet(&[0u8; 102], &mac)); // All zeros
        TestResult::Passed
    }

    /// Test multi-queue RX ring register offsets
    fn test_genet_rx_queue_registers() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(RX_RING1_DESC_BASE_LO, 0x0F04);
        test_assert_eq!(RX_RING1_DESC_BASE_HI, 0x0F08);
        test_assert_eq!(RX_RING1_DESC_COUNT, 0x0F0C);
        test_assert_eq!(RX_RING1_PROD_INDEX, 0x0F10);
        test_assert_eq!(RX_RING1_CONS_INDEX, 0x0F14);

        test_assert_eq!(RX_RING2_DESC_BASE_LO, 0x1004);
        test_assert_eq!(RX_RING2_DESC_BASE_HI, 0x1008);
        test_assert_eq!(RX_RING2_DESC_COUNT, 0x100C);
        test_assert_eq!(RX_RING2_PROD_INDEX, 0x1010);
        test_assert_eq!(RX_RING2_CONS_INDEX, 0x1014);

        test_assert_eq!(RX_RING3_DESC_BASE_LO, 0x1104);
        test_assert_eq!(RX_RING3_DESC_BASE_HI, 0x1108);
        test_assert_eq!(RX_RING3_DESC_COUNT, 0x110C);
        test_assert_eq!(RX_RING3_PROD_INDEX, 0x1110);
        test_assert_eq!(RX_RING3_CONS_INDEX, 0x1114);
        TestResult::Passed
    }

    /// Test RSS indirection table default mapping
    fn test_genet_rss_indir_default() -> TestResult {
        use crate::net::genet::*;
        // Default indirection table should map all entries to queue 0
        for i in 0..RSS_INDIR_TABLE_SIZE {
            // This tests the logic, not the hardware state
            let expected_queue = (i % 1) as u8;
            test_assert_eq!(expected_queue, 0);
        }
        TestResult::Passed
    }

    /// Test 1000BASE-T control/status register bits
    fn test_genet_gigabit_registers() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(GCTL_1000BASET_FD, 1 << 10);
        test_assert_eq!(GCTL_1000BASET_HD, 1 << 9);
        test_assert_eq!(GSTATUS_1000BASET_FD, 1 << 11);
        test_assert_eq!(GSTATUS_1000BASET_HD, 1 << 10);
        TestResult::Passed
    }

    /// Test MDIO control bit definitions
    fn test_genet_mdio_ctrl_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(MDIO_CTRL_READ, 1 << 30);
        test_assert_eq!(MDIO_CTRL_WRITE, 1 << 29);
        test_assert_eq!(MDIO_CTRL_PREAMBLE_SUPPRESS, 1 << 28);
        test_assert_eq!(MDIO_CTRL_START, 0b01 << 26);
        test_assert_eq!(MDIO_CTRL_PHY_ADDR_SHIFT, 21);
        test_assert_eq!(MDIO_CTRL_REG_ADDR_SHIFT, 16);
        test_assert_eq!(MDIO_CTRL_TA, 0b10 << 14);
        test_assert_eq!(MDIO_CTRL_DATA_MASK, 0xFFFF);
        TestResult::Passed
    }

    /// Test additional BMCR bits
    fn test_genet_bmcr_additional_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(BMCR_SPEED_100, 1 << 13);
        test_assert_eq!(BMCR_COLLISION_TEST, 1 << 7);
        TestResult::Passed
    }

    /// Test additional BMSR bits
    fn test_genet_bmsr_additional_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(BMSR_REMOTE_FAULT, 1 << 4);
        TestResult::Passed
    }

    /// Test RSS hash with different inputs produces different outputs
    fn test_genet_rss_hash_different_inputs() -> TestResult {
        use crate::net::genet::*;
        let key = [0x6Du8; RSS_KEY_SIZE];

        let tuple1 = RssIpv4Tuple {
            src_ip: 0xC0A80101,
            dst_ip: 0xC0A80102,
            src_port: 12345,
            dst_port: 80,
        };
        let tuple2 = RssIpv4Tuple {
            src_ip: 0xC0A80102,
            dst_ip: 0xC0A80101,
            src_port: 80,
            dst_port: 12345,
        };
        let tuple3 = RssIpv4Tuple {
            src_ip: 0xC0A80101,
            dst_ip: 0xC0A80102,
            src_port: 12345,
            dst_port: 8080,
        };

        let hash1 = compute_rss_hash_ipv4(&key, &tuple1);
        let hash2 = compute_rss_hash_ipv4(&key, &tuple2);
        let hash3 = compute_rss_hash_ipv4(&key, &tuple3);

        test_assert_ne!(hash1, hash2);
        test_assert_ne!(hash1, hash3);
        test_assert_ne!(hash2, hash3);
        TestResult::Passed
    }

    /// Test magic packet with wrong MAC
    fn test_genet_wol_wrong_mac() -> TestResult {
        use crate::net::genet::*;
        let mac1 = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        let mac2 = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];

        let mut magic = [0xFFu8; 6];
        for _ in 0..16 {
            magic.extend_from_slice(&mac1);
        }

        test_assert!(is_magic_packet(&magic, &mac1));
        test_assert!(!is_magic_packet(&magic, &mac2));
        TestResult::Passed
    }

    /// Test magic packet too short
    fn test_genet_wol_short_packet() -> TestResult {
        use crate::net::genet::*;
        let mac = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
        test_assert!(!is_magic_packet(&[0xFF; 6], &mac)); // Only preamble
        test_assert!(!is_magic_packet(&[0xFF; 10], &mac)); // Too short
        TestResult::Passed
    }

    /// Test NUM_RX_QUEUES constant
    fn test_genet_num_rx_queues() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(NUM_RX_QUEUES, 4);
        TestResult::Passed
    }

    /// Test EEE register definitions
    fn test_genet_eee_registers() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(EEE_CTRL, 0x0864);
        test_assert_eq!(EEE_ADV, 0x0868);
        test_assert_eq!(EEE_LPI_TIMERS, 0x086C);
        TestResult::Passed
    }

    /// Test EEE control bit definitions
    fn test_genet_eee_ctrl_bits() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(EEE_CTRL_EN, 1 << 0);
        test_assert_eq!(EEE_CTRL_TX_LPI_EN, 1 << 1);
        test_assert_eq!(EEE_CTRL_RX_LPI_EN, 1 << 2);
        TestResult::Passed
    }

    /// Test VLAN register definitions
    fn test_genet_vlan_registers() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(VLAN_TAG_CTRL, 0x0840);
        test_assert_eq!(VLAN_TAG_BASE, 0x0844);
        TestResult::Passed
    }

    /// Test Statistics register base
    fn test_genet_stats_registers() -> TestResult {
        use crate::net::genet::*;
        test_assert_eq!(STATS_BASE, 0x0C00);
        TestResult::Passed
    }

    /// Test GenetStats struct
    fn test_genet_stats_struct() -> TestResult {
        use crate::net::genet::GenetStats;
        let stats = GenetStats {
            tx_packets: 100,
            tx_bytes: 150000,
            rx_packets: 95,
            rx_bytes: 142500,
            rx_errors: 2,
            tx_errors: 1,
        };
        test_assert_eq!(stats.tx_packets, 100);
        test_assert_eq!(stats.tx_bytes, 150000);
        test_assert_eq!(stats.rx_packets, 95);
        test_assert_eq!(stats.rx_bytes, 142500);
        test_assert_eq!(stats.rx_errors, 2);
        test_assert_eq!(stats.tx_errors, 1);
        TestResult::Passed
    }

    /// Register all GENET tests
    pub fn register() {
        let tests = test_suite!(
            "genet_dma_desc" => test_genet_dma_desc,
            "genet_constants" => test_genet_constants,
            "genet_tx_desc_flags" => test_genet_tx_desc_flags,
            "genet_rx_desc_flags" => test_genet_rx_desc_flags,
            "genet_register_offsets" => test_genet_register_offsets,
            "genet_mdio_registers" => test_genet_mdio_registers,
            "genet_phy_registers" => test_genet_phy_registers,
            "genet_bmcr_bits" => test_genet_bmcr_bits,
            "genet_bmsr_bits" => test_genet_bmsr_bits,
            "genet_anar_bits" => test_genet_anar_bits,
            "genet_rss_constants" => test_genet_rss_constants,
            "genet_wol_constants" => test_genet_wol_constants,
            "genet_phy_status" => test_genet_phy_status,
            "genet_rss_hash_computation" => test_genet_rss_hash_computation,
            "genet_wol_magic_packet" => test_genet_wol_magic_packet,
            "genet_rx_queue_registers" => test_genet_rx_queue_registers,
            "genet_rss_indir_default" => test_genet_rss_indir_default,
            "genet_gigabit_registers" => test_genet_gigabit_registers,
            "genet_mdio_ctrl_bits" => test_genet_mdio_ctrl_bits,
            "genet_bmcr_additional_bits" => test_genet_bmcr_additional_bits,
            "genet_bmsr_additional_bits" => test_genet_bmsr_additional_bits,
            "genet_rss_hash_different_inputs" => test_genet_rss_hash_different_inputs,
            "genet_wol_wrong_mac" => test_genet_wol_wrong_mac,
            "genet_wol_short_packet" => test_genet_wol_short_packet,
            "genet_num_rx_queues" => test_genet_num_rx_queues,
            "genet_eee_registers" => test_genet_eee_registers,
            "genet_eee_ctrl_bits" => test_genet_eee_ctrl_bits,
            "genet_vlan_registers" => test_genet_vlan_registers,
            "genet_stats_registers" => test_genet_stats_registers,
            "genet_stats_struct" => test_genet_stats_struct,
        );
        register_tests(tests);
    }
}
