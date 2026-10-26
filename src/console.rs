//! Console abstraction — print! / println! macros using UART (AArch64)

#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
use crate::drivers::uart::Uart10;
use core::fmt;

/// Console writer that implements fmt::Write
struct ConsoleWriter;

impl fmt::Write for ConsoleWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
        Uart10::puts(s);
        #[cfg(all(target_arch = "x86_64", not(feature = "std")))]
        crate::arch::x86_64::console::serial_write_str(s);
        #[cfg(feature = "std")]
        let _ = s; // host test builds: swallow console output
        Ok(())
    }
}

/// Initialize the console (UART must be initialized first)
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::console::init();
}

/// Core print implementation
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::disable_irqs();
    let _ = ConsoleWriter.write_fmt(args);
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::enable_irqs();
}

/// Print to serial console (no newline)
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::_print(format_args!($($arg)*))
    };
}

/// Print to serial console (with newline)
#[macro_export]
macro_rules! println {
    () => {{
        $crate::console::_print(format_args!("\n"));
    }};
    ($($arg:tt)*) => {{
        $crate::console::_print(format_args!($($arg)*));
        $crate::console::_print(format_args!("\n"));
    }};
}
