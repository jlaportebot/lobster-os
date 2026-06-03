//! AArch64 spinlock implementation using atomic operations

use core::sync::atomic::{AtomicBool, Ordering};

/// A spinlock using atomic operations
pub struct Spinlock {
    locked: AtomicBool,
}

impl Spinlock {
    /// Create a new unlocked spinlock
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    /// Acquire the spinlock, blocking until available
    pub fn lock(&self) {
        // Try to acquire the lock with atomic compare-and-swap
        while self.locked.compare_exchange(
            false, 
            true, 
            Ordering::Acquire,
            Ordering::Relaxed
        ).is_err() {
            // Wait for lock to be released
            core::hint::spin_loop();
        }
    }

    /// Release the spinlock
    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/// A RAII guard that unlocks the spinlock when dropped
pub struct SpinlockGuard<'a> {
    spinlock: &'a Spinlock,
}

impl<'a> SpinlockGuard<'a> {
    pub fn new(spinlock: &'a Spinlock) -> Self {
        spinlock.lock();
        Self { spinlock }
    }
}

impl<'a> Drop for SpinlockGuard<'a> {
    fn drop(&mut self) {
        self.spinlock.unlock();
    }
}

/// An IRQ-safe variant that disables interrupts while holding the lock
pub struct IrqSafeSpinlock {
    locked: AtomicBool,
}

impl IrqSafeSpinlock {
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) -> IrqSafeSpinlockGuard {
        // Disable IRQs completely while we're trying to acquire the lock
        let irq_state = disable_irqs_save();
        
        // Try to acquire the lock
        while self.locked.compare_exchange(
            false, 
            true, 
            Ordering::Acquire,
            Ordering::Relaxed
        ).is_err() {
            // Wait for lock to be released
            core::hint::spin_loop();
        }
        
        // Return a guard that will restore IRQ state when dropped
        IrqSafeSpinlockGuard {
            spinlock: self,
            irq_enabled: irq_state,
        }
    }
}

pub struct IrqSafeSpinlockGuard<'a> {
    spinlock: &'a IrqSafeSpinlock,
    irq_enabled: bool,
}

impl<'a> Drop for IrqSafeSpinlockGuard<'a> {
    fn drop(&mut self) {
        self.spinlock.locked.store(false, Ordering::Release);
        
        // Restore IRQ state
        if self.irq_enabled {
            crate::arch::cpu::enable_irqs();
        }
    }
}

/// Helper function to save IRQ state and disable IRQs
fn disable_irqs_save() -> bool {
    // This is a placeholder - in a real implementation we would
    // check the current IRQ state and disable IRQs
    crate::arch::cpu::disable_irqs();
    false // Return false to indicate IRQs were disabled
}