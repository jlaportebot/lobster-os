//! QEMU Integration Tests
//!
//! These tests run inside the kernel when booted in QEMU with test_mode=1.

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod qemu_tests {
    use crate::qemu_test::*;
    use crate::println;

    // Include display tests module
    include!("display_tests.rs");

    // Include GENET tests module
    include!("genet_tests.rs");

    /// Test memory allocator basic functionality
    fn test_heap_allocation() -> TestResult {
        use core::alloc::{GlobalAlloc, Layout};
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = crate::memory::heap::LobsterAllocator.alloc(layout);
        test_assert!(!ptr.is_null(), "[test] 256-byte allocation failed");
        // Write test pattern
        for i in 0u8..=255u8 {
            unsafe { core::ptr::write_volatile(ptr.add(i as usize), i); }
        }
        // Verify
        for i in 0u8..=255u8 {
            let val = unsafe { core::ptr::read_volatile(ptr.add(i as usize)) };
            test_assert_eq!(val, i, "[test] Memory verification failed at offset {}", i);
        }
        unsafe { crate::memory::heap::LobsterAllocator.dealloc(ptr, layout); }
        TestResult::Passed
    }

    /// Test frame allocator
    fn test_frame_allocation() -> TestResult {
        if let Some(frame) = crate::memory::frame_alloc::allocate_frame() {
            crate::memory::frame_alloc::deallocate_frame(frame);
            TestResult::Passed
        } else {
            TestResult::Failed
        }
    }

    /// Test spinlock basic functionality
    fn test_spinlock() -> TestResult {
        let lock = crate::sync::Spinlock::new(0);
        {
            let mut guard = lock.lock();
            *guard = 42;
            test_assert_eq!(*guard, 42);
        }
        test_assert_eq!(*lock.lock(), 42);
        TestResult::Passed
    }

    /// Test network stack basic functionality
    fn test_network_stack() -> TestResult {
        // Test IP address parsing
        use core::net::Ipv4Addr;
        let addr: Ipv4Addr = "192.168.1.1".parse().unwrap();
        test_assert_eq!(addr.octets(), [192, 168, 1, 1]);
        TestResult::Passed
    }

    /// Test SHA-1 implementation
    fn test_sha1() -> TestResult {
        use crate::drivers::wifi::sha1::sha1_hash;
        let hash = sha1_hash(b"");
        test_assert_eq!(
            hash,
            [
                0xda, 0x39, 0xa3, 0xee, 0x5e, 0x6b, 0x4b, 0x0d, 0x32, 0x55, 0xbf,
                0xef, 0x95, 0x60, 0x18, 0x90, 0xaf, 0xd8, 0x07, 0x09
            ]
        );
        let hash = sha1_hash(b"abc");
        test_assert_eq!(
            hash,
            [
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25,
                0x71, 0x78, 0x50, 0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d
            ]
        );
        TestResult::Passed
    }

    /// Test HMAC-SHA1 implementation
    fn test_hmac_sha1() -> TestResult {
        use crate::drivers::wifi::sha1::hmac_sha1;
        let key = [0x0b; 20];
        let data = b"Hi There";
        let mac = hmac_sha1(&key, data);
        test_assert_eq!(
            mac,
            [
                0xb6, 0x17, 0x31, 0x86, 0x55, 0x05, 0x72, 0x64, 0xe2, 0x8b, 0xc0,
                0xb6, 0xfb, 0x37, 0x8c, 0x8e, 0xf1, 0x46, 0x4e, 0xdc
            ]
        );
        TestResult::Passed
    }

    /// Test USB xHCI TRB operations
    fn test_usb_xhci_trb() -> TestResult {
        use crate::usb::xhci::Trb;
        let trb = Trb::new(0x1000, 0, 0);
        test_assert_eq!(trb.param, 0x1000);
        test_assert_eq!(trb.status, 0);

        let mut trb = Trb::ZERO;
        trb.set_type(9);
        test_assert_eq!(trb.trb_type(), 9);

        test_assert!(!trb.cycle_bit());
        trb.set_cycle(true);
        test_assert!(trb.cycle_bit());
        trb.toggle_cycle();
        test_assert!(!trb.cycle_bit());
        TestResult::Passed
    }

    /// Test GENET DMA descriptor
    fn test_genet_dma_desc() -> TestResult {
        use crate::net::genet::DmaDesc;
        let desc = DmaDesc::default();
        test_assert_eq!(desc.status, 0);
        test_assert_eq!(desc.addr_lo, 0);
        test_assert_eq!(desc.addr_hi, 0);
        test_assert_eq!(desc.ctrl, 0);
        TestResult::Passed
    }

    /// Test scheduler task creation
    fn test_scheduler_task_creation() -> TestResult {
        // This test requires scheduler to be initialized
        // Just test that types compile and basic functions work
        let _task_state = crate::sched::TaskState::Ready;
        let _priority = crate::sched::Priority::Normal;
        TestResult::Passed
    }

    /// Test window manager types
    fn test_window_manager_types() -> TestResult {
        use crate::wm::window::Window;
        let w = Window::new("test", 0, 0, 100, 100);
        test_assert_eq!(w.title(), "test");
        TestResult::Passed
    }

    /// Test GPIO basic operations
    fn test_gpio() -> TestResult {
        // GPIO tests would require hardware
        TestResult::Skipped
    }

    /// Test timer functionality
    fn test_timer() -> TestResult {
        let before = crate::arch::timer::Timer::read_cntpct();
        crate::arch::cpu::delay_us(1000);
        let after = crate::arch::timer::Timer::read_cntpct();
        test_assert!(after > before, "[test] Timer should advance");
        TestResult::Passed
    }

    /// Test UART echo
    fn test_uart_echo() -> TestResult {
        // UART test would need loopback
        TestResult::Skipped
    }

    /// Test interrupt handling
    fn test_interrupts() -> TestResult {
        // Test that IRQs can be enabled/disabled
        crate::arch::cpu::enable_irqs();
        crate::arch::cpu::disable_irqs();
        TestResult::Passed
    }

    /// Test GPU driver initialization and display info
    fn test_gpu_display_init() -> TestResult {
        // This test runs on real hardware/QEMU where mailbox is available
        // For now, just test that the GPU module types are accessible
        let _info = crate::drivers::gpu::display_info();
        let _gpu_info = crate::drivers::gpu::gpu_info();
        let _initialized = crate::drivers::gpu::is_initialized();
        TestResult::Passed
    }

    /// Test GPU EDID parsing (unit test - runs without hardware)
    fn test_gpu_edid_parsing() -> TestResult {
        use crate::drivers::gpu::{parse_edid, EdidVersion};
        
        // Create a minimal valid EDID block
        let mut edid = [0u8; 128];
        edid[0] = 0x00;
        edid[1..7].fill(0xFF);
        edid[7] = 0x00;
        edid[8] = 0x10; // Manufacturer "DEL"
        edid[9] = 0xAC;
        edid[10] = 0x71; // Product code
        edid[11] = 0x40;
        edid[12..16].copy_from_slice(&[0x4E, 0x61, 0xBC, 0x00]); // Serial
        edid[16] = 25; // Week
        edid[17] = 30; // Year (2020)
        edid[18] = 1; // Version major
        edid[19] = 4; // Version minor
        edid[20] = 0xA0;
        edid[21] = 53;
        edid[22] = 30;
        edid[23] = 78;
        edid[24] = 0x0A;
        // ... fill rest with zeros
        // Calculate checksum
        let sum: u8 = edid[..127].iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        edid[127] = sum.wrapping_neg();
        
        let result = parse_edid(&edid);
        test_assert!(result.is_ok(), "[test] EDID parsing failed");
        
        let info = result.unwrap();
        test_assert_eq!(info.version.major, 1);
        test_assert_eq!(info.version.minor, 4);
        test_assert_eq!(info.manufacturer_id, [b'D', b'E', b'L']);
        TestResult::Passed
    }

    /// Test HDMI hotplug state enum
    fn test_gpu_hotplug_state() -> TestResult {
        use crate::drivers::gpu::HotplugState;
        test_assert_eq!(HotplugState::Disconnected as u8, 0);
        test_assert_eq!(HotplugState::Connected as u8, 1);
        TestResult::Passed
    }

    /// Test HDMI audio infoframe struct
    fn test_gpu_hdmi_audio_infoframe() -> TestResult {
        use crate::drivers::gpu::HdmiAudioInfoFrame;
        let info = HdmiAudioInfoFrame {
            coding_type: 1,
            sample_rate: 3,
            sample_size: 2,
            channels: 2,
            speaker_placement: 0,
            channel_allocation: 0,
            level_shift: 0,
            downmix_inhibit: false,
        };
        test_assert_eq!(info.coding_type, 1);
        test_assert_eq!(info.sample_rate, 3);
        test_assert_eq!(info.channels, 2);
        TestResult::Passed
    }

    /// Test cursor info and state
    fn test_gpu_cursor() -> TestResult {
        use crate::drivers::gpu::{CursorInfo, CursorState};
        let info = CursorInfo {
            width: 64,
            height: 64,
            format: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            data_ptr: 0x1000,
        };
        test_assert_eq!(info.width, 64);
        test_assert_eq!(info.height, 64);
        test_assert_eq!(CursorState::Hidden as u8, 0);
        test_assert_eq!(CursorState::Visible as u8, 1);
        TestResult::Passed
    }

    /// Test display info struct
    fn test_gpu_display_info() -> TestResult {
        use crate::drivers::gpu::DisplayInfo;
        let info = DisplayInfo {
            width: 1920,
            height: 1080,
            bpp: 32,
            pitch: 7680,
            fb_addr: 0x3F000000,
            fb_size: 8294400,
            pixel_order: 1,
            alpha_mode: 0,
        };
        test_assert_eq!(info.width, 1920);
        test_assert_eq!(info.height, 1080);
        test_assert_eq!(info.bpp, 32);
        test_assert_eq!(info.pixel_order, 1);
        TestResult::Passed
    }

    /// Test GPU info struct
    fn test_gpu_gpu_info() -> TestResult {
        use crate::drivers::gpu::GpuInfo;
        let info = GpuInfo {
            firmware_rev: 0x12345678,
            board_model: 0x00000011,
            board_rev: 0x00000010,
            vc_mem_base: 0x3A000000,
            vc_mem_size: 0x06000000,
            arm_mem_base: 0x00000000,
            arm_mem_size: 0x80000000,
            v3d_clock: 800_000_000,
            core_clock: 600_000_000,
        };
        test_assert_eq!(info.firmware_rev, 0x12345678);
        test_assert_eq!(info.v3d_clock, 800_000_000);
        TestResult::Passed
    }

    /// Register all QEMU integration tests
    pub fn register() {
        let tests = test_suite!(
            "heap_allocation" => test_heap_allocation,
            "frame_allocation" => test_frame_allocation,
            "spinlock" => test_spinlock,
            "network_stack" => test_network_stack,
            "sha1" => test_sha1,
            "hmac_sha1" => test_hmac_sha1,
            "usb_xhci_trb" => test_usb_xhci_trb,
            "genet_dma_desc" => test_genet_dma_desc,
            "scheduler_task_creation" => test_scheduler_task_creation,
            "window_manager_types" => test_window_manager_types,
            "gpio" => test_gpio,
            "timer" => test_timer,
            "uart_echo" => test_uart_echo,
            "interrupts" => test_interrupts,
            "gpu_display_init" => test_gpu_display_init,
            "gpu_edid_parsing" => test_gpu_edid_parsing,
            "gpu_hotplug_state" => test_gpu_hotplug_state,
            "gpu_hdmi_audio_infoframe" => test_gpu_hdmi_audio_infoframe,
            "gpu_cursor" => test_gpu_cursor,
            "gpu_display_info" => test_gpu_display_info,
            "gpu_gpu_info" => test_gpu_gpu_info,
            // Display subsystem tests
            "fb_info_struct" => display_tests::test_fb_info_struct,
            "fb_error_enum" => display_tests::test_fb_error_enum,
            "graphics_draw_pixel_bounds" => display_tests::test_graphics_draw_pixel_bounds,
            "graphics_draw_rect" => display_tests::test_graphics_draw_rect,
            "graphics_draw_line" => display_tests::test_graphics_draw_line,
            "graphics_draw_circle" => display_tests::test_graphics_draw_circle,
            "graphics_fill_region" => display_tests::test_graphics_fill_region,
            "graphics_draw_bitmap" => display_tests::test_graphics_draw_bitmap,
            "graphics_blit" => display_tests::test_graphics_blit,
            "console_font" => display_tests::test_console_font,
            "graphics_draw_text" => display_tests::test_graphics_draw_text,
        );
        register_tests(tests);
    }
}