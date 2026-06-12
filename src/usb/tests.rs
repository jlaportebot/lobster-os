//! USB subsystem QEMU unit tests
//!
//! Tests for USB hub and mass storage drivers that run
//! inside the kernel under QEMU.

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod usb_tests {
    use crate::qemu_test::*;
    use crate::test_assert;
    use crate::test_assert_eq;
    use crate::test_suite;
    use crate::println;
    use crate::usb::hub::*;
    use crate::usb::mass::*;

    // ─── Hub Descriptor Tests ───────────────────────────────────────────

    fn test_hub_descriptor_parse() -> TestResult {
        let bytes = [0x09, 0x29, 0x04, 0x00, 0x01, 0x32, 0x64, 0x00, 0x00];
        let desc = HubDescriptor::from_bytes(&bytes).expect("parse failed");

        test_assert_eq!(desc.b_length, 9);
        test_assert_eq!(desc.b_descriptor_type, 0x29);
        test_assert_eq!(desc.b_nbr_ports, 4);
        test_assert_eq!(desc.w_hub_characteristics, 0x0001);
        test_assert_eq!(desc.b_pwr_on_2_pwr_good, 50);
        test_assert_eq!(desc.b_hub_contr_current, 100);
        test_assert!(!desc.per_port_power_switching());
        test_assert!(!desc.compound_device());

        TestResult::Passed
    }

    fn test_hub_descriptor_rejects_short() -> TestResult {
        test_assert!(HubDescriptor::from_bytes(&[]).is_none());
        test_assert!(HubDescriptor::from_bytes(&[0x09]).is_none());
        test_assert!(HubDescriptor::from_bytes(&[0x09, 0x29, 0x04]).is_none());
        TestResult::Passed
    }

    fn test_hub_descriptor_rejects_wrong_type() -> TestResult {
        let bytes = [0x07, 0x22, 0x04, 0x00, 0x01, 0x32, 0x64];
        test_assert!(HubDescriptor::from_bytes(&bytes).is_none());
        TestResult::Passed
    }

    // ─── Hub Port Status Tests ─────────────────────────────────────────

    fn test_hub_port_status_parse() -> TestResult {
        let bytes = [0x01, 0x05, 0x00, 0x00]; // CCS=1, PES=1, high-speed
        let status = HubPortStatus::from_bytes(&bytes).expect("parse failed");

        test_assert!(status.current_connect_status());
        test_assert!(status.port_enabled());
        test_assert!(status.high_speed());
        test_assert!(!status.low_speed());
        test_assert_eq!(status.port_speed(), HubPortSpeed::High);
        TestResult::Passed
    }

    fn test_hub_port_status_rejects_short() -> TestResult {
        test_assert!(HubPortStatus::from_bytes(&[0x01, 0x05]).is_none());
        test_assert!(HubPortStatus::from_bytes(&[]).is_none());
        TestResult::Passed
    }

    fn test_hub_port_speed_values() -> TestResult {
        test_assert_eq!(HubPortSpeed::from_bits(0b00), HubPortSpeed::Low);
        test_assert_eq!(HubPortSpeed::from_bits(0b01), HubPortSpeed::Full);
        test_assert_eq!(HubPortSpeed::from_bits(0b10), HubPortSpeed::High);
        test_assert_eq!(HubPortSpeed::from_bits(0b11), HubPortSpeed::Super);
        TestResult::Passed
    }

    fn test_hub_feature_values() -> TestResult {
        test_assert_eq!(HubFeature::CHubLocalPower as u16, 0);
        test_assert_eq!(HubFeature::CHubOverCurrent as u16, 1);
        test_assert_eq!(HubFeature::PortConnection as u16, 0x10);
        test_assert_eq!(HubFeature::PortPower as u16, 0x18);
        test_assert_eq!(HubFeature::PortConnectionChange as u16, 0x20);
        test_assert_eq!(HubFeature::PortResetChange as u16, 0x24);
        TestResult::Passed
    }

    // ─── Mass Storage Structure Tests ──────────────────────────────────

    fn test_mass_scsi_inquiry_parse() -> TestResult {
        // Minimal valid inquiry response (36 bytes)
        let bytes = [
            0x00, // PDT = 0 (direct access)
            0x80, // Removable = 1
            0x06, // Version = SPC-3
            0x02, // Response data format = 2
            0x20, // Additional length = 32
            0x00, 0x00, 0x00, // Reserved
            b'L', b'O', b'B', b'S', b'T', b'E', b'R', b' ', // Vendor: "LOBSTER "
            b'U', b'S', b'B', b' ', b'D', b'R', b'I', b'V',
            b'E', b' ', b'1', b'0', b'2', b'4', b' ', b' ', // Product: "USB DRIVE 1024  "
            b'1', b'.', b'0', b'0', // Revision: "1.00"
        ];

        let inq = ScsiInquiry::from_bytes(&bytes).expect("parse failed");
        test_assert!(inq.is_block_device());
        test_assert!(inq.removable_media);
        test_assert_eq!(inq.version, 0x06);
        test_assert_eq!(inq.additional_length, 0x20);

        TestResult::Passed
    }

    fn test_mass_scsi_inquiry_rejects_short() -> TestResult {
        let short = [0u8; 35];
        test_assert!(ScsiInquiry::from_bytes(&short).is_none());
        TestResult::Passed
    }

    fn test_mass_read_capacity_parse() -> TestResult {
        // 1GB device: last_lba=2097151 (0x1FFFFF), block_size=512 (0x200)
        let bytes: [u8; 8] = [0x00, 0x1F, 0xFF, 0xFF, 0x00, 0x00, 0x02, 0x00];

        let cap = ScsiReadCapacity::from_bytes(&bytes).expect("parse failed");
        test_assert_eq!(cap.last_lba, 0x001FFFFF);
        test_assert_eq!(cap.block_size, 512);
        test_assert_eq!(cap.block_count(), 0x00200000);
        test_assert_eq!(cap.capacity_bytes(), 0x00200000 * 512);
        test_assert_eq!(cap.capacity_mib(), 1024);

        TestResult::Passed
    }

    fn test_mass_read_capacity_rejects_short() -> TestResult {
        test_assert!(ScsiReadCapacity::from_bytes(&[0; 7]).is_none());
        test_assert!(ScsiReadCapacity::from_bytes(&[]).is_none());
        TestResult::Passed
    }

    fn test_mass_scsi_sense_parse() -> TestResult {
        let bytes = [
            0x70, // Response code = 0x70 (current errors)
            0x00, // Reserved
            0x02, // Sense key = NOT READY
            0x00, 0x00, 0x00, 0x00, // Information
            0x0A, // Additional sense length = 10
            0x00, 0x00, 0x00, 0x00, // Command-specific info
            0x3A, // ASC = 0x3A (MEDIUM NOT PRESENT)
            0x00, // ASCQ = 0x00
            0x00, 0x00, 0x00, 0x00, // FRU code + sense-specific
        ];

        let sense = ScsiSense::from_bytes(&bytes).expect("parse failed");
        test_assert_eq!(sense.sense_key, 0x02);
        test_assert_eq!(sense.additional_sense_code, 0x3A);
        test_assert!(!sense.is_no_sense());
        test_assert!(!sense.is_unit_attention());

        TestResult::Passed
    }

    fn test_mass_scsi_sense_no_sense() -> TestResult {
        let bytes = [
            0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0A,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let sense = ScsiSense::from_bytes(&bytes).expect("parse failed");
        test_assert!(sense.is_no_sense());
        TestResult::Passed
    }

    fn test_mass_scsi_sense_unit_attention() -> TestResult {
        let bytes = [
            0x70, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x0A,
            0x00, 0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let sense = ScsiSense::from_bytes(&bytes).expect("parse failed");
        test_assert!(sense.is_unit_attention());
        test_assert_eq!(sense.sense_key, 0x06);
        TestResult::Passed
    }

    fn test_mass_csw_parse() -> TestResult {
        // Valid CSW: signature=0x53425355, tag=1, residue=0, status=0
        let bytes: [u8; 13] = [
            0x55, 0x53, 0x42, 0x53, // Signature ("USBS" in LE)
            0x01, 0x00, 0x00, 0x00, // Tag = 1
            0x00, 0x00, 0x00, 0x00, // Data residue = 0
            0x00, // Status = OK
        ];

        let csw = CommandStatusWrapper::from_bytes(&bytes).expect("parse failed");
        test_assert!(csw.valid_signature());
        test_assert!(csw.is_ok());
        test_assert!(!csw.is_failed());
        test_assert!(!csw.is_phase_error());
        test_assert_eq!(csw.tag, 1);
        test_assert_eq!(csw.data_residue, 0);

        TestResult::Passed
    }

    fn test_mass_csw_failed() -> TestResult {
        let bytes: [u8; 13] = [
            0x55, 0x53, 0x42, 0x53, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, // Status = FAILED
        ];
        let csw = CommandStatusWrapper::from_bytes(&bytes).expect("parse failed");
        test_assert!(csw.is_failed());
        test_assert!(!csw.is_ok());
        TestResult::Passed
    }

    fn test_mass_csw_phase_error() -> TestResult {
        let bytes: [u8; 13] = [
            0x55, 0x53, 0x42, 0x53, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x02, // Status = PHASE ERROR
        ];
        let csw = CommandStatusWrapper::from_bytes(&bytes).expect("parse failed");
        test_assert!(csw.is_phase_error());
        TestResult::Passed
    }

    fn test_mass_csw_invalid_signature() -> TestResult {
        let bytes: [u8; 13] = [
            0xAA, 0xBB, 0xCC, 0xDD, // Bad signature
            0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x00,
        ];
        let csw = CommandStatusWrapper::from_bytes(&bytes).expect("parse failed");
        test_assert!(!csw.valid_signature());
        TestResult::Passed
    }

    fn test_mass_csw_rejects_short() -> TestResult {
        test_assert!(CommandStatusWrapper::from_bytes(&[0; 12]).is_none());
        test_assert!(CommandStatusWrapper::from_bytes(&[]).is_none());
        TestResult::Passed
    }

    fn test_mass_cbw_construction() -> TestResult {
        let cbw = CommandBlockWrapper::inquiry(1, 0);
        test_assert_eq!(cbw.signature, 0x43425355); // CBW_SIGNATURE
        test_assert_eq!(cbw.tag, 1);
        test_assert_eq!(cbw.transfer_length, 36);
        test_assert_eq!(cbw.flags, 0x80); // DIR_IN
        test_assert_eq!(cbw.lun, 0);
        test_assert_eq!(cbw.cb_length, 6);
        test_assert_eq!(cbw.cb[0], ScsiOpcode::Inquiry as u8);

        TestResult::Passed
    }

    fn test_mass_cbw_read10() -> TestResult {
        let cbw = CommandBlockWrapper::read10(2, 0, 0x00001000, 8);
        test_assert_eq!(cbw.transfer_length, BLOCK_SIZE * 8);
        test_assert_eq!(cbw.cb[0], ScsiOpcode::Read10 as u8);
        // LBA = 0x00001000
        test_assert_eq!(cbw.cb[2], 0x00);
        test_assert_eq!(cbw.cb[3], 0x00);
        test_assert_eq!(cbw.cb[4], 0x10);
        test_assert_eq!(cbw.cb[5], 0x00);
        // Block count = 8
        test_assert_eq!(cbw.cb[7], 0x00);
        test_assert_eq!(cbw.cb[8], 0x08);

        TestResult::Passed
    }

    fn test_mass_cbw_write10() -> TestResult {
        let cbw = CommandBlockWrapper::write10(3, 1, 0, 1);
        test_assert_eq!(cbw.flags, 0x00); // DIR_OUT
        test_assert_eq!(cbw.lun, 1);
        test_assert_eq!(cbw.cb[0], ScsiOpcode::Write10 as u8);
        test_assert_eq!(cbw.transfer_length, BLOCK_SIZE);

        TestResult::Passed
    }

    /// Register all USB unit tests
    pub fn register() {
        let tests = test_suite!(
            // Hub descriptor
            "usb_hub_descriptor_parse" => test_hub_descriptor_parse,
            "usb_hub_descriptor_rejects_short" => test_hub_descriptor_rejects_short,
            "usb_hub_descriptor_rejects_wrong_type" => test_hub_descriptor_rejects_wrong_type,
            // Hub port status
            "usb_hub_port_status_parse" => test_hub_port_status_parse,
            "usb_hub_port_status_rejects_short" => test_hub_port_status_rejects_short,
            "usb_hub_port_speed_values" => test_hub_port_speed_values,
            "usb_hub_feature_values" => test_hub_feature_values,
            // Mass storage parsing
            "usb_mass_scsi_inquiry_parse" => test_mass_scsi_inquiry_parse,
            "usb_mass_scsi_inquiry_rejects_short" => test_mass_scsi_inquiry_rejects_short,
            "usb_mass_read_capacity_parse" => test_mass_read_capacity_parse,
            "usb_mass_read_capacity_rejects_short" => test_mass_read_capacity_rejects_short,
            "usb_mass_scsi_sense_parse" => test_mass_scsi_sense_parse,
            "usb_mass_scsi_sense_no_sense" => test_mass_scsi_sense_no_sense,
            "usb_mass_scsi_sense_unit_attention" => test_mass_scsi_sense_unit_attention,
            "usb_mass_csw_parse" => test_mass_csw_parse,
            "usb_mass_csw_failed" => test_mass_csw_failed,
            "usb_mass_csw_phase_error" => test_mass_csw_phase_error,
            "usb_mass_csw_invalid_signature" => test_mass_csw_invalid_signature,
            "usb_mass_csw_rejects_short" => test_mass_csw_rejects_short,
            "usb_mass_cbw_construction" => test_mass_cbw_construction,
            "usb_mass_cbw_read10" => test_mass_cbw_read10,
            "usb_mass_cbw_write10" => test_mass_cbw_write10
        );
        register_tests(tests);
    }
}
