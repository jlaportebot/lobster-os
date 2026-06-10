//! Architecture abstraction layer — AArch64 and x86_64 support

pub mod cpu;

#[cfg(target_arch = "aarch64")]
pub mod exception;
#[cfg(target_arch = "aarch64")]
pub mod gic;
#[cfg(target_arch = "aarch64")]
pub mod timer;

#[cfg(target_arch = "x86_64")]
pub mod x86_64 {
    pub mod cpu;
    pub mod exception;
    pub mod lapic;
    pub mod timer;
    pub mod console;
}

// Common interrupt controller interface
#[cfg(target_arch = "aarch64")]
pub mod interrupt {
    pub use crate::arch::gic::Gic;
    pub use crate::arch::gic::Gic as InterruptController;
    // IPI functions from GIC
    pub fn send_ipi(targets: u8, vector: u32) {
        crate::arch::gic::Gic::send_sgi(vector, targets);
    }
    pub fn send_ipi_cpu(cpu: u8, vector: u32) {
        crate::arch::gic::Gic::send_sgi(vector, 1 << cpu);
    }
    pub fn send_ipi_all(_targets: u32, _vector: u32, _delivery_mode: u8) {
        // GIC send_sgi with target mask handles "all" case
        // Not needed for ARM64 as we use target mask
    }
    pub fn interrupt_init() {
        crate::arch::gic::Gic::init();
    }
    // IPI constants for ARM64 (GIC SGI 0-3)
    pub const IPI_RESCHEDULE: u32 = 0;
    pub const IPI_CALL_FUNCTION: u32 = 1;
    pub const IPI_STOP: u32 = 2;
    pub const IPI_WAKE: u32 = 3;
}

#[cfg(target_arch = "x86_64")]
pub mod interrupt {
    pub use crate::arch::x86_64::lapic::{
        current_apic_id, eoi, handle_error_irq, handle_spurious_irq, handle_timer_irq,
        init_ioapic, init_lapic, ioapic_disable_irq, ioapic_enable_irq, ioapic_set_irq,
        send_ipi, send_ipi_all, send_ipi_cpu, send_ipi_others, ERROR_VECTOR, SPURIOUS_VECTOR,
        TIMER_VECTOR, IPI_RESCHEDULE, IPI_CALL_FUNCTION, IPI_STOP, IPI_WAKE,
    };
    pub type InterruptController = ();
    pub fn interrupt_init() {
        crate::arch::x86_64::lapic::init();
    }
}

// Common timer interface
#[cfg(target_arch = "aarch64")]
pub mod timer_iface {
    pub use crate::arch::timer::Timer;
}

#[cfg(target_arch = "x86_64")]
pub mod timer_iface {
    pub use crate::arch::x86_64::timer::Timer;
}

// Re-exports for current target
#[cfg(target_arch = "aarch64")]
pub use self::cpu::*;
#[cfg(target_arch = "aarch64")]
pub use self::exception::*;
#[cfg(target_arch = "aarch64")]
pub use self::gic::*;
#[cfg(target_arch = "aarch64")]
pub use self::timer::*;

#[cfg(target_arch = "x86_64")]
pub use self::x86_64::cpu::*;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64::exception::*;
#[cfg(target_arch = "x86_64")]
pub use self::interrupt::*;
#[cfg(target_arch = "x86_64")]
pub use self::timer_iface::Timer;

// SMP uses interrupt controller
#[cfg(target_arch = "aarch64")]
pub use self::gic::Gic as SmpInterruptController;
#[cfg(target_arch = "x86_64")]
pub use self::interrupt::InterruptController as SmpInterruptController;

// Timer re-export for drivers
pub use self::timer_iface::Timer as ArchTimer;
