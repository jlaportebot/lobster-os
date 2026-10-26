//! Process lifecycle management — full process isolation support
//!
//! Provides:
//! - fork(): clone address space via COW, duplicate Task struct
//! - exec(): load new ELF image, reset registers, keep PID
//! - exit(): mark zombie, free address space, wake parent
//! - wait(): block parent until child exits
//! - mmap()/munmap(): VMA-based anonymous memory mapping
//! - brk(): program break adjustment

use crate::memory::frame_alloc;
use crate::memory::process_vmem::{
    self, cow_mark_all, destroy_address_space, map_user_page, AddressSpace, PTE_USER_RW,
    PTE_USER_RX, USER_HEAP_START,
};
use crate::memory::{PhysAddr, PAGE_SIZE};
use crate::println;
use crate::sched::elf;
use crate::sched::{self, find_task_index, get_next_pid, Task, TaskState, TrapFrame, TASK_TABLE};

/// Error codes for process operations
const EAGAIN: i64 = -11;
const ENOMEM: i64 = -12;
const EINVAL: i64 = -22;
const ECHILD: i64 = -10;
const ESRCH: i64 = -3;

/// Fork the current process
/// Returns child PID to parent, 0 to child, -err to parent on failure
pub fn fork() -> i64 {
    let parent_pid = sched::current_pid();

    // Find parent task
    let parent_idx = match find_task_index(parent_pid) {
        Some(idx) => idx,
        None => {
            println!("[process] fork: parent PID {} not found!", parent_pid);
            return ESRCH;
        }
    };

    // Clone address space using COW
    // Build an AddressSpace from the parent's ttbr0_phys for cloning
    // (the parent's VMAs/brk are not tracked per-task currently, so we
    //  construct a minimal AddressSpace; clone_address_space copies the
    //  actual page tables and handles COW marking)
    let parent_ttbr0 = unsafe { TASK_TABLE[parent_idx].as_ref().unwrap().ttbr0_phys };
    let parent_space = AddressSpace {
        ttbr0_phys: parent_ttbr0,
        vmas: [process_vmem::Vma {
            start: 0,
            size: 0,
            prot: 0,
            file_offset: 0,
            shared: false,
            in_use: false,
        }; process_vmem::MAX_VMAS],
        vma_count: 0,
        brk: 0,
    };

    let new_space = if let Some(cloned_space) = process_vmem::clone_address_space(&parent_space) {
        cloned_space
    } else {
        println!("[process] fork: failed to clone address space");
        return ENOMEM;
    };

    // Allocate new task slot
    let child_pid = get_next_pid();

    // Clone the parent task
    let parent_task = unsafe { TASK_TABLE[parent_idx].as_ref().unwrap() };
    let mut child_task = Task {
        pid: child_pid,
        parent_pid,
        pgid: parent_task.pgid,
        sid: parent_task.sid,
        uid: parent_task.uid,
        euid: parent_task.euid,
        gid: parent_task.gid,
        egid: parent_task.egid,
        groups: parent_task.groups,
        group_count: parent_task.group_count,
        state: TaskState::Ready,
        priority: parent_task.priority,
        affinity: parent_task.affinity,
        cpu_id: 0, // Will be assigned on first schedule
        runtime_ticks: 0,
        time_slice: parent_task.time_slice,
        nice: parent_task.nice,
        regs: parent_task.regs,
        // Q3: fork copies the parent's saved FP state too.
        fp: parent_task.fp,
        kernel_sp: 0, // Will allocate new kernel stack
        kernel_stack_base: 0,
        user_sp: parent_task.user_sp,
        user_stack_base: parent_task.user_stack_base,
        ttbr0_phys: new_space.ttbr0_phys,
        entry_point: parent_task.entry_point,
        is_user: parent_task.is_user,
        ipc_ns: parent_task.ipc_ns,           // Inherit IPC namespace
        pid_ns: parent_task.pid_ns,           // Inherit PID namespace
        mnt_ns: parent_task.mnt_ns,           // Inherit mount namespace
        net_ns: parent_task.net_ns,           // Inherit network namespace
        uts_ns: parent_task.uts_ns,           // Inherit UTS namespace
        user_ns: parent_task.user_ns,         // Inherit user namespace
        pending_signals: 0,                   // Child starts with no pending signals
        signal_mask: parent_task.signal_mask, // Inherit signal mask
        signal_handlers: parent_task.signal_handlers, // Inherit handlers
        pipe_read_fd: -1,                     // Obsoleted by per-process fd table
        pipe_write_fd: -1,
        exit_code: 0,
        exit_signal: 0,
        bg: false,
        wake_ms: 0,
        rr_ticket: 0,
    };

    // Allocate kernel stack for child
    if let Some((kern_sp, kern_base)) = alloc_kernel_stack() {
        child_task.kernel_sp = kern_sp;
        child_task.kernel_stack_base = kern_base;
    } else {
        destroy_address_space(&new_space);
        println!("[process] fork: out of kernel stacks");
        return ENOMEM;
    }

    // Set up registers for child: return 0 to child
    child_task.regs.x0 = 0;

    // Mark COW on cloned pages
    unsafe {
        cow_mark_all(child_task.ttbr0_phys);
    }

    // Add to VMA tracking
    // (the cloned address space already has VMAs, just need to ensure brk is set)
    // Note: We need to store the new_address_space somewhere. For simplicity,
    // we're storing ttbr0_phys. In a full implementation, we'd have an AddressSpace
    // field per task.

    // Insert child task
    let inserted = unsafe {
        let mut found = None;
        for i in 0..sched::MAX_TASKS {
            if TASK_TABLE[i].is_none() {
                TASK_TABLE[i] = Some(child_task);
                found = Some(i);
                break;
            }
        }
        found
    };

    if inserted.is_none() {
        // Clean up
        destroy_address_space(&new_space);
        println!("[process] fork: no free task slots");
        return ENOMEM;
    }

    // Mark parent's pages as COW after fork
    unsafe {
        cow_mark_all(TASK_TABLE[parent_idx].as_ref().unwrap().ttbr0_phys);
    }

    // Clone parent's fd table into child (increment IPC refcounts)
    if let Some(child_idx) = inserted {
        crate::ipc::clone_fd_table(parent_idx, child_idx, false);
        // Increment IPC namespace refcount for the child process
        crate::ipc::ipc_ns_incref(parent_task.ipc_ns);

        // Clone parent's credentials for child (uid/gid/capabilities inherit across fork)
        let p_uid = parent_task.uid;
        let p_gid = parent_task.gid;
        crate::security::clone_credentials(parent_idx, child_idx, p_uid, p_gid);
    }

    println!(
        "[process] fork: PID {} created child PID {}",
        parent_pid, child_pid
    );
    child_pid as i64
}

/// Execute a new ELF image in the current process
/// Returns on error (does not return on success)
pub fn exec(path: &str) -> i64 {
    let pid = sched::current_pid();
    println!("[process] exec: PID {} loading {:?}", pid, path);

    // 1. Read the ELF binary from VFS
    let elf_data = match crate::fs::vfs::read_file(path) {
        Ok(data) => data,
        Err(e) => {
            println!("[process] exec: failed to open {:?}: {:?}", path, e);
            return EINVAL;
        }
    };

    if elf_data.is_empty() {
        println!("[process] exec: empty ELF file");
        return EINVAL;
    }

    // 2. Load the ELF into a new address space
    let load_result = match elf::load_elf(&elf_data) {
        Ok(r) => r,
        Err(e) => {
            println!("[process] exec: ELF load failed: {}", e);
            return EINVAL;
        }
    };

    // 3. Destroy the old user address space
    let task_idx = match find_task_index(pid) {
        Some(idx) => idx,
        None => return ESRCH,
    };

    unsafe {
        if let Some(ref mut task) = TASK_TABLE[task_idx] {
            // Free old address space
            if task.ttbr0_phys != 0 {
                let old_space = AddressSpace {
                    ttbr0_phys: task.ttbr0_phys,
                    vmas: [process_vmem::Vma {
                        start: 0,
                        size: 0,
                        prot: 0,
                        file_offset: 0,
                        shared: false,
                        in_use: false,
                    }; process_vmem::MAX_VMAS],
                    vma_count: 0,
                    brk: 0,
                };
                destroy_address_space(&old_space);
            }

            // 4. Install new address space
            task.ttbr0_phys = load_result.ttbr0_phys;
            task.entry_point = load_result.entry_point;
            task.user_sp = elf::USER_STACK_TOP;
            task.is_user = true;

            // 5. Reset trap frame to enter at new entry point
            task.regs = TrapFrame::new();
            task.regs.elr = load_result.entry_point;
            task.regs.sp = elf::USER_STACK_TOP;
            task.regs.spsr = 0x0000; // EL0t, all IRQs unmasked
            task.regs.x30 = load_result.entry_point;

            // Switch to the new address space immediately
            switch_address_space(load_result.ttbr0_phys);

            // 6. Credential transformation on exec (setuid/setgid binaries)
            {
                let (file_uid, file_gid, file_mode) = match crate::fs::vfs::resolve_path(path) {
                    Ok(id) => crate::fs::vfs::get_node(id)
                        .map(|n| (n.metadata.uid, n.metadata.gid, n.metadata.permissions))
                        .unwrap_or((0, 0, 0)),
                    Err(_) => (0, 0, 0),
                };
                crate::security::execve_credential_transform(
                    task_idx, file_uid, file_gid, file_mode,
                );
            }

            println!(
                "[process] exec: PID {} now at entry={:#018x} ttbr0={:#010x}",
                pid, load_result.entry_point, load_result.ttbr0_phys
            );
        }
    }

    0 // Success — the scheduler will eret to the new entry point
}

/// Switch to a user process address space (public for exec)
unsafe fn switch_address_space(ttbr0_phys: u64) {
    core::arch::asm!("msr TTBR0_EL1, {}", in(reg) ttbr0_phys);
    core::arch::asm!("tlbi vmalle1is");
    crate::arch::cpu::dsb();
    crate::arch::cpu::isb();
}

/// Exit the current process
pub fn exit(code: i64) -> ! {
    let pid = sched::current_pid();
    println!("[process] exit: PID {} with code {}", pid, code);

    // Mark as zombie
    if let Some(idx) = find_task_index(pid) {
        // Close all open file descriptors for this process
        crate::ipc::close_all_fds(idx);

        // Release IPC namespace refcount
        let ns_id = unsafe { TASK_TABLE[idx].as_ref().map(|t| t.ipc_ns).unwrap_or(0) };
        crate::ipc::ipc_ns_release(ns_id);

        // Release credentials for this process
        crate::security::free_credentials(idx);

        unsafe {
            if let Some(ref mut task) = TASK_TABLE[idx] {
                task.state = TaskState::Zombie;
                task.exit_code = code;
            }
        }

        // Free address space if user process
        unsafe {
            if let Some(ref task) = TASK_TABLE[idx] {
                if task.is_user && task.ttbr0_phys != 0 {
                    let space = AddressSpace {
                        ttbr0_phys: task.ttbr0_phys,
                        vmas: [process_vmem::Vma {
                            start: 0,
                            size: 0,
                            prot: 0,
                            file_offset: 0,
                            shared: false,
                            in_use: false,
                        }; process_vmem::MAX_VMAS],
                        vma_count: 0,
                        brk: 0,
                    };
                    destroy_address_space(&space);
                }
            }
        }

        // Wake parent if waiting
        let parent_pid = unsafe { TASK_TABLE[idx].as_ref().unwrap().parent_pid };
        if parent_pid != 0 {
            // Mark parent as ready if it was waiting
            if let Some(pidx) = find_task_index(parent_pid) {
                unsafe {
                    if let Some(ref mut parent) = TASK_TABLE[pidx] {
                        if parent.state == TaskState::Blocked {
                            parent.state = TaskState::Ready;
                        }
                    }
                }
            }
        }
    }

    // Yield to scheduler (will eventually call idle or another task)
    sched::yield_task();
    panic!("exit should never return");
}

/// Wait for a child process
/// Returns child PID on success, -1 on error
pub fn wait(pid: u64) -> i64 {
    let parent_pid = sched::current_pid();

    // Find parent task
    let _parent_idx = match find_task_index(parent_pid) {
        Some(idx) => idx,
        None => return ESRCH,
    };

    // Look for zombie children
    let mut found_zombie = None;
    unsafe {
        for i in 0..sched::MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.parent_pid == parent_pid && task.state == TaskState::Zombie {
                    if pid == 0 || task.pid == pid || pid == u64::MAX {
                        found_zombie = Some(i);
                        break;
                    }
                }
            }
        }
    }

    if let Some(zombie_idx) = found_zombie {
        // Reap the zombie
        let exit_code = unsafe { TASK_TABLE[zombie_idx].as_ref().unwrap().exit_code };

        // Free resources
        unsafe {
            if let Some(ref task) = TASK_TABLE[zombie_idx] {
                if task.is_user && task.ttbr0_phys != 0 {
                    let space = AddressSpace {
                        ttbr0_phys: task.ttbr0_phys,
                        vmas: [process_vmem::Vma {
                            start: 0,
                            size: 0,
                            prot: 0,
                            file_offset: 0,
                            shared: false,
                            in_use: false,
                        }; process_vmem::MAX_VMAS],
                        vma_count: 0,
                        brk: 0,
                    };
                    destroy_address_space(&space);
                }
                TASK_TABLE[zombie_idx] = None;
            }
        }

        println!(
            "[process] wait: reaped child PID {}, exit code {}",
            unsafe { TASK_TABLE[zombie_idx].as_ref().unwrap().pid },
            exit_code
        );
        return exit_code;
    }

    // Check if any children exist (not yet exited)
    let has_children = unsafe {
        for i in 0..sched::MAX_TASKS {
            if let Some(ref task) = TASK_TABLE[i] {
                if task.parent_pid == parent_pid {
                    // Child exists but isn't a zombie
                    // Block waiting for it
                    if let Some(pidx) = find_task_index(parent_pid) {
                        TASK_TABLE[pidx].as_mut().unwrap().state = TaskState::Blocked;
                    }
                    return -1; // Will be woken when child exits
                }
            }
        }
        false
    };

    if !has_children {
        return ECHILD; // No children
    }

    // Block until child exits (handled above)
    loop {
        crate::arch::cpu::wfe();
        // Re-check for zombies
        unsafe {
            for i in 0..sched::MAX_TASKS {
                if let Some(ref task) = TASK_TABLE[i] {
                    if task.parent_pid == parent_pid && task.state == TaskState::Zombie {
                        if pid == 0 || pid == u64::MAX || task.pid == pid {
                            let exit_code = task.exit_code;
                            if task.is_user && task.ttbr0_phys != 0 {
                                let space = AddressSpace {
                                    ttbr0_phys: task.ttbr0_phys,
                                    vmas: [process_vmem::Vma {
                                        start: 0,
                                        size: 0,
                                        prot: 0,
                                        file_offset: 0,
                                        shared: false,
                                        in_use: false,
                                    };
                                        process_vmem::MAX_VMAS],
                                    vma_count: 0,
                                    brk: 0,
                                };
                                destroy_address_space(&space);
                            }
                            TASK_TABLE[i] = None;
                            return exit_code;
                        }
                    }
                }
            }
        }
    }
}

/// Memory map for user process
/// addr: hint address (can be NULL/0 for kernel)
/// len: length in bytes
/// prot: protection flags (PROT_READ, PROT_WRITE, PROT_EXEC)
/// flags: MAP flags (MAP_PRIVATE, MAP_SHARED, MAP_ANONYMOUS)
pub fn mmap(addr: u64, len: usize, prot: u32, flags: u32) -> i64 {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(idx) => idx,
        None => return -1,
    };

    // For now, only support anonymous private mappings
    let is_anon = (flags & 0x20) != 0; // MAP_ANONYMOUS
    let is_private = (flags & 0x02) != 0; // MAP_PRIVATE

    if !is_anon || !is_private {
        println!("[process] mmap: only anonymous private mappings supported");
        return -1;
    }

    // Calculate page-aligned length
    let len_pages = (len + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;
    let len_aligned = (len_pages as u64) * PAGE_SIZE;

    // Determine address
    let map_addr = if addr == 0 {
        // Use heap area
        USER_HEAP_START
    } else {
        addr & !(PAGE_SIZE - 1)
    };

    // Allocate frames and map them
    let prot_flags = if prot & 1 != 0 && prot & 2 != 0 {
        PTE_USER_RW // PROT_READ | PROT_WRITE
    } else if prot & 1 != 0 && prot & 4 != 0 {
        PTE_USER_RX // PROT_READ | PROT_EXEC
    } else if prot & 1 != 0 {
        PTE_USER_RW // PROT_READ (use RW since RO not exported)
    } else {
        PTE_USER_RW // Default to readable/writable
    };

    unsafe {
        let task = TASK_TABLE[task_idx].as_mut().unwrap();
        if task.ttbr0_phys == 0 {
            println!("[process] mmap: task has no address space");
            return -1;
        }

        for i in 0..len_pages {
            let frame = match frame_alloc::allocate_frame() {
                Some(f) => f,
                None => {
                    println!("[process] mmap: out of frames at page {}", i);
                    // Unmap already mapped pages
                    for j in 0..i {
                        let va = map_addr + (j as u64) * PAGE_SIZE;
                        process_vmem::unmap_user_page(task.ttbr0_phys, va);
                    }
                    return -1;
                }
            };

            let va = map_addr + (i as u64) * PAGE_SIZE;
            map_user_page(task.ttbr0_phys, va, frame.0, prot_flags);

            // Add VMA
            let vma = process_vmem::Vma {
                start: va,
                size: len_aligned,
                prot: prot_flags,
                file_offset: 0,
                shared: !is_private,
                in_use: true,
            };
            // Note: We'd need to update the AddressSpace here
            // For now, just skip VMA tracking (simplified)
            let _ = vma;
        }
    }

    println!(
        "[process] mmap: mapped {} pages at {:#018x}",
        len_pages, map_addr
    );
    map_addr as i64
}

/// Unmap memory
pub fn munmap(addr: u64, len: usize) -> i64 {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(idx) => idx,
        None => return -1,
    };

    let page_addr = addr & !(PAGE_SIZE - 1);
    let len_pages = (len + PAGE_SIZE as usize - 1) / PAGE_SIZE as usize;

    unsafe {
        let task = TASK_TABLE[task_idx].as_mut().unwrap();
        if task.ttbr0_phys == 0 {
            return -1;
        }

        for i in 0..len_pages {
            let va = page_addr + (i as u64) * PAGE_SIZE;
            let opt_pa = process_vmem::unmap_user_page(task.ttbr0_phys, va);
            if let Some(pa) = opt_pa {
                frame_alloc::deallocate_frame(PhysAddr::new(pa));
            }
        }
    }

    println!(
        "[process] munmap: unmapped {} pages at {:#018x}",
        len_pages, page_addr
    );
    0
}

/// Program break adjustment (brk system call)
pub fn brk(new_break: u64) -> i64 {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(idx) => idx,
        None => return -1,
    };

    unsafe {
        let task = TASK_TABLE[task_idx].as_mut().unwrap();
        if task.ttbr0_phys == 0 {
            return -1;
        }

        // Align to page boundary
        let _break_aligned = (new_break + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

        // TODO: Implement actual brk adjustment
        // This would involve:
        // 1. Checking if break can move (within limits)
        // 2. Mapping/unmapping pages as needed
        // 3. Updating task's program break

        // For now, just return success
    }

    0
}

/// Handle page fault from exception handler
pub fn handle_page_fault(va: u64, is_write: bool) -> bool {
    let pid = sched::current_pid();
    let task_idx = match find_task_index(pid) {
        Some(idx) => idx,
        None => return false,
    };

    let ttbr0_phys = unsafe { TASK_TABLE[task_idx].as_ref().unwrap().ttbr0_phys };

    if ttbr0_phys == 0 {
        // Kernel page fault - not a user fault
        return false;
    }

    // Check if this is a COW fault
    if is_write {
        let page_addr = va & !(PAGE_SIZE - 1);
        if process_vmem::handle_cow_fault(ttbr0_phys, page_addr) {
            println!(
                "[process] page fault: handled COW fault at {:#018x}",
                page_addr
            );
            return true;
        }
    }

    // Check if there's a VMA for this address
    // (simplified: for now, just try to handle as COW)

    println!(
        "[process] page fault: unhandled fault at {:#018x} (write={})",
        va, is_write
    );
    false
}

/// Map shared memory pages into a process's address space.
/// Returns the virtual address where the region was mapped.
pub fn mmap_shm(ttbr0: u64, base_phys: u64, size_pages: usize, writable: bool) -> Option<u64> {
    use crate::memory::process_vmem::{map_user_page, PTE_USER_RW, PTE_USER_RX};

    // Map at USER_HEAP_START (simplified — a real impl would find a free VMA)
    let map_addr = crate::memory::process_vmem::USER_HEAP_START;
    let prot_flags = if writable { PTE_USER_RW } else { PTE_USER_RX };

    for i in 0..size_pages {
        let va = map_addr + (i as u64) * crate::memory::PAGE_SIZE;
        let pa = base_phys + (i as u64) * crate::memory::PAGE_SIZE;
        unsafe {
            map_user_page(ttbr0, va, pa, prot_flags);
        }
    }

    Some(map_addr)
}

/// Allocate a kernel stack
fn alloc_kernel_stack() -> Option<(u64, u64)> {
    use crate::memory::frame_alloc;
    use crate::memory::{phys_to_virt, PAGE_SIZE};

    const KERN_STACK_PAGES: usize = 8;
    const KERN_STACK_SIZE: usize = KERN_STACK_PAGES * PAGE_SIZE as usize;

    let base_phys = frame_alloc::allocate_contiguous(KERN_STACK_PAGES)?;
    let base_virt = phys_to_virt(PhysAddr::new(base_phys.0)).0;

    // Map all pages
    for i in 0..KERN_STACK_PAGES {
        let offset = (i as u64) * PAGE_SIZE;
        unsafe {
            crate::memory::vmem::map_kernel_page(
                base_virt + offset,
                base_phys.0 + offset,
                crate::memory::vmem::PTE_KERNEL_DATA,
            );
        }
    }

    // Stack pointer is at the top (grows downward)
    let sp = base_virt + KERN_STACK_SIZE as u64;
    Some((sp, base_virt))
}
