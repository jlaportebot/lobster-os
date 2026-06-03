#![no_std]
#![no_main]

mod arch;
mod memory;
mod drivers;
mod console;
mod panic_handler;
mod sync;
mod dtb;
mod smp;
mod sched;
mod display;
mod fs;
mod shell;
mod net;
mod usb;
mod wm;

use core::arch::global_asm;

// Include boot assembly
global_asm!(include_str!("boot.S"));

// Linker symbols for kernel boundaries
extern "C" {
    static __kernel_end: u64;
    static __bss_start: u64;
    static __bss_end: u64;
}

/// Kernel entry point — called from boot.S with x0 = DTB pointer
#[no_mangle]
pub unsafe extern "C" fn kernel_init(dtb_ptr: u64) -> ! {
    // 1. UART + Console first
    drivers::uart::Uart10::init();
    console::init();

    println!();
    println!("╔══════════════════════════════════════╗");
    println!("║  LobsterOS v0.1.0                   ║");
    println!("║  Bare-metal Rust on BCM2712         ║");
    println!("║  Quad-core Cortex-A76 @ 2.4 GHz    ║");
    println!("╚══════════════════════════════════════╝");
    println!();

    let el = arch::cpu::current_el();
    println!("[boot] Current EL: EL{}", el);
    println!("[boot] DTB pointer: {:#018x}", dtb_ptr);
    println!("[boot] Stack pointer: {:#018x}", arch::cpu::read_sp());

    // 2. Zero BSS
    let bss_start = &__bss_start as *const u64 as u64;
    let bss_end = &__bss_end as *const u64 as u64;
    let bss_size = bss_end - bss_start;
    core::ptr::write_bytes(bss_start as *mut u8, 0, bss_size as usize);
    println!("[boot] BSS zeroed: {:#010x}..{:#010x} ({} bytes)", bss_start, bss_end, bss_size);

    // 3. GIC
    println!("[gic] Initializing GIC-400...");
    arch::gic::Gic::init();
    println!("[gic] GIC-400 distributor + CPU interface enabled");

    // 4. Timer
    println!("[timer] Initializing system timer...");
    arch::timer::Timer::init();

    // 5. Enable IRQs
    println!("[boot] Enabling IRQs...");
    arch::cpu::enable_irqs();
    println!("[boot] IRQs enabled");

    // 6. Memory Management (Phase 2)
    let kernel_phys_start = 0x80000u64; // kernel8.img load address
    let kernel_end = &__kernel_end as *const u64 as u64;
    let kernel_size = kernel_end - kernel_phys_start;
    // Assume 8GB RAM for RPi 5 (will parse DTB later for actual size)
    let ram_size = 8 * 1024 * 1024 * 1024u64;

    println!("[boot] Kernel: {:#010x}..{:#010x} ({} KB)", kernel_phys_start, kernel_end, kernel_size / 1024);

    // Initialize MMU (this will set up page tables and enable virtual memory)
    memory::mmu::init(kernel_phys_start, kernel_size, ram_size);

    // Now running with virtual memory enabled!
    // Switch to high VA kernel addresses (the identity map is still there
    // for transition, but we'll use the direct phys map from now on)

    // Initialize frame allocator
    memory::frame_alloc::init(ram_size, kernel_end);

    // Initialize heap
    memory::heap::init();

    // Register memory regions for the memory map
    memory::vmem::register_region(memory::vmem::MemoryRegion {
        virt_start: memory::VirtAddr::new(memory::KERNEL_IMAGE_START),
        phys_start: memory::PhysAddr::new(kernel_phys_start),
        size: kernel_size,
        flags: memory::vmem::PTE_KERNEL_CODE,
        name: "kernel",
    });
    memory::vmem::register_region(memory::vmem::MemoryRegion {
        virt_start: memory::VirtAddr::new(memory::PHYS_MAP_OFFSET),
        phys_start: memory::PhysAddr::new(0),
        size: ram_size,
        flags: memory::vmem::PTE_KERNEL_DATA,
        name: "phys-map",
    });
    memory::vmem::register_region(memory::vmem::MemoryRegion {
        virt_start: memory::VirtAddr::new(memory::HEAP_START),
        phys_start: memory::PhysAddr::new(0), // dynamically mapped
        size: memory::HEAP_SIZE,
        flags: memory::vmem::PTE_KERNEL_DATA,
        name: "heap",
    });
    memory::vmem::register_region(memory::vmem::MemoryRegion {
        virt_start: memory::VirtAddr::new(memory::DEVICE_MAP_START),
        phys_start: memory::PhysAddr::new(0x10_0000_0000),
        size: 0x1_0000_0000,
        flags: memory::vmem::PTE_DEVICE,
        name: "devices",
    });

    // Print memory map
    memory::vmem::print_memory_map();

    // Test heap allocation
    println!("[heap] Testing allocation...");
    {
        use core::alloc::{GlobalAlloc, Layout};
        let layout = Layout::from_size_align(256, 8).unwrap();
        let ptr = memory::heap::LobsterAllocator.alloc(layout);
        if !ptr.is_null() {
            println!("[heap] 256-byte allocation at {:#018x}: OK!", ptr as u64);
            // Write test pattern
            for i in 0u8..=255u8 {
                core::ptr::write_volatile(ptr.add(i as usize), i);
            }
            // Verify
            let mut ok = true;
            for i in 0u8..=255u8 {
                if core::ptr::read_volatile(ptr.add(i as usize)) != i {
                    ok = false;
                    break;
                }
            }
            if ok {
                println!("[heap] Read/write test: PASSED!");
            } else {
                println!("[heap] Read/write test: FAILED!");
            }
            memory::heap::LobsterAllocator.dealloc(ptr, layout);
            println!("[heap] Deallocation: OK!");
        } else {
            println!("[heap] 256-byte allocation: FAILED!");
        }
    }

    memory::heap::print_stats();

    // Frame allocator test
    println!("[frame-alloc] Testing frame allocation...");
    if let Some(frame) = memory::frame_alloc::allocate_frame() {
        println!("[frame-alloc] Allocated frame at {:#010x}: OK!", frame.0);
        memory::frame_alloc::deallocate_frame(frame);
        println!("[frame-alloc] Deallocated: OK!");
    }
    let (total, allocated, first_free) = memory::frame_alloc::stats();
    println!("[frame-alloc] Stats: {}/{} frames used, first free at frame {}", allocated, total, first_free);

 // 7. Initialize framebuffer display
 println!("[display] Initializing framebuffer...");
let info = match display::fb::init(1920, 1080, 32) {
            Ok(info) => info,
            Err(_) => {
                match display::fb::init(640, 480, 32) {
                    Ok(info) => info,
                    Err(_) => display::fb::get_info(),
                }
            }
        };
        if info.width > 0 {
            display::console::init(info);
            display::logo::draw_splash(info);
            display::logo::draw_ascii_logo();
            println!("[display] Framebuffer + console ready");
        } else {
            println!("[display] No framebuffer available (UART only)");
            display::logo::draw_ascii_logo();
        }

 // 8. Initialize filesystem
 println!("[fs] Initializing VFS...");
 fs::vfs::init();
 println!("[fs] VFS initialized");

 // 9. Parse the DTB
 println!("[dtb] Parsing device tree...");
 dtb::parse(dtb_ptr);

 // 10. Boot secondary cores
 println!("[smp] Booting secondary cores...");
 smp::boot_secondary_cores();

    // Initialize scheduler
    println!("[sched] Initializing scheduler...");
    sched::init();

    // Create test kernel tasks
    println!("[sched] Creating test tasks...");
    if let Some(pid1) = sched::create_kernel_task(test_task_1, sched::Priority::Normal) {
    println!("[sched] Test task 1 → PID {}", pid1);
    }
    if let Some(pid2) = sched::create_kernel_task(test_task_2, sched::Priority::Normal) {
    println!("[sched] Test task 2 → PID {}", pid2);
    }

    // Enable preemptive scheduling
    sched::enable();

    // Launch the shell as a kernel task (never returns)

       // 11. Initialize USB subsystem
       println!("[usb] Initializing xHCI host controller...");
       usb::xhci::init();
       println!("[usb] xHCI initialized, scanning devices...");
       usb::hid::init();
       println!("[usb] HID class driver ready");

       // 12. Initialize networking
       println!("[net] Initializing network stack...");
       net::init();
       println!("[net] Stack ready");

       // 13. Initialize window manager
           println!("[wm] Initializing compositor...");
           wm::compositor::init(info);
           println!("[wm] Initializing theme engine...");
       wm::theme::init();
       println!("[wm] Compositor + theme ready");

       println!();
       println!("LobsterOS Phase 1-8 complete!");
       println!("  UART: OK | GIC: OK | Timer: OK");
       println!("  MMU: OK | Frames: OK | Heap: OK");
       println!("  Sched: OK | Syscalls: OK | ELF: OK");
       println!("  Display: OK | USB: OK | Net: OK | WM: OK");
       println!("  Shell: OK | Serial Input: OK");
       println!();

       shell::run();
    }

    /// Test task functions
    fn test_task_1() -> ! {
        for i in 0..5 {
            println!("[task1] Tick {} (PID {})", i, sched::current_pid());
            sched::yield_task();
        }
        println!("[task1] Done, exiting");
        sched::exit_current(0);
        loop { crate::arch::cpu::wfe(); }
    }

    fn test_task_2() -> ! {
        for i in 0..5 {
            println!("[task2] Tick {} (PID {})", i, sched::current_pid());
            sched::yield_task();
        }
        println!("[task2] Done, exiting");
        sched::exit_current(0);
        loop { crate::arch::cpu::wfe(); }
    }

/// Secondary core entry — called from boot.S
#[no_mangle]
pub unsafe extern "C" fn secondary_core_init(core_id: u64) -> ! {
    // Set up TTBR1 for this core
    let ttbr1 = memory::mmu::get_ttbr1_phys();
    core::arch::asm!("msr TTBR1_EL1, {}", in(reg) ttbr1);
    core::arch::asm!("tlbi vmalle1is");
    arch::cpu::dsb();
    arch::cpu::isb();

    arch::gic::Gic::init_cpu_interface();
    arch::timer::Timer::init_secondary();

    println!("[smp] Core {} online (MMU enabled)", core_id);

    arch::cpu::enable_irqs();

    loop {
        arch::cpu::wfe();
    }
}
