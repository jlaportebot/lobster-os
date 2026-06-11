//! Panic handler for no_std bare-metal kernel

#[cfg(not(feature = "std"))]
use core::panic::PanicInfo;

#[cfg(not(feature = "std"))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::arch::cpu::disable_irqs();

    crate::println!();
    crate::println!("======================================");
    crate::println!("  PANIC! LobsterOS crashed!");
    crate::println!("======================================");

    if let Some(loc) = info.location() {
        crate::println!("  at {}:{}:{}", loc.file(), loc.line(), loc.column());
    }

    crate::println!("======================================");
    crate::println!("  System halted.");
    crate::println!();

    loop {
        crate::arch::cpu::wfe()
    }
}

/// Panic handler for std feature (host testing)
#[cfg(feature = "std")]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    use std::process;
    eprintln!("PANIC: {:?}", info);
    process::abort()
}
