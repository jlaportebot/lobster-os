//! Inter-Processor Interrupt (IPI) subsystem
//!
//! Provides:
//! - IPI sending via GIC SGI (Software Generated Interrupts)
//! - Per-core callback queues for function-call IPIs
//! - Reschedule IPIs for cross-core task migration

use crate::arch::gic;
use crate::println;
use crate::smp::{CORE_COUNT, IPI_CALL_FUNCTION, IPI_RESCHEDULE, IPI_STOP, IPI_WAKE};

/// Maximum pending callbacks per core
const MAX_PENDING_CALLBACKS: usize = 16;

/// IPI callback function type
pub type IpiCallback = fn() -> ();

/// Per-core pending IPI callback queue
struct IpiQueue {
    /// Pending callbacks
    callbacks: [Option<IpiCallback>; MAX_PENDING_CALLBACKS],
    /// Number of pending callbacks
    count: usize,
}

impl IpiQueue {
    const fn new() -> Self {
        Self {
            callbacks: [None; MAX_PENDING_CALLBACKS],
            count: 0,
        }
    }

    fn push(&mut self, cb: IpiCallback) -> bool {
        if self.count >= MAX_PENDING_CALLBACKS {
            return false;
        }
        self.callbacks[self.count] = Some(cb);
        self.count += 1;
        true
    }

    fn drain(&mut self) -> [Option<IpiCallback>; MAX_PENDING_CALLBACKS] {
        let result = self.callbacks;
        self.callbacks = [None; MAX_PENDING_CALLBACKS];
        let _count = self.count;
        self.count = 0;
        // Return count via a side-channel... or just process inline
        // We'll process inline in process_pending_callbacks
        result
    }
}

/// Per-core IPI queues
static mut IPI_QUEUES: [IpiQueue; CORE_COUNT] = [
    IpiQueue::new(),
    IpiQueue::new(),
    IpiQueue::new(),
    IpiQueue::new(),
];

/// Send a reschedule IPI to a specific core
pub fn send_reschedule(target_core: u64) {
    if target_core >= CORE_COUNT as u64 {
        return;
    }
    // Don't send IPI to ourselves
    if target_core == crate::smp::current_core_id() {
        return;
    }
    if !crate::smp::is_core_online(target_core as usize) {
        return;
    }
    unsafe {
        gic::Gic::send_sgi(IPI_RESCHEDULE, 1 << target_core as u8);
    }
}

/// Send a reschedule IPI to all other cores
pub fn broadcast_reschedule() {
    let me = crate::smp::current_core_id();
    let mut targets = 0u8;
    for i in 0..CORE_COUNT {
        if i as u64 != me && crate::smp::is_core_online(i) {
            targets |= 1 << i;
        }
    }
    if targets != 0 {
        unsafe {
            gic::Gic::send_sgi(IPI_RESCHEDULE, targets);
        }
    }
}

/// Send a function-call IPI to a specific core
/// The callback will be executed on the target core in interrupt context
pub fn send_call_function(target_core: u64, callback: IpiCallback) -> bool {
    if target_core >= CORE_COUNT as u64 {
        return false;
    }
    if !crate::smp::is_core_online(target_core as usize) {
        return false;
    }

    // Enqueue the callback
    unsafe {
        if !IPI_QUEUES[target_core as usize].push(callback) {
            println!("[ipi] Callback queue full for core {}", target_core);
            return false;
        }
    }

    // Send the IPI
    unsafe {
        gic::Gic::send_sgi(IPI_CALL_FUNCTION, 1 << target_core as u8);
    }
    true
}

/// Send a function-call IPI to all cores (except self)
pub fn broadcast_call_function(callback: IpiCallback) -> bool {
    let me = crate::smp::current_core_id();
    let mut targets = 0u8;
    let mut all_ok = true;

    for i in 0..CORE_COUNT {
        if i as u64 != me && crate::smp::is_core_online(i) {
            unsafe {
                if !IPI_QUEUES[i].push(callback) {
                    println!("[ipi] Callback queue full for core {}", i);
                    all_ok = false;
                } else {
                    targets |= 1 << i;
                }
            }
        }
    }

    if targets != 0 {
        unsafe {
            gic::Gic::send_sgi(IPI_CALL_FUNCTION, targets);
        }
    }
    all_ok
}

/// Send a stop IPI to a specific core
pub fn send_stop(target_core: u64) {
    if target_core >= CORE_COUNT as u64 {
        return;
    }
    unsafe {
        gic::Gic::send_sgi(IPI_STOP, 1 << target_core as u8);
    }
}

/// Send a wake IPI to a specific core (wake from WFE)
pub fn send_wake(target_core: u64) {
    if target_core >= CORE_COUNT as u64 {
        return;
    }
    if !crate::smp::is_core_online(target_core as usize) {
        return;
    }
    unsafe {
        gic::Gic::send_sgi(IPI_WAKE, 1 << target_core as u8);
    }
}

/// Process pending IPI callbacks for a core
/// Called from handle_irq when an IPI_CALL_FUNCTION SGI arrives
pub fn process_pending_callbacks(core_id: usize) {
    unsafe {
        let queue = &mut IPI_QUEUES[core_id];
        let count = queue.count;
        let callbacks = queue.callbacks;

        // Reset the queue
        queue.count = 0;
        queue.callbacks = [None; MAX_PENDING_CALLBACKS];

        // Execute all pending callbacks
        for i in 0..count {
            if let Some(cb) = callbacks[i] {
                cb();
            }
        }
    }
}

/// Initialize IPI subsystem
pub fn init() {
    println!("[ipi] IPI subsystem initialized (SGIs 0-3 allocated)");
}
