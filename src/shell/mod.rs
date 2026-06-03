//! Lobster Shell — serial console command interpreter
//!
//! Features:
//! - Line-at-a-time command processing via serial_input
//! - Built-in commands: help, clear, uptime, mem, ps, kill, echo, reboot, halt
//! - Command argument parsing
//! - Prompt with OS name
//! - Integrates with scheduler, memory, and process subsystems

use crate::drivers::serial_input;
use crate::drivers::uart::Uart10;

/// Shell prompt string
const PROMPT: &str = "lobster> ";

/// Maximum command line length
const MAX_CMD_LEN: usize = 256;

/// Maximum number of arguments per command
const MAX_ARGS: usize = 16;

/// Command handler result
enum CmdResult {
    Ok,
    Exit,    // Shell should exit
    Reboot,  // System reboot requested
}

/// Process info returned by the scheduler for `ps`
pub struct ProcInfo {
    pub pid: u64,
    pub state: u8,
    pub priority: u8,
    pub name: &'static str,
}

/// Initialize and run the shell (never returns)
pub fn run() -> ! {
    // Enable serial input with UART RX interrupts
    unsafe {
        serial_input::init();
    }

    // Print welcome banner
    print_banner();

    // Main shell loop
    loop {
        // Print prompt
        Uart10::puts(PROMPT);

        // Wait for a complete line
        loop {
            // Check if a line is available
            let mut line_buf = [0u8; MAX_CMD_LEN];
            let len = serial_input::read_line(&mut line_buf);
            if len > 0 {
                // Convert to str
                let cmd_str = core::str::from_utf8(&line_buf[..len]).unwrap_or("");

                // Execute the command
                let result = execute_command(cmd_str.trim());

                match result {
                    CmdResult::Exit => {
                        Uart10::puts("Shell exiting.\r\n");
                        break;
                    }
                    CmdResult::Reboot => {
                        Uart10::puts("Rebooting...\r\n");
                        crate::drivers::power::reset();
                    }
                    CmdResult::Ok => {}
                }
                break;
            }

            // No line yet — yield CPU or WFE
            crate::arch::cpu::wfe();
        }
    }
}

/// Print the welcome banner
fn print_banner() {
    Uart10::puts("\r\n");
    Uart10::puts("  +======================================+\r\n");
    Uart10::puts("  |       Lobster OS v0.5.0              |\r\n");
    Uart10::puts("  |   AArch64 Bare-Metal Operating System |\r\n");
    Uart10::puts("  |        Raspberry Pi 5 / BCM2712       |\r\n");
    Uart10::puts("  +======================================+\r\n");
    Uart10::puts("\r\n");
    Uart10::puts("Type 'help' for a list of commands.\r\n\r\n");
}

/// Parse and execute a command string
fn execute_command(line: &str) -> CmdResult {
    if line.is_empty() {
        return CmdResult::Ok;
    }

    // Split into command name and arguments
    let mut args: [&str; MAX_ARGS] = [""; MAX_ARGS];
    let mut argc = 0;

    for part in line.split_whitespace() {
        if argc >= MAX_ARGS {
            break;
        }
        args[argc] = part;
        argc += 1;
    }

    if argc == 0 {
        return CmdResult::Ok;
    }

    let cmd = args[0];

    match cmd {
        "help" | "?" => cmd_help(),
        "clear" => cmd_clear(),
        "uptime" => cmd_uptime(),
        "mem" | "memory" => cmd_mem(),
        "ps" | "procs" => cmd_ps(),
        "kill" => cmd_kill(args, argc),
        "echo" => cmd_echo(args, argc),
        "reboot" => return CmdResult::Reboot,
        "halt" | "shutdown" => cmd_halt(),
        "version" | "ver" => cmd_version(),
        "cpu" => cmd_cpu(),
        "timer" => cmd_timer(),
        "panic" => cmd_panic(),
        "yield" => cmd_yield(),
        "test" => cmd_test(args, argc),
        "hexdump" => cmd_hexdump(args, argc),
        "gpio" => cmd_gpio(args, argc),
            "ifconfig" => cmd_ifconfig(),
            "ping" => cmd_ping(args, argc),
            "arp" => cmd_arp(args, argc),
            "dhcp" => cmd_dhcp(args, argc),
            "net" => cmd_net(args, argc),
            "wm" => cmd_wm(args, argc),
            _ => {
            Uart10::puts("Unknown command: ");
            Uart10::puts(cmd);
            Uart10::puts("\r\nType 'help' for available commands.\r\n");
        }
    }

    CmdResult::Ok
}

/// === Built-in commands ===

fn cmd_help() {
    Uart10::puts("Lobster OS Shell Commands:\r\n");
    Uart10::puts("-------------------------------\r\n");
    Uart10::puts("  help / ?        Show this help message\r\n");
    Uart10::puts("  clear           Clear the screen\r\n");
    Uart10::puts("  uptime          Show system uptime\r\n");
    Uart10::puts("  mem / memory    Show memory usage\r\n");
    Uart10::puts("  ps / procs      List running processes\r\n");
    Uart10::puts("  kill <pid>      Kill a process by PID\r\n");
    Uart10::puts("  echo <text>     Print text to console\r\n");
    Uart10::puts("  version / ver   Show OS version\r\n");
    Uart10::puts("  cpu             Show CPU info\r\n");
    Uart10::puts("  timer           Show timer info\r\n");
    Uart10::puts(" gpio <pin> Read/set GPIO pin state\r\n");
       Uart10::puts(" ifconfig Show network interface config\r\n");
       Uart10::puts(" ping <ip> Send ICMP echo request\r\n");
       Uart10::puts(" arp <ip> Resolve IP via ARP\r\n");
       Uart10::puts(" dhcp [renew|release] DHCP client control\r\n");
       Uart10::puts(" net [poll|stats] Network utilities\r\n");
       Uart10::puts(" wm [info|theme] Window manager info\r\n");
       Uart10::puts(" hexdump <addr> Hex dump memory at address\r\n");
    Uart10::puts("  yield           Yield the current timeslice\r\n");
    Uart10::puts("  test <name>     Run a built-in test\r\n");
    Uart10::puts("  reboot          Reboot the system\r\n");
    Uart10::puts("  halt / shutdown Halt the system\r\n");
    Uart10::puts("  panic           Trigger a kernel panic (TEST!)\r\n");
    Uart10::puts("-------------------------------\r\n");
}

fn cmd_clear() {
    // ANSI escape: clear screen + cursor home
    Uart10::puts("\x1B[2J\x1B[H");
}

fn cmd_uptime() {
    let ms = crate::arch::timer::Timer::uptime_ms();
    let secs = ms / 1000;
    let mins = secs / 60;
    let hrs = mins / 60;
    let days = hrs / 24;

    if days > 0 {
        crate::println!(
            "Uptime: {}d {}h {}m {}s ({}ms total)",
            days, hrs % 24, mins % 60, secs % 60, ms
        );
    } else if hrs > 0 {
        crate::println!(
            "Uptime: {}h {}m {}s ({}ms total)",
            hrs % 24, mins % 60, secs % 60, ms
        );
    } else if mins > 0 {
        crate::println!("Uptime: {}m {}s ({}ms total)", mins % 60, secs % 60, ms);
    } else {
        crate::println!("Uptime: {}.{:03}s", secs, ms % 1000);
    }
}

fn cmd_mem() {
    let (total_frames, used_frames, free_frames) = crate::memory::frame_alloc::stats();
    let page_size = 4096u64;
    let total_kb = (total_frames as u64 * page_size) / 1024;
    let used_kb = (used_frames as u64 * page_size) / 1024;
    let free_kb = (free_frames as u64 * page_size) / 1024;

    crate::println!("Memory Statistics:");
    crate::println!(
        "  Total: {} frames ({} KB / {} MB)",
        total_frames, total_kb, total_kb / 1024
    );
    crate::println!("  Used:  {} frames ({} KB)", used_frames, used_kb);
    crate::println!("  Free:  {} frames ({} KB)", free_frames, free_kb);
}

fn cmd_ps() {
    // Use the scheduler's built-in print_status which dumps all tasks
    crate::sched::print_status();
}

fn cmd_kill(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: kill <pid>\r\n");
        return;
    }
    match parse_u64(args[1]) {
        Some(pid) => {
            crate::sched::kill(pid);
            crate::println!("Killed PID {}", pid);
        }
        None => Uart10::puts("Invalid PID\r\n"),
    }
}

fn cmd_echo(args: [&str; MAX_ARGS], argc: usize) {
    for i in 1..argc {
        if i > 1 {
            Uart10::putc(b' ');
        }
        Uart10::puts(args[i]);
    }
    Uart10::puts("\r\n");
}

fn cmd_version() {
    Uart10::puts("Lobster OS v0.5.0\r\n");
    Uart10::puts("Target: AArch64 (BCM2712 / Raspberry Pi 5)\r\n");
    Uart10::puts("Kernel: 0x80000000\r\n");
    Uart10::puts("Scheduler: Round-robin preemptive\r\n");
}

fn cmd_cpu() {
    let midr: u64;
    unsafe {
        core::arch::asm!("mrs {}, MIDR_EL1", out(reg) midr);
    }

    let implementer = (midr >> 24) & 0xFF;
    let variant = (midr >> 20) & 0xF;
    let architecture = (midr >> 16) & 0xF;
    let part_num = (midr >> 4) & 0xFFF;
    let revision = midr & 0xF;

    let impl_name = match implementer {
        0x41 => "ARM",
        0x42 => "Broadcom",
        0x51 => "Qualcomm",
        _ => "Unknown",
    };

    crate::println!(
        "CPU: {} (implementer={:#x}, variant={}, arch={}, part={:#x}, rev={})",
        impl_name, implementer, variant, architecture, part_num, revision
    );

    // Read current EL
    let current_el: u64;
    unsafe {
        core::arch::asm!("mrs {}, CurrentEL", out(reg) current_el);
    }
    let el = (current_el >> 2) & 0x3;
    crate::println!("Current EL: EL{}", el);

    // Read core ID
    let mpidr: u64;
    unsafe {
        core::arch::asm!("mrs {}, MPIDR_EL1", out(reg) mpidr);
    }
    let core_id = mpidr & 0xFF;
    crate::println!("Core ID: {} (MPIDR={:#x})", core_id, mpidr);
}

fn cmd_timer() {
    let freq = crate::arch::timer::Timer::frequency();
    let uptime = crate::arch::timer::Timer::uptime_ms();
    let sys_time = crate::arch::timer::Timer::system_time_us();

    crate::println!("Timer frequency: {} Hz", freq);
    crate::println!("System time: {} us ({} ms)", sys_time, sys_time / 1000);
    crate::println!("Uptime: {} ms", uptime);
}

fn cmd_halt() {
    Uart10::puts("System halting...\r\n");
    crate::drivers::power::shutdown();
}

fn cmd_panic() {
    Uart10::puts("Triggering test panic...\r\n");
    panic!("User-initiated test panic from shell");
}

fn cmd_yield() {
    crate::sched::yield_task();
    Uart10::puts("Yielded.\r\n");
}

fn cmd_test(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Available tests:\r\n");
        Uart10::puts("  alloc    - allocate and free frames\r\n");
        Uart10::puts("  sched    - create test tasks\r\n");
        Uart10::puts("  map      - test virtual memory mapping\r\n");
        Uart10::puts("  uart     - UART TX test\r\n");
        Uart10::puts("  syscall  - test syscall path\r\n");
        return;
    }

    match args[1] {
        "alloc" => test_alloc(),
        "sched" => test_sched(),
        "map" => test_map(),
        "uart" => test_uart(),
        "syscall" => test_syscall(),
        _ => {
            Uart10::puts("Unknown test: ");
            Uart10::puts(args[1]);
            Uart10::puts("\r\n");
        }
    }
}

fn cmd_hexdump(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: hexdump <address> [length]\r\n");
        return;
    }

    let addr = match parse_u64(args[1]) {
        Some(a) => a,
        None => {
            Uart10::puts("Invalid address\r\n");
            return;
        }
    };

    let len = if argc >= 3 {
        parse_u64(args[2]).unwrap_or(64) as usize
    } else {
        64
    };

    // Align to 16-byte boundary
    let start = addr & !0xF;
    let end = start + len as u64;

    Uart10::puts("Address         00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F  ASCII\r\n");

    let mut row = start;
    while row < end {
        crate::print!("{:#018x}  ", row);
        let mut ascii = [b'.'; 16];
        for col in 0..16usize {
            let ptr = (row + col as u64) as *const u8;
            let byte = unsafe { ptr.read_volatile() };
            crate::print!("{:02x} ", byte);
            if col == 7 {
                Uart10::putc(b' ');
            }
            if byte >= 0x20 && byte <= 0x7E {
                ascii[col] = byte;
            }
        }
        Uart10::puts("  ");
        for &b in &ascii {
            Uart10::putc(b);
        }
        Uart10::puts("\r\n");
        row += 16;
    }
}

fn cmd_gpio(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: gpio <pin> [high|low]\r\n");
        return;
    }

    let pin = match parse_u64(args[1]) {
        Some(p) if p <= 53 => p as u8,
        _ => {
            Uart10::puts("Pin must be 0-53\r\n");
            return;
        }
    };

    if argc >= 3 {
        match args[2] {
            "high" => {
                crate::drivers::gpio::set_pin_mode(pin, crate::drivers::gpio::PinMode::Output);
                crate::drivers::gpio::set_pin(pin);
                crate::println!("GPIO {} set HIGH", pin);
            }
            "low" => {
                crate::drivers::gpio::set_pin_mode(pin, crate::drivers::gpio::PinMode::Output);
                crate::drivers::gpio::clear_pin(pin);
                crate::println!("GPIO {} set LOW", pin);
            }
            _ => Uart10::puts("Use 'high' or 'low'\r\n"),
        }
    } else {
        let level = crate::drivers::gpio::read_pin(pin);
        crate::println!("GPIO {} = {}", pin, if level { "HIGH" } else { "LOW" });
    }
}

/// === Test functions ===

fn test_alloc() {
    Uart10::puts("[test] Frame allocator test...\r\n");

    // Allocate a frame
    let frame = crate::memory::frame_alloc::allocate_frame();
    match frame {
        Some(addr) => crate::println!("[test] Allocated frame at {:#x}", addr.0),
        None => Uart10::puts("[test] FAILED: no frames available\r\n"),
    }

    // Allocate another
    let frame2 = crate::memory::frame_alloc::allocate_frame();
    match frame2 {
        Some(addr) => crate::println!("[test] Allocated frame at {:#x}", addr.0),
        None => Uart10::puts("[test] FAILED: no frames available\r\n"),
    }

    // Free the first
    if let Some(addr) = frame {
        crate::memory::frame_alloc::deallocate_frame(addr);
        crate::println!("[test] Freed frame at {:#x}", addr.0);
    }

    Uart10::puts("[test] Frame allocator test complete.\r\n");
}

fn test_sched() {
    Uart10::puts("[test] Creating test tasks...\r\n");

    fn test_task_a() -> ! {
        for i in 0..3 {
            crate::println!("[task-a] iteration {}", i);
            crate::sched::yield_task();
        }
        crate::println!("[task-a] done");
        crate::sched::exit_current(0);
        loop {
            crate::arch::cpu::wfe();
        }
    }

    fn test_task_b() -> ! {
        for i in 0..3 {
            crate::println!("[task-b] iteration {}", i);
            crate::sched::yield_task();
        }
        crate::println!("[task-b] done");
        crate::sched::exit_current(0);
        loop {
            crate::arch::cpu::wfe();
        }
    }

    let pid_a = crate::sched::create_kernel_task(test_task_a, crate::sched::Priority::Normal);
    let pid_b = crate::sched::create_kernel_task(test_task_b, crate::sched::Priority::Normal);

    match pid_a {
        Some(p) => crate::println!("[test] Task A created (PID {})", p),
        None => Uart10::puts("[test] Failed to create Task A\r\n"),
    }
    match pid_b {
        Some(p) => crate::println!("[test] Task B created (PID {})", p),
        None => Uart10::puts("[test] Failed to create Task B\r\n"),
    }
}

fn test_map() {
    Uart10::puts("[test] Virtual memory mapping test...\r\n");
    let (total, used, free) = crate::memory::frame_alloc::stats();
    crate::println!("[test] Frames: total={}, used={}, free={}", total, used, free);
    Uart10::puts("[test] VM map test complete.\r\n");
}

fn test_uart() {
    Uart10::puts("[test] UART loopback test...\r\n");
    Uart10::puts("[test] Sending bytes to UART TX...\r\n");
    Uart10::puts("Hello from Lobster OS UART!\r\n");
    Uart10::puts("[test] UART TX test complete.\r\n");
}

fn test_syscall() {
    Uart10::puts("[test] Syscall path test...\r\n");
    let args = [0u64; 6];
    let ret = unsafe { crate::sched::syscall::dispatch(0, &args) };
    crate::println!("[test] Syscall 0 (sys_exit) returned: {}", ret);
    Uart10::puts("[test] Syscall test complete.\r\n");
}

/// === Utility functions ===

/// Parse a hex or decimal u64 from a string
fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }

    if s.starts_with("0x") || s.starts_with("0X") {
        // Hex
        let hex_str = &s[2..];
        let mut result: u64 = 0;
        for c in hex_str.chars() {
            result = result.checked_mul(16)?;
            match c {
                '0'..='9' => result += (c as u64 - '0' as u64),
                'a'..='f' => result += (c as u64 - 'a' as u64 + 10),
                'A'..='F' => result += (c as u64 - 'A' as u64 + 10),
                _ => return None,
            }
        }
        Some(result)
    } else {
        // Decimal
        let mut result: u64 = 0;
        for c in s.chars() {
            result = result.checked_mul(10)?;
            match c {
                '0'..='9' => result += (c as u64 - '0' as u64),
                _ => return None,
            }
        }
        Some(result)
    }
}

/// === Network commands ===

/// Parse an IPv4 address string (e.g. "192.168.1.1")
fn parse_ip_addr(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 { return None; }
        let mut val: u8 = 0;
        let mut any = false;
        for c in part.chars() {
            match c {
                '0'..='9' => {
                    val = val.checked_mul(10)?.checked_add(c as u8 - '0' as u8)?;
                    any = true;
                }
                _ => return None,
            }
        }
        if !any { return None; }
        octets[idx] = val;
        idx += 1;
    }
    if idx == 4 { Some(octets) } else { None }
}

fn cmd_ifconfig() {
    let cfg = crate::net::config();
    crate::println!("eth0: LobsterOS Network Interface");
    crate::println!("  MAC:   {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        cfg.mac_addr[0], cfg.mac_addr[1], cfg.mac_addr[2],
        cfg.mac_addr[3], cfg.mac_addr[4], cfg.mac_addr[5]);
    crate::println!("  IP:    {}.{}.{}.{}",
        cfg.ip_addr[0], cfg.ip_addr[1], cfg.ip_addr[2], cfg.ip_addr[3]);
    crate::println!("  Mask:  {}.{}.{}.{}",
        cfg.netmask[0], cfg.netmask[1], cfg.netmask[2], cfg.netmask[3]);
    crate::println!("  GW:    {}.{}.{}.{}",
        cfg.gateway[0], cfg.gateway[1], cfg.gateway[2], cfg.gateway[3]);
    crate::println!("  DNS:   {}.{}.{}.{}",
        cfg.dns_server[0], cfg.dns_server[1], cfg.dns_server[2], cfg.dns_server[3]);
    crate::println!("  DHCP:  {}", if cfg.dhcp_enabled { "enabled" } else { "disabled" });
}

fn cmd_ping(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: ping <ip>\r\n");
        return;
    }
    let dst = match parse_ip_addr(args[1]) {
        Some(ip) => ip,
        None => {
            Uart10::puts("Invalid IP address (use x.x.x.x format)\r\n");
            return;
        }
    };
    crate::println!("PING {}.{}.{}.{}: sending ICMP echo request...", dst[0], dst[1], dst[2], dst[3]);
    crate::net::icmp::send_echo_request(dst, 1);
    // Poll for replies for a few iterations
    for _ in 0..100 {
        crate::net::poll();
    }
    crate::println!("PING done.");
}

fn cmd_arp(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: arp <ip> | arp dump\r\n");
        return;
    }
    if args[1] == "dump" {
        crate::println!("ARP cache:");
        crate::net::arp::dump_cache();
        return;
    }
    let ip = match parse_ip_addr(args[1]) {
        Some(addr) => addr,
        None => {
            Uart10::puts("Invalid IP address\r\n");
            return;
        }
    };
    crate::println!("Resolving {}.{}.{}.{} via ARP...", ip[0], ip[1], ip[2], ip[3]);
    match crate::net::arp::resolve(ip) {
        Some(mac) => crate::println!("  -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]),
        None => crate::println!("  -> not in cache (request sent)"),
    }
}

fn cmd_dhcp(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 || args[1] == "status" {
        let cfg = crate::net::config();
        crate::println!("DHCP status: {}", if cfg.dhcp_enabled { "enabled" } else { "disabled" });
        crate::println!("  IP: {}.{}.{}.{}", cfg.ip_addr[0], cfg.ip_addr[1], cfg.ip_addr[2], cfg.ip_addr[3]);
        return;
    }
    match args[1] {
        "renew" => {
            Uart10::puts("DHCP: sending discover...\r\n");
            if crate::net::dhcp::discover() {
                let cfg = crate::net::config();
                crate::println!("DHCP: leased {}.{}.{}.{}", cfg.ip_addr[0], cfg.ip_addr[1], cfg.ip_addr[2], cfg.ip_addr[3]);
            } else {
                Uart10::puts("DHCP: no response\r\n");
            }
        }
        "release" => {
            crate::net::set_ip_addr([0, 0, 0, 0]);
            crate::net::set_gateway([0, 0, 0, 0]);
            Uart10::puts("DHCP: lease released\r\n");
        }
        _ => Uart10::puts("Usage: dhcp [renew|release|status]\r\n"),
    }
}

fn cmd_net(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: net <poll|stats>\r\n");
        return;
    }
    match args[1] {
        "poll" => {
            crate::println!("Polling network (1 packet)...");
            crate::net::poll();
            Uart10::puts("Done.\r\n");
        }
        "stats" => {
            let cfg = crate::net::config();
            crate::println!("Network stats:");
            crate::println!("  MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                cfg.mac_addr[0], cfg.mac_addr[1], cfg.mac_addr[2],
                cfg.mac_addr[3], cfg.mac_addr[4], cfg.mac_addr[5]);
            crate::println!("  IP:  {}.{}.{}.{}", cfg.ip_addr[0], cfg.ip_addr[1], cfg.ip_addr[2], cfg.ip_addr[3]);
            crate::net::arp::dump_cache();
        }
        _ => Uart10::puts("Usage: net <poll|stats>\r\n"),
    }
}

fn cmd_wm(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: wm <info|theme|composite>\r\n");
        return;
    }
    match args[1] {
        "info" => {
            let info = crate::wm::compositor::fb_info();
            crate::println!("Window Manager:");
            crate::println!("  Resolution: {}x{}", info.width, info.height);
            crate::println!("  BPP: {}", info.bpp);
            crate::println!("  Pitch: {}", info.pitch);
            crate::println!("  Windows: {}", crate::wm::window::count());
        }
        "theme" => {
            let name = crate::wm::theme::current_name();
            crate::println!("Current theme: {}", name);
        }
        "composite" => {
            crate::wm::compositor::composite();
            Uart10::puts("Composited.\r\n");
        }
        _ => Uart10::puts("Usage: wm <info|theme|composite>\r\n"),
    }
}
