//! Console abstraction — print! / println! macros using UART (AArch64)

#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
use crate::drivers::uart::Uart10;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

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

/// Global mirror gate: when true, UART bytes are also mirrored to the graphical terminal.
/// Default false — kernel log output goes to serial + klog only, never the Terminal window.
static MIRROR_TO_TERMINAL: AtomicBool = AtomicBool::new(false);

/// Returns whether terminal mirroring is currently enabled.
pub fn mirror_to_terminal_enabled() -> bool {
    MIRROR_TO_TERMINAL.load(Ordering::Relaxed)
}

/// Enable terminal mirroring (for shell command output, prompt echo, etc.)
pub fn set_mirror_to_terminal(enabled: bool) {
    MIRROR_TO_TERMINAL.store(enabled, Ordering::Relaxed);
}

/// Run a closure with terminal mirroring enabled, then restore previous state.
pub fn with_terminal_mirror<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let prev = mirror_to_terminal_enabled();
    set_mirror_to_terminal(true);
    let result = f();
    set_mirror_to_terminal(prev);
    result
}

/// Initialize the console (UART must be initialized first)
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::console::init();
}

/// Core print implementation — writes to UART with mirroring OFF (kernel log path).
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::disable_irqs();
    let _ = ConsoleWriter.write_fmt(args);
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::enable_irqs();
}

/// Core print implementation with terminal mirroring ON — writes to UART with mirroring enabled.
/// Use this for shell command output, user-typed echo, and prompts in the graphical Terminal window.
#[doc(hidden)]
pub fn _print_terminal(args: fmt::Arguments) {
    use core::fmt::Write;
    let prev = mirror_to_terminal_enabled();
    set_mirror_to_terminal(true);
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::disable_irqs();
    let _ = ConsoleWriter.write_fmt(args);
    #[cfg(all(target_arch = "aarch64", not(feature = "std")))]
    crate::arch::cpu::enable_irqs();
    set_mirror_to_terminal(prev);
}

/// Print to serial console (no newline) — kernel log path, no terminal mirror.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::console::_print(format_args!($($arg)*))
    };
}

/// Print to serial console (with newline) — kernel log path, no terminal mirror.
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

/// Print to serial console (no newline) — with terminal mirroring enabled.
/// Use for shell command output, keystroke echo, prompts that must appear in the GUI Terminal window.
#[macro_export]
macro_rules! print_terminal {
    ($($arg:tt)*) => {
        $crate::console::_print_terminal(format_args!($($arg)*))
    };
}

/// Print to serial console (with newline) — with terminal mirroring enabled.
/// Use for shell command output, keystroke echo, prompts that must appear in the GUI Terminal window.
#[macro_export]
macro_rules! println_terminal {
    () => {{
        $crate::console::_print_terminal(format_args!("\n"));
    }};
    ($($arg:tt)*) => {{
        $crate::console::_print_terminal(format_args!($($arg)*));
        $crate::console::_print_terminal(format_args!("\n"));
    }};
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_gate_defaults_to_false() {
        assert!(
            !mirror_to_terminal_enabled(),
            "Mirror gate should default to false"
        );
    }

    #[test]
    fn test_mirror_gate_set_get() {
        set_mirror_to_terminal(true);
        assert!(
            mirror_to_terminal_enabled(),
            "Mirror gate should be true after set(true)"
        );
        set_mirror_to_terminal(false);
        assert!(
            !mirror_to_terminal_enabled(),
            "Mirror gate should be false after set(false)"
        );
    }

    #[test]
    fn test_with_terminal_mirror_restores_state() {
        // Start with gate off
        set_mirror_to_terminal(false);
        assert!(!mirror_to_terminal_enabled());

        // Inside the closure, gate should be on
        let result = with_terminal_mirror(|| {
            assert!(
                mirror_to_terminal_enabled(),
                "Gate should be on inside closure"
            );
            42
        });
        assert_eq!(result, 42);

        // After closure, gate should be restored to off
        assert!(
            !mirror_to_terminal_enabled(),
            "Gate should be restored to off after closure"
        );

        // Test with gate initially on
        set_mirror_to_terminal(true);
        assert!(mirror_to_terminal_enabled());

        let result = with_terminal_mirror(|| {
            assert!(
                mirror_to_terminal_enabled(),
                "Gate should remain on inside closure"
            );
            100
        });
        assert_eq!(result, 100);

        // After closure, gate should be restored to on
        assert!(
            mirror_to_terminal_enabled(),
            "Gate should be restored to on after closure"
        );

        // Cleanup
        set_mirror_to_terminal(false);
    }

    #[test]
    fn test_print_macros_exist() {
        // These are compile-time checks — if they compile, the macros exist.
        print!("test");
        println!("test");
        print_terminal!("test");
        println_terminal!("test");
    }
}
