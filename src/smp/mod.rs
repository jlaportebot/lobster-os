//! SMP (Symmetric Multiprocessing) support for Raspberry Pi 5

use crate::println;

/// Boot secondary cores using the spin-table method
pub fn boot_secondary_cores() {
    println!("[smp] Booting secondary cores...");
    
    // Boot cores 1, 2, and 3
    for core_id in 1..4 {
        boot_secondary_core(core_id);
    }
    
    println!("[smp] Secondary cores booted");
}

/// Boot a specific secondary core
fn boot_secondary_core(core_id: u64) {
    // Calculate the spin-table address for this core
    // RPi5 spin table for secondary cores: firmware places core boot vectors at 0xD8, 0xE0, 0xE8 per core
    let spin_table_addr = 0xD8 + (core_id * 8);
    
    // Write the secondary entry point address to the spin-table location
    // This will cause the core to jump to our secondary_init function
    let entry_point = secondary_entry as *const fn() as u64;
    
    unsafe {
        core::ptr::write_volatile(spin_table_addr as *mut u64, entry_point);
    }
    
    println!("[smp] Core {} boot vector written to {:#010x}", core_id, spin_table_addr);
}

/// Entry point for secondary cores
extern "C" fn secondary_entry() -> ! {
    // This function is called by secondary cores when they boot
    // It should set up the GIC CPU interface, enable MMU, and enter the scheduler
    
    let core_id = crate::arch::cpu::mpidr();
    println!("[smp] Secondary core {} online", core_id);
    
 // Set up GIC CPU interface
 unsafe { crate::arch::gic::Gic::init_cpu_interface(); }
    
    // Enable MMU (same page tables as core 0)
    // This is already done in the boot process
    
    // Enable IRQs
    crate::arch::cpu::enable_irqs();
    
    // Enter the scheduler idle loop
    loop {
        crate::arch::cpu::wfe();
    }
}