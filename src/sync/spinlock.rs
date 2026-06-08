//! AArch64 spinlock implementation using atomic operations

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// Raw spinlock implementation (no data protection)
pub struct SpinlockRaw {
    locked: AtomicBool,
}

impl SpinlockRaw {
    /// Create a new unlocked spinlock
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    /// Acquire the spinlock, blocking until available
    pub fn lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    /// Release the spinlock
    pub fn unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/// A RAII guard that unlocks the spinlock when dropped
pub struct SpinlockRawGuard<'a> {
    spinlock: &'a SpinlockRaw,
}

impl<'a> SpinlockRawGuard<'a> {
    fn new(spinlock: &'a SpinlockRaw) -> Self {
        spinlock.lock();
        Self { spinlock }
    }
}

impl<'a> Drop for SpinlockRawGuard<'a> {
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

    pub fn lock(&self) -> IrqSafeSpinlockGuard<'_> {
        let irq_state = disable_irqs_save();

        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

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

        if self.irq_enabled {
            crate::arch::cpu::enable_irqs();
        }
    }
}

/// Helper function to save IRQ state and disable IRQs
fn disable_irqs_save() -> bool {
    crate::arch::cpu::disable_irqs();
    false
}

/// A generic spinlock wrapper that protects arbitrary data
/// This is const-compatible for static initialization and replaces `static mut`
pub struct Spinlock<T> {
    lock: SpinlockRaw,
    data: UnsafeCell<T>,
}

// Safe to send since we only access data through the lock
unsafe impl<T: Send> Send for Spinlock<T> {}
unsafe impl<T: Send> Sync for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            lock: SpinlockRaw::new(),
            data: UnsafeCell::new(value),
        }
    }

    /// Acquire the lock and get mutable access to the data
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        self.lock.lock();
        SpinlockGuard { data: self }
    }
}

/// RAII guard for Spinlock that provides mutable access to the inner data
pub struct SpinlockGuard<'a, T> {
    data: &'a Spinlock<T>,
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.data.lock.unlock();
    }
}

impl<'a, T> Deref for SpinlockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data.data.get() }
    }
}

impl<'a, T> DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.data.data.get() }
    }
}
