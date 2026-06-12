//! Display Subsystem Integration Tests
//!
//! Tests for framebuffer, graphics primitives, and console.

#[cfg(all(feature = "qemu", target_arch = "aarch64"))]
pub mod display_tests {
    use crate::qemu_test::*;
    use crate::display::{fb, graphics};

    /// Test framebuffer info struct
    fn test_fb_info_struct() -> TestResult {
        let info = fb::FramebufferInfo::empty();
        test_assert_eq!(info.base_addr, 0);
        test_assert_eq!(info.size, 0);
        test_assert_eq!(info.width, 0);
        test_assert_eq!(info.height, 0);
        test_assert_eq!(info.pitch, 0);
        test_assert_eq!(info.bpp, 0);

        let info = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };
        test_assert_eq!(info.total_pixels(), 1920 * 1080);
        test_assert_eq!(info.buffer_size(), 8294400);
        TestResult::Passed
    }

    /// Test framebuffer error enum
    fn test_fb_error_enum() -> TestResult {
        use fb::FbError;
        test_assert_eq!(FbError::MailboxTimeout as u8, 0);
        test_assert_eq!(FbError::InvalidResponse as u8, 1);
        test_assert_eq!(FbError::BufferAllocationFailed as u8, 2);
        test_assert_eq!(FbError::UnsupportedDepth as u8, 3);
        test_assert_eq!(FbError::ConfigurationFailed as u8, 4);
        TestResult::Passed
    }

    /// Test graphics draw_pixel bounds checking
    fn test_graphics_draw_pixel_bounds() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        // Test in-bounds (should not panic)
        graphics::draw_pixel(&fb, 100, 100, 0xFFFFFFFF);
        graphics::draw_pixel(&fb, 0, 0, 0xFFFFFFFF);
        graphics::draw_pixel(&fb, 1919, 1079, 0xFFFFFFFF);

        // Test out-of-bounds (should return early)
        graphics::draw_pixel(&fb, 1920, 100, 0xFFFFFFFF);
        graphics::draw_pixel(&fb, 100, 1080, 0xFFFFFFFF);
        graphics::draw_pixel(&fb, 2000, 2000, 0xFFFFFFFF);

        TestResult::Passed
    }

    /// Test graphics draw_rect
    fn test_graphics_draw_rect() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        // Test normal rect
        graphics::draw_rect(&fb, 100, 100, 200, 150, 0xFF0000FF);

        // Test partial out-of-bounds
        graphics::draw_rect(&fb, 1900, 100, 200, 150, 0xFF0000FF);

        // Test zero size
        graphics::draw_rect(&fb, 100, 100, 0, 0, 0xFF0000FF);

        TestResult::Passed
    }

    /// Test graphics draw_line (Bresenham)
    fn test_graphics_draw_line() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        // Horizontal line
        graphics::draw_line(&fb, 100, 100, 200, 100, 0xFF0000FF);
        // Vertical line
        graphics::draw_line(&fb, 100, 100, 100, 200, 0xFF0000FF);
        // Diagonal line
        graphics::draw_line(&fb, 100, 100, 200, 200, 0xFF0000FF);
        // Reverse diagonal
        graphics::draw_line(&fb, 200, 100, 100, 200, 0xFF0000FF);
        // Single point
        graphics::draw_line(&fb, 100, 100, 100, 100, 0xFF0000FF);
        // Out of bounds
        graphics::draw_line(&fb, -100, 100, 200, 200, 0xFF0000FF); // Will be treated as large unsigned

        TestResult::Passed
    }

    /// Test graphics draw_circle (midpoint algorithm)
    fn test_graphics_draw_circle() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        graphics::draw_circle(&fb, 500, 500, 100, 0xFF0000FF);
        graphics::draw_circle(&fb, 100, 100, 50, 0xFF0000FF);
        graphics::draw_circle(&fb, 0, 0, 10, 0xFF0000FF); // Partial out of bounds

        TestResult::Passed
    }

    /// Test graphics fill_region
    fn test_graphics_fill_region() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        graphics::fill_region(&fb, 100, 100, 200, 150, 0xFF0000FF);
        graphics::fill_region(&fb, 1900, 100, 200, 150, 0xFF0000FF); // Partial OOB
        graphics::fill_region(&fb, 100, 100, 0, 0, 0xFF0000FF);

        TestResult::Passed
    }

    /// Test graphics draw_bitmap
    fn test_graphics_draw_bitmap() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        // 8x8 bitmap (8 bytes, 1 byte per row)
        let bitmap: [u8; 8] = [
            0b11111111,
            0b10000001,
            0b10000001,
            0b10000001,
            0b10000001,
            0b10000001,
            0b10000001,
            0b11111111,
        ];

        graphics::draw_bitmap(&fb, 100, 100, &bitmap, 8, 8, 0xFFFFFFFF, 0xFF000000);
        graphics::draw_bitmap(&fb, 1915, 100, &bitmap, 8, 8, 0xFFFFFFFF, 0xFF000000); // Partial OOB

        TestResult::Passed
    }

    /// Test graphics blit
    fn test_graphics_blit() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        let src = [0xFFFFFFFFu32; 64]; // 8x8 white square
        graphics::blit(&fb, 100, 100, &src, 8, 8);
        graphics::blit(&fb, 1915, 1075, &src, 8, 8); // Partial OOB

        TestResult::Passed
    }

    /// Test console font access
    fn test_console_font() -> TestResult {
        use crate::display::console::{FONT, CHAR_WIDTH, CHAR_HEIGHT};

        test_assert_eq!(CHAR_WIDTH, 8);
        test_assert_eq!(CHAR_HEIGHT, 16);
        test_assert_eq!(FONT.len(), 128);
        
        // Check space character (0x20) is empty
        for row in 0..16 {
            test_assert_eq!(FONT[0x20][row], 0);
        }

        // Check 'A' character (0x41) has some pixels
        let mut has_pixels = false;
        for row in 0..16 {
            if FONT[0x41][row] != 0 {
                has_pixels = true;
                break;
            }
        }
        test_assert!(has_pixels);

        TestResult::Passed
    }

    /// Test graphics draw_text
    fn test_graphics_draw_text() -> TestResult {
        let fb = fb::FramebufferInfo {
            base_addr: 0x3F000000,
            size: 8294400,
            width: 1920,
            height: 1080,
            pitch: 7680,
            bpp: 32,
        };

        graphics::draw_text(&fb, 100, 100, "Hello", 0xFFFFFFFF, 0xFF000000);
        graphics::draw_text(&fb, 100, 100, "", 0xFFFFFFFF, 0xFF000000); // Empty string
        graphics::draw_text(&fb, 1910, 100, "World", 0xFFFFFFFF, 0xFF000000); // Partial OOB

        TestResult::Passed
    }

    /// Register all display tests
    pub fn register() {
        let tests = test_suite!(
            "fb_info_struct" => test_fb_info_struct,
            "fb_error_enum" => test_fb_error_enum,
            "graphics_draw_pixel_bounds" => test_graphics_draw_pixel_bounds,
            "graphics_draw_rect" => test_graphics_draw_rect,
            "graphics_draw_line" => test_graphics_draw_line,
            "graphics_draw_circle" => test_graphics_draw_circle,
            "graphics_fill_region" => test_graphics_fill_region,
            "graphics_draw_bitmap" => test_graphics_draw_bitmap,
            "graphics_blit" => test_graphics_blit,
            "console_font" => test_console_font,
            "graphics_draw_text" => test_graphics_draw_text,
        );
        register_tests(tests);
    }
}
