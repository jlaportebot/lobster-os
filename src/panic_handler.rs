//! Panic handler for no_std bare-metal kernel

use core::panic::PanicInfo;

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
