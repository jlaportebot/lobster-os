//! USB Hub Driver Integration Tests
//!
//! Tests for USB hub support in the xHCI driver:
//! - Hub descriptor parsing
//! - Port status and change detection
//! - Hub port power/reset control
//! - Compound device handling

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod usb_hub_tests {
    use crate::qemu_test::*;
    use crate::usb::hub::*;

    /// Test Hub descriptor structure
    fn test_usb_hub_descriptor() -> TestResult {
        // Minimal hub descriptor (from USB 2.0 spec Table 11-13)
        let desc_bytes = [
            0x09, // bLength (9 bytes for USB 2.0)
            0x29, // bDescriptorType = HUB (0x29)
            0x04, // bNbrPorts = 4
            0x00, 0x01, // wHubCharacteristics = 0x0001 (Ganged power switching)
            0x32, // bPwrOn2PwrGood = 50 * 2ms = 100ms
            0x64, // bHubContrCurrent = 100mA
            0x00, // DeviceRemovable[0] = all ports removable
            0x00, // PortPwrCtrlMask[0] = all ports controlled
        ];

        let desc = HubDescriptor::from_bytes(&desc_bytes).expect("Failed to parse hub descriptor");

        test_assert_eq!(desc.b_length, 9);
        test_assert_eq!(desc.b_descriptor_type, 0x29);
        test_assert_eq!(desc.b_nbr_ports, 4);
        test_assert_eq!(desc.w_hub_characteristics, 0x0001);
        test_assert_eq!(desc.b_pwr_on_2_pwr_good, 50);
        test_assert_eq!(desc.b_hub_contr_current, 100);

        TestResult::Passed
    }

    /// Test Hub descriptor with per-port power switching
    fn test_usb_hub_descriptor_per_port() -> TestResult {
        // Hub descriptor with per-port power switching
        let desc_bytes = [
            0x09, // bLength
            0x29, // bDescriptorType = HUB
            0x07, // bNbrPorts = 7
            0x00, 0x00, // wHubCharacteristics = 0x0000 (Per-port power switching)
            0x14, // bPwrOn2PwrGood = 20 * 2ms = 40ms
            0x32, // bHubContrCurrent = 50mA
            0x00,
            0x00,
        ];

        let desc = HubDescriptor::from_bytes(&desc_bytes).expect("Failed to parse hub descriptor");

        test_assert_eq!(desc.b_nbr_ports, 7);
        test_assert_eq!(desc.w_hub_characteristics & 0x0003, 0x0000); // Per-port power switching
        test_assert!(desc.per_port_power_switching());
        test_assert_eq!(desc.b_pwr_on_2_pwr_good, 20);
        test_assert_eq!(desc.b_hub_contr_current, 50);

        TestResult::Passed
    }

    /// Test Hub descriptor — too short returns None
    fn test_usb_hub_descriptor_too_short() -> TestResult {
        let short_bytes = [0x05, 0x29, 0x04]; // Only 3 bytes
        test_assert!(HubDescriptor::from_bytes(&short_bytes).is_none());

        let wrong_type = [0x07, 0x22, 0x04, 0x00, 0x01, 0x32, 0x64]; // Wrong descriptor type
        test_assert!(HubDescriptor::from_bytes(&wrong_type).is_none());

        TestResult::Passed
    }

    /// Test Hub descriptor compound device and over-current flags
    fn test_usb_hub_descriptor_flags() -> TestResult {
        // Compound device, per-port over-current, TT think time = 2
        let desc_bytes = [
            0x09,
            0x29,
            0x04,
            0x34, 0x00, // wHubCharacteristics: compound=1, per-port OC=1, TT think=2
            0x32,
            0x64,
            0x00,
            0x00,
        ];

        let desc = HubDescriptor::from_bytes(&desc_bytes).expect("Failed to parse hub descriptor");
        test_assert!(desc.compound_device());
        test_assert!(desc.per_port_overcurrent());
        test_assert_eq!(desc.tt_think_time(), 2);

        TestResult::Passed
    }

    /// Test Hub port status structure — basic fields
    fn test_usb_hub_port_status() -> TestResult {
        // Port status: connected, enabled, high-speed
        let status_bytes = [
            0x01, 0x05, // wPortStatus: CCS=1, PES=1, PSS=high-speed (bit 10)
            0x00, 0x00, // wPortChange: no changes
        ];

        let status = HubPortStatus::from_bytes(&status_bytes).expect("Failed to parse port status");

        test_assert!(status.current_connect_status());
        test_assert!(status.port_enabled());
        test_assert!(status.high_speed());
        test_assert_eq!(status.port_speed(), HubPortSpeed::High);

        TestResult::Passed
    }

    /// Test Hub port status — too short returns None
    fn test_usb_hub_port_status_too_short() -> TestResult {
        let short_bytes = [0x01, 0x05]; // Only 2 bytes
        test_assert!(HubPortStatus::from_bytes(&short_bytes).is_none());
        TestResult::Passed
    }

    /// Test Hub port change detection
    fn test_usb_hub_port_change() -> TestResult {
        // Port status with connect status change
        let status_bytes = [
            0x01, 0x01, // wPortStatus
            0x01, 0x00, // wPortChange: C_CSC = 1
        ];

        let status = HubPortStatus::from_bytes(&status_bytes).expect("Failed to parse port status");

        test_assert!(status.connect_status_change());
        test_assert!(!status.port_enable_change());
        test_assert!(!status.over_current_change());

        TestResult::Passed
    }

    /// Test Hub port speed enumeration
    fn test_usb_hub_port_speed_enum() -> TestResult {
        test_assert_eq!(HubPortSpeed::from_bits(0b00), HubPortSpeed::Low);
        test_assert_eq!(HubPortSpeed::from_bits(0b01), HubPortSpeed::Full);
        test_assert_eq!(HubPortSpeed::from_bits(0b10), HubPortSpeed::High);
        test_assert_eq!(HubPortSpeed::from_bits(0b11), HubPortSpeed::Super);

        TestResult::Passed
    }

    /// Test Hub port speed derived from status bits
    fn test_usb_hub_port_speed_from_status() -> TestResult {
        // Low speed
        let lo_bytes = [0x01, 0x03, 0x00, 0x00]; // CCS=1, PES=1, low-speed bit
        let lo = HubPortStatus::from_bytes(&lo_bytes).unwrap();
        test_assert_eq!(lo.port_speed(), HubPortSpeed::Low);

        // Full speed (enabled but no low/high speed bits)
        let full_bytes = [0x03, 0x00, 0x00, 0x00]; // CCS=1, PES=1
        let full = HubPortStatus::from_bytes(&full_bytes).unwrap();
        test_assert_eq!(full.port_speed(), HubPortSpeed::Full);

        TestResult::Passed
    }

    /// Test Hub class request codes
    fn test_usb_hub_class_requests() -> TestResult {
        test_assert_eq!(HubRequest::GetStatus as u8, 0x00);
        test_assert_eq!(HubRequest::ClearFeature as u8, 0x01);
        test_assert_eq!(HubRequest::GetState as u8, 0x02);
        test_assert_eq!(HubRequest::SetFeature as u8, 0x03);
        test_assert_eq!(HubRequest::GetDescriptor as u8, 0x06);
        test_assert_eq!(HubRequest::SetDescriptor as u8, 0x07);
        test_assert_eq!(HubRequest::ClearTTBuffer as u8, 0x08);
        test_assert_eq!(HubRequest::ResetTT as u8, 0x09);
        test_assert_eq!(HubRequest::GetTTState as u8, 0x0A);
        test_assert_eq!(HubRequest::StopTT as u8, 0x0B);

        TestResult::Passed
    }

    /// Test Hub feature selectors match USB 2.0 spec values
    fn test_usb_hub_feature_selectors() -> TestResult {
        // Hub-level features
        test_assert_eq!(HubFeature::CHubLocalPower as u16, 0);
        test_assert_eq!(HubFeature::CHubOverCurrent as u16, 1);

        // Port features (start at 0x10 per USB 2.0 spec Table 11-17)
        test_assert_eq!(HubFeature::PortConnection as u16, 0x10);
        test_assert_eq!(HubFeature::PortEnable as u16, 0x11);
        test_assert_eq!(HubFeature::PortSuspend as u16, 0x12);
        test_assert_eq!(HubFeature::PortOverCurrent as u16, 0x13);
        test_assert_eq!(HubFeature::PortReset as u16, 0x14);
        test_assert_eq!(HubFeature::PortPower as u16, 0x18);
        test_assert_eq!(HubFeature::PortLowSpeed as u16, 0x19);

        // Port change features (C_* — start at 0x20)
        test_assert_eq!(HubFeature::PortConnectionChange as u16, 0x20);
        test_assert_eq!(HubFeature::PortEnableChange as u16, 0x21);
        test_assert_eq!(HubFeature::PortSuspendChange as u16, 0x22);
        test_assert_eq!(HubFeature::PortOverCurrentChange as u16, 0x23);
        test_assert_eq!(HubFeature::PortResetChange as u16, 0x24);

        TestResult::Passed
    }

    /// Register all USB Hub tests
    pub fn register() {
        let tests = test_suite!(
            "usb_hub_descriptor" => test_usb_hub_descriptor,
            "usb_hub_descriptor_per_port" => test_usb_hub_descriptor_per_port,
            "usb_hub_descriptor_too_short" => test_usb_hub_descriptor_too_short,
            "usb_hub_descriptor_flags" => test_usb_hub_descriptor_flags,
            "usb_hub_port_status" => test_usb_hub_port_status,
            "usb_hub_port_status_too_short" => test_usb_hub_port_status_too_short,
            "usb_hub_port_change" => test_usb_hub_port_change,
            "usb_hub_port_speed_enum" => test_usb_hub_port_speed_enum,
            "usb_hub_port_speed_from_status" => test_usb_hub_port_speed_from_status,
            "usb_hub_class_requests" => test_usb_hub_class_requests,
            "usb_hub_feature_selectors" => test_usb_hub_feature_selectors,
        );
        register_tests(tests);
    }
}
