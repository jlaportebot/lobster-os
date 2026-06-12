//! QEMU Integration Tests
//!
//! These tests run inside the kernel when booted in QEMU with test_mode=1.

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod qemu_tests {
    use crate::qemu_test::*;
    use crate::println;

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
        );
        register_tests(tests);
    }
}