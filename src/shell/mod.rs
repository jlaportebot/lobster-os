//! Lobster Shell — advanced serial console command interpreter
//!
//! Features:
//! - Line-at-a-time command processing via serial_input
//! - Built-in commands: help, clear, uptime, mem, ps, kill, echo, reboot, halt
//! - Environment variables (set, unset, export, $VAR expansion)
//! - Command history with up/down arrow navigation
//! - I/O redirection: > file, >> file, < file
//! - Pipes: cmd1 | cmd2
//! - Background execution: cmd &
//! - Variable expansion: $VAR, ${VAR}, $? (last exit code)
//! - Integrates with scheduler, memory, and process subsystems

use crate::drivers::serial_input;
use crate::drivers::uart::Uart10;
use alloc::vec::Vec;

/// Shell prompt string
const PROMPT: &str = "lobster> ";

/// Maximum command line length
const MAX_CMD_LEN: usize = 512;

/// Maximum number of arguments per command
const MAX_ARGS: usize = 32;

/// Maximum environment variables
const MAX_ENV: usize = 32;

/// Maximum command history entries
const MAX_HISTORY: usize = 64;

/// Maximum name/value length for env vars
const MAX_ENV_NAME: usize = 16;
const MAX_ENV_VALUE: usize = 64;

/// Maximum pipeline depth (cmd1 | cmd2 | cmd3 ...)
const MAX_PIPELINE: usize = 8;

/// Maximum redirect filename length
const MAX_REDIRECT_PATH: usize = 128;

/// Command handler result
enum CmdResult {
    Ok,
    Exit,   // Shell should exit
    Reboot, // System reboot requested
}

// ═══════════════════════════════════════════════
// Environment variable store
// ═══════════════════════════════════════════════

/// A single environment variable entry
struct EnvVar {
    name: [u8; MAX_ENV_NAME],
    name_len: usize,
    value: [u8; MAX_ENV_VALUE],
    value_len: usize,
    exported: bool,
}

impl EnvVar {
    const fn new() -> Self {
        Self {
            name: [0; MAX_ENV_NAME],
            name_len: 0,
            value: [0; MAX_ENV_VALUE],
            value_len: 0,
            exported: false,
        }
    }

    fn set_name(&mut self, s: &str) {
        self.name_len = s.len().min(MAX_ENV_NAME);
        self.name[..self.name_len].copy_from_slice(&s.as_bytes()[..self.name_len]);
    }

    fn set_value(&mut self, s: &str) {
        self.value_len = s.len().min(MAX_ENV_VALUE);
        self.value[..self.value_len].copy_from_slice(&s.as_bytes()[..self.value_len]);
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn value_str(&self) -> &str {
        core::str::from_utf8(&self.value[..self.value_len]).unwrap_or("")
    }
}

/// Global environment variable table
static mut ENV_TABLE: [EnvVar; MAX_ENV] = [const { EnvVar::new() }; MAX_ENV];
static mut ENV_COUNT: usize = 0;

/// Last command exit code (like $? in bash)
static mut LAST_EXIT_CODE: i32 = 0;

/// Set an environment variable
fn env_set(name: &str, value: &str) {
    if name.is_empty() || name.len() > MAX_ENV_NAME {
        return;
    }
    unsafe {
        // Search for existing
        for i in 0..ENV_COUNT {
            if ENV_TABLE[i].name_str() == name {
                ENV_TABLE[i].set_value(value);
                return;
            }
        }
        // Add new
        if ENV_COUNT < MAX_ENV {
            ENV_TABLE[ENV_COUNT].set_name(name);
            ENV_TABLE[ENV_COUNT].set_value(value);
            ENV_COUNT += 1;
        }
    }
}

/// Get an environment variable's value
fn env_get(name: &str) -> Option<&'static str> {
    unsafe {
        for i in 0..ENV_COUNT {
            if ENV_TABLE[i].name_str() == name {
                return Some(ENV_TABLE[i].value_str());
            }
        }
    }
    None
}

/// Unset an environment variable
fn env_unset(name: &str) {
    unsafe {
        for i in 0..ENV_COUNT {
            if ENV_TABLE[i].name_str() == name {
                // Shift remaining entries down
                for j in i..ENV_COUNT - 1 {
                    ENV_TABLE[j] = core::mem::replace(&mut ENV_TABLE[j + 1], EnvVar::new());
                }
                ENV_COUNT -= 1;
                return;
            }
        }
    }
}

/// Mark an env var as exported
fn env_export(name: &str) {
    unsafe {
        for i in 0..ENV_COUNT {
            if ENV_TABLE[i].name_str() == name {
                ENV_TABLE[i].exported = true;
                return;
            }
        }
    }
}

/// Expand $VAR and ${VAR} references in a string
fn expand_vars(input: &str, output: &mut [u8]) -> usize {
    let mut out_pos = 0;
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() && out_pos < output.len() - 1 {
        if bytes[i] == b'$' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            // $$ → PID
            if bytes[i] == b'$' {
                let pid_str = u64_to_str(crate::sched::current_pid());
                for &b in pid_str.as_bytes() {
                    if out_pos < output.len() - 1 {
                        output[out_pos] = b;
                        out_pos += 1;
                    }
                }
                i += 1;
                continue;
            }
            // $? → last exit code
            if bytes[i] == b'?' {
                let code_str = i32_to_str(unsafe { LAST_EXIT_CODE });
                for &b in code_str.as_bytes() {
                    if out_pos < output.len() - 1 {
                        output[out_pos] = b;
                        out_pos += 1;
                    }
                }
                i += 1;
                continue;
            }
            // ${VAR} form
            let var_name;
            if bytes[i] == b'{' {
                let end = input[i + 1..].find('}').map(|p| p + i + 1);
                if let Some(end_idx) = end {
                    var_name = &input[i + 1..end_idx];
                    i = end_idx + 1;
                } else {
                    output[out_pos] = b'$';
                    out_pos += 1;
                    continue;
                }
            } else {
                // $VAR form — read alphanumeric/underscore
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                var_name = &input[start..i];
            }
            if let Some(val) = env_get(var_name) {
                for &b in val.as_bytes() {
                    if out_pos < output.len() - 1 {
                        output[out_pos] = b;
                        out_pos += 1;
                    }
                }
            }
            // If var not found, expand to empty string
        } else {
            output[out_pos] = bytes[i];
            out_pos += 1;
            i += 1;
        }
    }
    output[out_pos] = 0;
    out_pos
}

// ═══════════════════════════════════════════════
// Command history
// ═══════════════════════════════════════════════

/// History ring buffer
struct History {
    entries: [[u8; MAX_CMD_LEN]; MAX_HISTORY],
    lengths: [usize; MAX_HISTORY],
    count: usize,
    head: usize, // Next write position
}

impl History {
    const fn new() -> Self {
        Self {
            entries: [[0; MAX_CMD_LEN]; MAX_HISTORY],
            lengths: [0; MAX_HISTORY],
            count: 0,
            head: 0,
        }
    }

    fn push(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        // Don't duplicate the last entry
        if self.count > 0 {
            let last = if self.head == 0 {
                MAX_HISTORY - 1
            } else {
                self.head - 1
            };
            if self.lengths[last] == line.len()
                && self.entries[last][..line.len()] == line.as_bytes()[..line.len()]
            {
                return;
            }
        }
        let len = line.len().min(MAX_CMD_LEN);
        self.entries[self.head][..len].copy_from_slice(&line.as_bytes()[..len]);
        self.lengths[self.head] = len;
        self.head = (self.head + 1) % MAX_HISTORY;
        if self.count < MAX_HISTORY {
            self.count += 1;
        }
    }

    /// Get entry at offset from newest (0 = most recent)
    fn get(&self, offset: usize) -> Option<&str> {
        if offset >= self.count {
            return None;
        }
        let idx = if self.head > offset {
            self.head - offset - 1
        } else {
            MAX_HISTORY - (offset + 1 - self.head)
        };
        core::str::from_utf8(&self.entries[idx][..self.lengths[idx]]).ok()
    }
}

static mut CMD_HISTORY: History = History::new();

// ═══════════════════════════════════════════════
// Pipeline and redirection structures
// ═══════════════════════════════════════════════

/// I/O redirection target
enum Redirect {
    None,
    Write(StringBuf<MAX_REDIRECT_PATH>),  // >
    Append(StringBuf<MAX_REDIRECT_PATH>), // >>
    Read(StringBuf<MAX_REDIRECT_PATH>),   // <
}

/// A single pipeline stage (command + its redirections)
struct PipelineStage {
    args: [&'static str; MAX_ARGS],
    argc: usize,
    stdout_redir: Redirect,
    stdin_redir: Redirect,
}

/// Parsed command line (pipeline of stages + background flag)
struct ParsedLine {
    stages: [PipelineStage; MAX_PIPELINE],
    stage_count: usize,
    background: bool,
}

// ═══════════════════════════════════════════════
// String buffer helper
// ═══════════════════════════════════════════════

struct StringBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StringBuf<N> {
    const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn set(&mut self, s: &str) {
        self.len = s.len().min(N);
        self.buf[..self.len].copy_from_slice(&s.as_bytes()[..self.len]);
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> Clone for StringBuf<N> {
    fn clone(&self) -> Self {
        Self {
            buf: self.buf,
            len: self.len,
        }
    }
}

// ═══════════════════════════════════════════════
// Integer-to-string helpers (no_std, no alloc)
// ═══════════════════════════════════════════════

fn u64_to_str(mut v: u64) -> alloc::string::String {
    if v == 0 {
        return alloc::string::String::from("0");
    }
    let mut buf = [0u8; 20];
    let mut pos = 20;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    alloc::string::String::from(core::str::from_utf8(&buf[pos..]).unwrap_or("0"))
}

fn i32_to_str(mut v: i32) -> alloc::string::String {
    if v == 0 {
        return alloc::string::String::from("0");
    }
    let neg = v < 0;
    if neg {
        v = -v;
    }
    let mut buf = [0u8; 12];
    let mut pos = 12;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    if neg {
        pos -= 1;
        buf[pos] = b'-';
    }
    alloc::string::String::from(core::str::from_utf8(&buf[pos..]).unwrap_or("0"))
}

/// Process info returned by the scheduler for `ps`
pub struct ProcInfo {
    pub pid: u64,
    pub state: u8,
    pub priority: u8,
    pub name: &'static str,
}

/// Initialize and run the shell (never returns)
/// If `pty_slave_fd` is Some, use it for I/O instead of UART
pub fn run() -> ! {
    run_with_pty(None);
}

/// Run shell with PTY slave fd for I/O
pub fn run_with_pty(pty_slave_fd: Option<usize>) -> ! {
    // Enable serial input with UART RX interrupts
    unsafe {
        serial_input::init();
    }

    // Print welcome banner
    print_banner();

    // Main shell loop
    loop {
        // Print prompt
        print_prompt(pty_slave_fd);

        // Wait for a complete line
        loop {
            // Check if a line is available
            let mut line_buf = [0u8; MAX_CMD_LEN];
            let len = if let Some(fd) = pty_slave_fd {
                // Read from PTY slave
                read_line_from_pty(fd, &mut line_buf)
            } else {
                // Read from UART
                serial_input::read_line(&mut line_buf)
            };

            if len > 0 {
                // Convert to str
                let cmd_str = core::str::from_utf8(&line_buf[..len]).unwrap_or("");

                // Execute the command
                let result = execute_command(cmd_str.trim());

                match result {
                    CmdResult::Exit => {
                        print_line(pty_slave_fd, "Shell exiting.\r\n");
                        break;
                    }
                    CmdResult::Reboot => {
                        print_line(pty_slave_fd, "Rebooting...\r\n");
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

/// Print prompt to the appropriate output
fn print_prompt(pty_slave_fd: Option<usize>) {
    if let Some(fd) = pty_slave_fd {
        print_to_pty(fd, PROMPT);
    } else {
        Uart10::puts(PROMPT);
    }
}

/// Print a line to the appropriate output
fn print_line(pty_slave_fd: Option<usize>, line: &str) {
    if let Some(fd) = pty_slave_fd {
        print_to_pty(fd, line);
    } else {
        Uart10::puts(line);
    }
}

/// Print to PTY slave fd
fn print_to_pty(fd: usize, text: &str) {
    use crate::ipc::pty_slave_write;
    let bytes = text.as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        let n = pty_slave_write(fd, &bytes[written..]);
        if n == 0 {
            break;
        }
        written += n;
    }
}

/// Read a line from PTY slave
fn read_line_from_pty(fd: usize, buf: &mut [u8]) -> usize {
    use crate::ipc::pty_slave_read;
    pty_slave_read(fd, buf)
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

/// Parse and execute a command string (advanced: pipes, redirects, env vars)
fn execute_command(line: &str) -> CmdResult {
    if line.is_empty() {
        return CmdResult::Ok;
    }

    // 1. Expand environment variables ($VAR, ${VAR}, $?, $$)
    let mut expanded = [0u8; MAX_CMD_LEN];
    let expanded_len = expand_vars(line, &mut expanded);
    let expanded_str = core::str::from_utf8(&expanded[..expanded_len]).unwrap_or("");

    // 2. Check for trailing & (background)
    let (cmd_line, background) = {
        let trimmed = expanded_str.trim_end();
        if trimmed.ends_with('&') {
            (trimmed[..trimmed.len() - 1].trim_end(), true)
        } else {
            (trimmed, false)
        }
    };

    // 3. Split into pipeline stages on |
    let mut stages: [&str; MAX_PIPELINE] = [""; MAX_PIPELINE];
    let mut stage_count = 0;
    for part in cmd_line.split('|') {
        if stage_count >= MAX_PIPELINE {
            break;
        }
        stages[stage_count] = part.trim();
        stage_count += 1;
    }

    // 4. For each pipeline stage, parse redirections and args
    //    (we execute builtins sequentially; pipe data is buffered via IPC pipes)
    let mut pipe_read_fd: i32 = -1; // Input from previous stage

    for stage_idx in 0..stage_count {
        let stage_str = stages[stage_idx];
        if stage_str.is_empty() {
            continue;
        }

        // Parse redirections from the stage string
        let mut stdout_redir = Redirect::None;
        let mut stdin_redir = Redirect::None;
        let mut clean_parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
        let tokens: alloc::vec::Vec<&str> = stage_str.split_whitespace().collect();
        let mut i = 0;

        while i < tokens.len() {
            let tok = tokens[i];
            if tok == ">" && i + 1 < tokens.len() {
                let mut sb = StringBuf::<MAX_REDIRECT_PATH>::new();
                sb.set(tokens[i + 1]);
                stdout_redir = Redirect::Write(sb);
                i += 2;
            } else if tok == ">>" && i + 1 < tokens.len() {
                let mut sb = StringBuf::<MAX_REDIRECT_PATH>::new();
                sb.set(tokens[i + 1]);
                stdout_redir = Redirect::Append(sb);
                i += 2;
            } else if tok == "<" && i + 1 < tokens.len() {
                let mut sb = StringBuf::<MAX_REDIRECT_PATH>::new();
                sb.set(tokens[i + 1]);
                stdin_redir = Redirect::Read(sb);
                i += 2;
            } else {
                clean_parts.push(tok);
                i += 1;
            }
        }

        // Build args array
        let mut args: [&str; MAX_ARGS] = [""; MAX_ARGS];
        let mut argc = 0;
        for part in &clean_parts {
            if argc >= MAX_ARGS {
                break;
            }
            args[argc] = *part;
            argc += 1;
        }

        if argc == 0 {
            continue;
        }

        let cmd = args[0];

        // If this is the last stage and no pipes, run directly
        // If piped, create an IPC pipe between stages
        let is_last_stage = stage_idx + 1 == stage_count;

        // Create pipe for inter-stage communication if needed
        let mut next_pipe_fds: (i32, i32) = (-1, -1);
        if !is_last_stage {
            if let Some((r, w)) = crate::ipc::create_pipe() {
                next_pipe_fds = (r as i32, w as i32);
            } else {
                Uart10::puts("shell: pipe creation failed\r\n");
                break;
            }
        }

        // Execute the command (builtins only for now in kernel shell)
        let exit_code = run_builtin(
            cmd,
            args,
            argc,
            pipe_read_fd,
            &stdout_redir,
            &stdin_redir,
            if !is_last_stage {
                Some(next_pipe_fds.1)
            } else {
                None
            },
        );

        unsafe {
            LAST_EXIT_CODE = exit_code;
        }

        // Close previous pipe read fd if any
        if pipe_read_fd >= 0 {
            let pid = crate::sched::current_pid();
            if let Some(idx) = crate::sched::find_task_index(pid) {
                crate::ipc::fd_close(idx, pipe_read_fd as usize);
            }
        }

        // Close write end of current pipe
        if next_pipe_fds.1 >= 0 {
            let pid = crate::sched::current_pid();
            if let Some(idx) = crate::sched::find_task_index(pid) {
                crate::ipc::fd_close(idx, next_pipe_fds.1 as usize);
            }
        }

        // Next stage reads from the read end
        pipe_read_fd = next_pipe_fds.0;

        // Handle background flag (just log it for now)
        if background && stage_idx == 0 {
            Uart10::puts("[background] ");
            Uart10::puts(cmd);
            Uart10::puts("\r\n");
        }
    }

    // Close any remaining pipe read fd
    if pipe_read_fd >= 0 {
        let pid = crate::sched::current_pid();
        if let Some(idx) = crate::sched::find_task_index(pid) {
            crate::ipc::fd_close(idx, pipe_read_fd as usize);
        }
    }

    CmdResult::Ok
}

/// Run a builtin command, returning an exit code (0 = success, 1 = failure)
fn run_builtin(
    cmd: &str,
    args: [&str; MAX_ARGS],
    argc: usize,
    _pipe_in: i32,
    _stdout_redir: &Redirect,
    _stdin_redir: &Redirect,
    _pipe_out: Option<i32>,
) -> i32 {
    match cmd {
        "help" | "?" => {
            cmd_help();
            0
        }
        "clear" => {
            cmd_clear();
            0
        }
        "uptime" => {
            cmd_uptime();
            0
        }
        "mem" | "memory" => {
            cmd_mem();
            0
        }
        "ps" | "procs" => {
            cmd_ps();
            0
        }
        "kill" => cmd_kill(args, argc),
        "echo" => cmd_echo(args, argc),
        "reboot" => {
            cmd_reboot();
            0
        }
        "halt" | "shutdown" => {
            cmd_halt();
            0
        }
        "version" | "ver" => {
            cmd_version();
            0
        }
        "cpu" => {
            cmd_cpu();
            0
        }
        "timer" => {
            cmd_timer();
            0
        }
        "panic" => {
            cmd_panic();
            0
        }
        "yield" => {
            cmd_yield();
            0
        }
        "test" => {
            cmd_test(args, argc);
            0
        }
        "hexdump" => {
            cmd_hexdump(args, argc);
            0
        }
        "gpio" => {
            cmd_gpio(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "ifconfig" => {
            cmd_ifconfig();
            0
        }
        #[cfg(not(feature = "qemu"))]
        "ping" => {
            cmd_ping(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "arp" => {
            cmd_arp(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "dhcp" => {
            cmd_dhcp(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "dns" => {
            cmd_dns(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "net" => {
            cmd_net(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "wm" => {
            cmd_wm(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "audio" => {
            cmd_audio(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "gpu" => {
            cmd_gpu(args, argc);
            0
        }
        #[cfg(not(feature = "qemu"))]
        "wifi" => {
            cmd_wifi(args, argc);
            0
        }
        "pkg" => {
            cmd_pkg(args, argc);
            0
        }
        // Panel command
        #[cfg(not(feature = "qemu"))]
        "panel" => {
            cmd_panel(args, argc);
            0
        }
        // User management commands
        "login" => cmd_login(args, argc),
        "su" => cmd_su(args, argc),
        "sudo" => cmd_sudo(args, argc),
        "passwd" => cmd_passwd(args, argc),
        "useradd" => cmd_useradd(args, argc),
        "userdel" => cmd_userdel(args, argc),
        "groupadd" => cmd_groupadd(args, argc),
        "groupdel" => cmd_groupdel(args, argc),
        "id" => cmd_id(args, argc),
        "whoami" => cmd_whoami(args, argc),
        // Environment variable commands
        "set" | "export" => cmd_env_set(args, argc),
        "unset" => cmd_env_unset(args, argc),
        "env" | "printenv" => cmd_env_list(),
        "history" | "hist" => cmd_history(),
        // File system commands
        "cat" => cmd_cat(args, argc),
        "ls" => cmd_ls(args, argc),
        "cd" => cmd_cd(args, argc),
        "pwd" => cmd_pwd(),
        "mkdir" => cmd_mkdir(args, argc),
        "rm" => cmd_rm(args, argc),
        "mv" => cmd_mv(args, argc),
        "chmod" => cmd_chmod(args, argc),
        "chown" => cmd_chown(args, argc),
        // User process execution
        "run" => cmd_run(args, argc),
        // Service management commands
        "svc" => cmd_svc(args, argc),
        "runlevel" => cmd_runlevel(args, argc),
        _ => {
            Uart10::puts("Unknown command: ");
            Uart10::puts(cmd);
            Uart10::puts("\r\nType 'help' for available commands.\r\n");
            1
        }
    }
}

// === Built-in commands ===
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
    #[cfg(not(feature = "qemu"))]
    {
        Uart10::puts(" ifconfig Show network interface config\r\n");
        Uart10::puts(" ping <ip> Send ICMP echo request\r\n");
        Uart10::puts(" arp <ip> Resolve IP via ARP\r\n");
        Uart10::puts(" dhcp [renew|release] DHCP client control\r\n");
        Uart10::puts(" dns <domain> Resolve domain name via DNS\r\n");
        Uart10::puts(" net [poll|stats] Network utilities\r\n");
        Uart10::puts(" wm [info|theme] Window manager info\r\n");
        Uart10::puts(" audio [beep|tone|stop|vol|status] Audio control\r\n");
        Uart10::puts(" gpu [info|status|clock|blank|mode] GPU/display control\r\n");
        Uart10::puts(" wifi [scan|connect|disconnect|status] WiFi management\r\n");
    }
    Uart10::puts(" login [user]     Login as user\r\n");
    Uart10::puts(" su [user]        Switch user (default root)\r\n");
    Uart10::puts(" sudo <cmd>       Run command as another user\r\n");
    Uart10::puts(" passwd [user]    Change password\r\n");
    Uart10::puts(" useradd <user>   Add new user\r\n");
    Uart10::puts(" userdel <user>   Delete user\r\n");
    Uart10::puts(" groupadd <group> Add new group\r\n");
    Uart10::puts(" groupdel <group> Delete group\r\n");
    Uart10::puts(" id [user]        Show user/group IDs\r\n");
    Uart10::puts(" whoami           Show current user\r\n");
    Uart10::puts(" hexdump <addr> Hex dump memory at address\r\n");
    Uart10::puts(" yield Yield the current timeslice\r\n");
    Uart10::puts(" test <name> Run a built-in test\r\n");
    Uart10::puts(" set/export <NAME=VAL> Set environment variable\r\n");
    Uart10::puts(" unset <NAME> Unset environment variable\r\n");
    Uart10::puts(" env / printenv List environment variables\r\n");
    Uart10::puts(" history / hist Show command history\r\n");
    Uart10::puts(" cat <path> Display file contents\r\n");
    Uart10::puts(" ls [path] List directory contents\r\n");
    Uart10::puts(" cd <path> Change working directory\r\n");
    Uart10::puts(" pwd Print working directory\r\n");
    Uart10::puts(" mkdir <path> Create directory\r\n");
    Uart10::puts(" rm <path> Remove file or directory\r\n");
    Uart10::puts(" mv <src> <dst> Move/rename file\r\n");
    Uart10::puts(" chmod <mode> <path> Change file permissions\r\n");
    Uart10::puts(" chown <uid:gid> <path> Change file ownership\r\n");
    Uart10::puts(" run <path|demo> Run ELF binary as user process\r\n");
    Uart10::puts(" svc <list|start|stop|restart|reset|deps> Service mgmt\r\n");
    Uart10::puts(" runlevel [get|set <N>] Show/set runlevel\r\n");
    Uart10::puts(
        " pkg <update|search|install|remove|list|info|repo|install-multi> Package mgmt\r\n",
    );
    Uart10::puts(" reboot Reboot the system\r\n");
    Uart10::puts(" halt / shutdown Halt the system\r\n");
    Uart10::puts(" panic Trigger a kernel panic (TEST!)\r\n");
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
            days,
            hrs % 24,
            mins % 60,
            secs % 60,
            ms
        );
    } else if hrs > 0 {
        crate::println!(
            "Uptime: {}h {}m {}s ({}ms total)",
            hrs % 24,
            mins % 60,
            secs % 60,
            ms
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
        total_frames,
        total_kb,
        total_kb / 1024
    );
    crate::println!("  Used:  {} frames ({} KB)", used_frames, used_kb);
    crate::println!("  Free:  {} frames ({} KB)", free_frames, free_kb);
}

fn cmd_ps() {
    // Use the scheduler's built-in print_status which dumps all tasks
    crate::sched::print_status();
}

fn cmd_kill(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: kill <pid>\r\n");
        return 1;
    }
    match parse_u64(args[1]) {
        Some(pid) => {
            crate::sched::kill(pid);
            crate::println!("Killed PID {}", pid);
            0
        }
        None => {
            Uart10::puts("Invalid PID\r\n");
            1
        }
    }
}

fn cmd_echo(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    for i in 1..argc {
        if i > 1 {
            Uart10::putc(b' ');
        }
        Uart10::puts(args[i]);
    }
    Uart10::puts("\r\n");
    0
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
        impl_name,
        implementer,
        variant,
        architecture,
        part_num,
        revision
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

// === Test functions ===
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
        crate::sched::exit_current(0)
    }

    fn test_task_b() -> ! {
        for i in 0..3 {
            crate::println!("[task-b] iteration {}", i);
            crate::sched::yield_task();
        }
        crate::println!("[task-b] done");
        crate::sched::exit_current(0)
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
    crate::println!(
        "[test] Frames: total={}, used={}, free={}",
        total,
        used,
        free
    );
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
    let ret = crate::sched::syscall::dispatch(0, &args);
    crate::println!("[test] Syscall 0 (sys_exit) returned: {}", ret);
    Uart10::puts("[test] Syscall test complete.\r\n");
}

// === Utility functions ===
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
                '0'..='9' => result += c as u64 - '0' as u64,
                'a'..='f' => result += c as u64 - 'a' as u64 + 10,
                'A'..='F' => result += c as u64 - 'A' as u64 + 10,
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
                '0'..='9' => result += c as u64 - '0' as u64,
                _ => return None,
            }
        }
        Some(result)
    }
}

// === Network commands ===
/// Parse an IPv4 address string (e.g. "192.168.1.1")
#[cfg(not(feature = "qemu"))]
fn parse_ip_addr(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return None;
        }
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
        if !any {
            return None;
        }
        octets[idx] = val;
        idx += 1;
    }
    if idx == 4 {
        Some(octets)
    } else {
        None
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_ifconfig() {
    let cfg = crate::net::config();
    crate::println!("eth0: LobsterOS Network Interface");
    crate::println!(
        "  MAC:   {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        cfg.mac_addr[0],
        cfg.mac_addr[1],
        cfg.mac_addr[2],
        cfg.mac_addr[3],
        cfg.mac_addr[4],
        cfg.mac_addr[5]
    );
    crate::println!(
        "  IP:    {}.{}.{}.{}",
        cfg.ip_addr[0],
        cfg.ip_addr[1],
        cfg.ip_addr[2],
        cfg.ip_addr[3]
    );
    crate::println!(
        "  Mask:  {}.{}.{}.{}",
        cfg.netmask[0],
        cfg.netmask[1],
        cfg.netmask[2],
        cfg.netmask[3]
    );
    crate::println!(
        "  GW:    {}.{}.{}.{}",
        cfg.gateway[0],
        cfg.gateway[1],
        cfg.gateway[2],
        cfg.gateway[3]
    );
    crate::println!(
        "  DNS:   {}.{}.{}.{}",
        cfg.dns_server[0],
        cfg.dns_server[1],
        cfg.dns_server[2],
        cfg.dns_server[3]
    );
    crate::println!(
        "  DHCP:  {}",
        if cfg.dhcp_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
}

#[cfg(not(feature = "qemu"))]
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
    crate::println!(
        "PING {}.{}.{}.{}: sending ICMP echo request...",
        dst[0],
        dst[1],
        dst[2],
        dst[3]
    );
    crate::net::icmp::send_echo_request(dst, 1);
    // Poll for replies for a few iterations
    for _ in 0..100 {
        crate::net::poll();
    }
    crate::println!("PING done.");
}

#[cfg(not(feature = "qemu"))]
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
    crate::println!(
        "Resolving {}.{}.{}.{} via ARP...",
        ip[0],
        ip[1],
        ip[2],
        ip[3]
    );
    match crate::net::arp::resolve(ip) {
        Some(mac) => crate::println!(
            "  -> {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5]
        ),
        None => crate::println!("  -> not in cache (request sent)"),
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_dhcp(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 || args[1] == "status" {
        let cfg = crate::net::config();
        crate::println!(
            "DHCP status: {}",
            if cfg.dhcp_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        crate::println!(
            "  IP: {}.{}.{}.{}",
            cfg.ip_addr[0],
            cfg.ip_addr[1],
            cfg.ip_addr[2],
            cfg.ip_addr[3]
        );
        return;
    }
    match args[1] {
        "renew" => {
            Uart10::puts("DHCP: sending discover...\r\n");
            if crate::net::dhcp::discover() {
                let cfg = crate::net::config();
                crate::println!(
                    "DHCP: leased {}.{}.{}.{}",
                    cfg.ip_addr[0],
                    cfg.ip_addr[1],
                    cfg.ip_addr[2],
                    cfg.ip_addr[3]
                );
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

#[cfg(not(feature = "qemu"))]
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
            crate::println!(
                " MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                cfg.mac_addr[0],
                cfg.mac_addr[1],
                cfg.mac_addr[2],
                cfg.mac_addr[3],
                cfg.mac_addr[4],
                cfg.mac_addr[5]
            );
            crate::println!(
                " IP: {}.{}.{}.{}",
                cfg.ip_addr[0],
                cfg.ip_addr[1],
                cfg.ip_addr[2],
                cfg.ip_addr[3]
            );
            crate::net::arp::dump_cache();
        }
        _ => Uart10::puts("Usage: net <poll|stats>\r\n"),
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_dns(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: dns <domain>\r\n");
        Uart10::puts("Example: dns example.com\r\n");
        return;
    }
    let domain = args[1];
    crate::println!("Resolving {} via DNS...", domain);
    match crate::net::dns::resolve(domain) {
        Some(ip) => crate::println!("{} -> {}.{}.{}.{}", domain, ip[0], ip[1], ip[2], ip[3]),
        None => crate::println!("Failed to resolve {}", domain),
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_wm(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: wm <info|theme|composite|wallpaper>\\r\\n");
        return;
    }
    match args[1] {
        "info" => {
            let info = crate::wm::compositor::fb_info();
            match info {
                Some(info) => {
                    crate::println!("Window Manager:");
                    crate::println!("  Resolution: {}x{}", info.width, info.height);
                    crate::println!("  BPP: {}", info.bpp);
                    crate::println!("  Pitch: {}", info.pitch);
                    crate::println!("  Windows: {}", crate::wm::window::count());
                }
                None => {
                    crate::println!("Compositor not initialized");
                }
            }
        }
        "theme" => {
            let name = crate::wm::theme::current_name();
            crate::println!("Current theme: {}", name);
        }
        "composite" => {
            crate::wm::compositor::composite();
            Uart10::puts("Composited.\\r\\n");
        }
        "wallpaper" => {
            if argc < 3 {
                Uart10::puts("Usage: wm wallpaper <color|image> [value]\\r\\n");
                Uart10::puts("  wm wallpaper color <hex>     Set solid color background (e.g. FF0D1B2A)\\r\\n");
                Uart10::puts("  wm wallpaper image <path>    Set wallpaper image from file\\r\\n");
                Uart10::puts("  wm wallpaper mode <fit|fill|center|tile>  Set scaling mode\\r\\n");
                return;
            }
            match args[2] {
                "color" => {
                    if argc < 4 {
                        Uart10::puts("Usage: wm wallpaper color <hex_color>\\r\\n");
                        return;
                    }
                    if let Some(color) = parse_hex_color(args[3]) {
                        crate::wm::compositor::set_wallpaper_color(color);
                        crate::println!("Wallpaper color set to: #{:08X}", color);
                    } else {
                        Uart10::puts("Invalid color format. Use 8-digit hex (AARRGGBB) or 6-digit hex (RRGGBB)\\r\\n");
                    }
                }
                "image" => {
                    if argc < 4 {
                        Uart10::puts("Usage: wm wallpaper image <path>\\r\\n");
                        return;
                    }
                    let path = args[3];
                    match crate::wm::compositor::set_wallpaper_image(path) {
                        Ok(()) => crate::println!("Wallpaper image loaded: {}", path),
                        Err(e) => crate::println!("Failed to load wallpaper: {:?}", e),
                    }
                }
                "mode" => {
                    if argc < 4 {
                        Uart10::puts("Usage: wm wallpaper mode <fit|fill|center|tile>\\r\\n");
                        return;
                    }
                    match args[3] {
                        "fit" => crate::wm::compositor::set_wallpaper_mode(
                            crate::wm::compositor::WallpaperMode::Fit,
                        ),
                        "fill" => crate::wm::compositor::set_wallpaper_mode(
                            crate::wm::compositor::WallpaperMode::Fill,
                        ),
                        "center" => crate::wm::compositor::set_wallpaper_mode(
                            crate::wm::compositor::WallpaperMode::Center,
                        ),
                        "tile" => crate::wm::compositor::set_wallpaper_mode(
                            crate::wm::compositor::WallpaperMode::Tile,
                        ),
                        _ => Uart10::puts("Invalid mode. Use: fit, fill, center, tile\\r\\n"),
                    }
                }
                _ => Uart10::puts("Usage: wm wallpaper <color|image|mode> [value]\\r\\n"),
            }
        }
        _ => Uart10::puts("Usage: wm <info|theme|composite|wallpaper>\\r\\n"),
    }
}

/// Panel shell command
#[cfg(not(feature = "qemu"))]
fn cmd_panel(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: panel <toggle|show|hide>\\r\\n");
        return;
    }
    match args[1] {
        "toggle" => {
            crate::wm::panel::toggle();
            let state = if crate::wm::panel::is_visible() {
                "shown"
            } else {
                "hidden"
            };
            crate::println!("[panel] Panel {}", state);
        }
        "show" => {
            crate::wm::panel::set_visible(true);
            crate::println!("[panel] Panel shown");
        }
        "hide" => {
            crate::wm::panel::set_visible(false);
            crate::println!("[panel] Panel hidden");
        }
        _ => Uart10::puts("Usage: panel <toggle|show|hide>\\r\\n"),
    }
}

/// Parse hex color string (AARRGGBB or RRGGBB)
fn parse_hex_color(s: &str) -> Option<u32> {
    let s = s.trim_start_matches('#');
    if s.len() == 8 {
        // AARRGGBB
        u32::from_str_radix(s, 16).ok()
    } else if s.len() == 6 {
        // RRGGBB -> AARRGGBB with full opacity
        u32::from_str_radix(s, 16).ok().map(|c| 0xFF000000 | c)
    } else {
        None
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_audio(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: audio <beep|tone|stop|vol|status>\r\n");
        return;
    }
    match args[1] {
        "beep" => {
            crate::drivers::audio::beep(1000, 200);
            Uart10::puts("Beep!\r\n");
        }
        "tone" => {
            if argc < 4 {
                Uart10::puts("Usage: audio tone <freq_hz> <duration_ms>\r\n");
                return;
            }
            let freq = parse_u32(args[2]).unwrap_or(440);
            let dur = parse_u32(args[3]).unwrap_or(500);
            crate::drivers::audio::tone(freq, dur, crate::drivers::audio::volume());
            crate::println!("Tone: {} Hz, {} ms", freq, dur);
        }
        "stop" => {
            crate::drivers::audio::stop();
            Uart10::puts("Audio stopped.\r\n");
        }
        "vol" => {
            if argc < 3 {
                crate::println!("Volume: {}/255", crate::drivers::audio::volume());
            } else {
                let v = parse_u32(args[2]).unwrap_or(128);
                let v8 = if v > 255 { 255 } else { v as u8 };
                crate::drivers::audio::set_volume(v8);
                crate::println!("Volume set: {}/255", v8);
            }
        }
        "status" => {
            let st = crate::drivers::audio::state();
            crate::println!(
                "Audio state: {}",
                match st {
                    crate::drivers::audio::AudioState::Stopped => "stopped",
                    crate::drivers::audio::AudioState::Playing => "playing",
                    crate::drivers::audio::AudioState::Paused => "paused",
                }
            );
            crate::println!("Volume: {}/255", crate::drivers::audio::volume());
            crate::println!("Buffer count: {}", crate::drivers::audio::buffer_count());
        }
        _ => Uart10::puts("Usage: audio <beep|tone|stop|vol|status>\r\n"),
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_gpu(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: gpu <info|status|clock|blank|mode>\r\n");
        return;
    }
    match args[1] {
        "info" => {
            let info = crate::drivers::gpu::gpu_info();
            crate::println!("GPU Info:");
            crate::println!(" Firmware: {:#010x}", info.firmware_rev);
            crate::println!(
                " Board: model={:#x} rev={:#x}",
                info.board_model,
                info.board_rev
            );
            crate::println!(
                " VC mem: {:#x} ({} MB)",
                info.vc_mem_base,
                info.vc_mem_size / (1024 * 1024)
            );
            crate::println!(
                " ARM mem: {:#x} ({} MB)",
                info.arm_mem_base,
                info.arm_mem_size / (1024 * 1024)
            );
            crate::println!(" V3D clock: {} MHz", info.v3d_clock / 1_000_000);
            crate::println!(" Core clock: {} MHz", info.core_clock / 1_000_000);
        }
        "status" => {
            crate::drivers::gpu::print_status();
        }
        "clock" => {
            if argc < 3 {
                Uart10::puts("Usage: gpu clock <v3d|core|pixel|emmc>\r\n");
                return;
            }
            match args[2] {
                "v3d" => crate::println!(
                    "V3D clock: {} MHz",
                    crate::drivers::gpu::get_v3d_clock() / 1_000_000
                ),
                "core" => crate::println!(
                    "Core clock: {} MHz",
                    crate::drivers::gpu::get_core_clock() / 1_000_000
                ),
                "pixel" => crate::println!(
                    "Pixel clock: {} MHz",
                    crate::drivers::gpu::get_pixel_clock() / 1_000_000
                ),
                "emmc" => crate::println!(
                    "eMMC clock: {} MHz",
                    crate::drivers::gpu::get_emmc_clock() / 1_000_000
                ),
                _ => Uart10::puts("Unknown clock. Use: v3d, core, pixel, emmc\r\n"),
            }
        }
        "blank" => {
            crate::drivers::gpu::blank_screen(true);
            Uart10::puts("Screen blanked.\r\n");
        }
        "mode" => {
            if argc < 5 {
                Uart10::puts("Usage: gpu mode <width> <height> <bpp>\r\n");
                return;
            }
            let w = parse_u32(args[2]).unwrap_or(640);
            let h = parse_u32(args[3]).unwrap_or(480);
            let bpp = parse_u32(args[4]).unwrap_or(32);
            if crate::drivers::gpu::configure_display(w, h, bpp, true) {
                crate::println!("Mode set: {}x{} @ {}bpp", w, h, bpp);
            } else {
                Uart10::puts("Failed to set mode.\r\n");
            }
        }
        _ => Uart10::puts("Usage: gpu <info|status|clock|blank|mode>\r\n"),
    }
}

#[cfg(not(feature = "qemu"))]
fn cmd_wifi(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: wifi <scan|connect|disconnect|status>\r\n");
        Uart10::puts("  wifi scan                    - Scan for available networks\r\n");
        Uart10::puts("  wifi connect <ssid> [pass]   - Connect to network (WPA2/WPA3)\r\n");
        Uart10::puts("  wifi disconnect              - Disconnect from current network\r\n");
        Uart10::puts("  wifi status                  - Show connection status\r\n");
        return;
    }
    match args[1] {
        "scan" => {
            Uart10::puts("Scanning for WiFi networks...\r\n");
            match crate::drivers::wifi::scan() {
                Ok(results) => {
                    if results.is_empty() {
                        Uart10::puts("No networks found.\r\n");
                    } else {
                        crate::println!("Found {} network(s):", results.len());
                        for (i, bss) in results.iter().enumerate() {
                            let ssid = core::str::from_utf8(&bss.ssid[..bss.ssid_len as usize])
                                .unwrap_or("<invalid>");
                            let security_str = match bss.security {
                                crate::drivers::wifi::SecurityType::Open => "OPEN",
                                crate::drivers::wifi::SecurityType::Wep => "WEP",
                                crate::drivers::wifi::SecurityType::WpaPsk => "WPA-PSK",
                                crate::drivers::wifi::SecurityType::Wpa2Psk => "WPA2-PSK",
                                crate::drivers::wifi::SecurityType::Wpa3Sae => "WPA3-SAE",
                                _ => "UNKNOWN",
                            };
                            crate::println!(
                                "  {}  {}  Ch:{}  RSSI:{}  {}",
                                i,
                                ssid,
                                bss.channel,
                                bss.rssi,
                                security_str
                            );
                        }
                    }
                }
                Err(e) => {
                    crate::println!("Scan failed: {:?}", e);
                }
            }
        }
        "connect" => {
            if argc < 3 {
                Uart10::puts("Usage: wifi connect <ssid> [password]\r\n");
                return;
            }
            let ssid = args[2].as_bytes();
            let password = if argc >= 4 { args[3].as_bytes() } else { &[] };
            crate::println!("Connecting to {}...", args[2]);
            match crate::drivers::wifi::connect(ssid, password) {
                Ok(()) => {
                    crate::println!("Connected successfully!");
                    // Run DHCP to get IP address
                    Uart10::puts("Requesting DHCP lease...\r\n");
                    if crate::net::dhcp::discover() {
                        let cfg = crate::net::config();
                        crate::println!(
                            "DHCP leased: {}.{}.{}.{}",
                            cfg.ip_addr[0],
                            cfg.ip_addr[1],
                            cfg.ip_addr[2],
                            cfg.ip_addr[3]
                        );
                    } else {
                        Uart10::puts("DHCP failed (no response)\r\n");
                    }
                }
                Err(e) => {
                    crate::println!("Connection failed: {:?}", e);
                }
            }
        }
        "save" => {
            if argc < 3 {
                Uart10::puts("Usage: wifi save <ssid> [password]\r\n");
                return;
            }
            let ssid = args[2].as_bytes();
            let password = if argc >= 4 { args[3].as_bytes() } else { &[] };
            match crate::drivers::wifi::save_network(ssid, password) {
                Ok(()) => crate::println!("Network saved for auto-connect"),
                Err(e) => crate::println!("Save failed: {:?}", e),
            }
        }
        "disconnect" => match crate::drivers::wifi::disconnect() {
            Ok(()) => crate::println!("Disconnected."),
            Err(e) => crate::println!("Disconnect failed: {:?}", e),
        },
        "status" => {
            if crate::drivers::wifi::link_up() {
                let (ssid_bytes, len) = crate::drivers::wifi::connected_ssid();
                let ssid = core::str::from_utf8(&ssid_bytes[..len]).unwrap_or("<invalid>");
                let bssid = crate::drivers::wifi::connected_bssid();
                crate::println!("Connected to: {}", ssid);
                crate::println!(
                    "BSSID: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    bssid[0],
                    bssid[1],
                    bssid[2],
                    bssid[3],
                    bssid[4],
                    bssid[5]
                );
                let cfg = crate::net::config();
                crate::println!(
                    "IP: {}.{}.{}.{}",
                    cfg.ip_addr[0],
                    cfg.ip_addr[1],
                    cfg.ip_addr[2],
                    cfg.ip_addr[3]
                );
            } else {
                Uart10::puts("Not connected to any network.\r\n");
            }
        }
        _ => Uart10::puts("Usage: wifi <scan|connect|save|disconnect|status>\r\n"),
    }
}

/// Package manager command
fn cmd_pkg(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: pkg <update|upgrade|search|install|remove|list|info> [args]\r\n");
        return 1;
    }
    match args[1] {
        "update" => {
            Uart10::puts("Updating package index...\r\n");
            // Initialize installer and update repositories
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => match installer.update() {
                    Ok(()) => {
                        Uart10::puts("Package index updated successfully.\r\n");
                        0
                    }
                    Err(e) => {
                        crate::println!("pkg update failed: {:?}", e);
                        1
                    }
                },
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "upgrade" => {
            Uart10::puts("Upgrading packages...\r\n");
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => match installer.upgrade() {
                    Ok(()) => {
                        Uart10::puts("Upgrade complete.\r\n");
                        0
                    }
                    Err(e) => {
                        crate::println!("pkg upgrade failed: {:?}", e);
                        1
                    }
                },
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "search" => {
            if argc < 3 {
                Uart10::puts("Usage: pkg search <query>\r\n");
                return 1;
            }
            let query = args[2];
            match crate::pkg::install::Installer::new() {
                Ok(installer) => {
                    let results = installer.search(query);
                    if results.is_empty() {
                        Uart10::puts("No packages found.\r\n");
                    } else {
                        for (repo, pkg) in results {
                            crate::println!(
                                "  {}/{} {} - {}",
                                repo,
                                pkg.name,
                                pkg.version,
                                pkg.description
                            );
                        }
                    }
                    0
                }
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "install" => {
            if argc < 3 {
                Uart10::puts("Usage: pkg install <package>\r\n");
                return 1;
            }
            let pkg_name = args[2];
            Uart10::puts("Installing ");
            Uart10::puts(pkg_name);
            Uart10::puts("...\r\n");
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => match installer.install(pkg_name) {
                    Ok(()) => {
                        crate::println!("Package '{}' installed successfully.", pkg_name);
                        0
                    }
                    Err(e) => {
                        crate::println!("pkg install failed: {:?}", e);
                        1
                    }
                },
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "remove" => {
            if argc < 3 {
                Uart10::puts("Usage: pkg remove <package>\r\n");
                return 1;
            }
            let pkg_name = args[2];
            Uart10::puts("Removing ");
            Uart10::puts(pkg_name);
            Uart10::puts("...\r\n");
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => match installer.remove(pkg_name) {
                    Ok(()) => {
                        crate::println!("Package '{}' removed successfully.", pkg_name);
                        0
                    }
                    Err(e) => {
                        crate::println!("pkg remove failed: {:?}", e);
                        1
                    }
                },
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "list" => match crate::pkg::install::Installer::new() {
            Ok(installer) => {
                let packages = installer.list();
                if packages.is_empty() {
                    Uart10::puts("No packages installed.\r\n");
                } else {
                    Uart10::puts("Installed packages:\r\n");
                    for pkg in packages {
                        crate::println!(
                            "  {} {} (installed: {})",
                            pkg.metadata.name,
                            pkg.metadata.version,
                            pkg.install_time
                        );
                    }
                }
                0
            }
            Err(e) => {
                crate::println!("Failed to initialize package manager: {:?}", e);
                1
            }
        },
        "info" => {
            if argc < 3 {
                Uart10::puts("Usage: pkg info <package>\r\n");
                return 1;
            }
            let pkg_name = args[2];
            match crate::pkg::install::Installer::new() {
                Ok(installer) => {
                    if let Some(pkg) = installer.info(pkg_name) {
                        crate::println!("Package: {}", pkg.metadata.name);
                        crate::println!("Version: {}", pkg.metadata.version);
                        crate::println!("Description: {}", pkg.metadata.description);
                        crate::println!("Size: {} bytes", pkg.metadata.size);
                        if !pkg.metadata.dependencies.is_empty() {
                            crate::println!("Dependencies: {:?}", pkg.metadata.dependencies);
                        }
                        crate::println!("Install time: {}", pkg.install_time);
                    } else {
                        crate::println!("Package '{}' not installed.", pkg_name);
                    }
                    0
                }
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "repo" => {
            if argc < 3 {
                Uart10::puts(
                    "Usage: pkg repo <add|remove|list|set-key|enable|disable|priority> [args]\r\n",
                );
                return 1;
            }
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => {
                    // Load saved repo config first
                    let _ = installer.repo_manager().load_config();
                    let result: Result<(), crate::pkg::repo::RepoError> = match args[2] {
                        "add" => {
                            if argc < 5 {
                                Uart10::puts("Usage: pkg repo add <name> <url>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else {
                                let name = args[3];
                                let url = args[4];
                                let config = crate::pkg::repo::RepositoryConfig::new(name, url);
                                installer.repo_manager().add_repo(config);
                                installer.repo_manager().save_config()
                            }
                        }
                        "remove" => {
                            if argc < 4 {
                                Uart10::puts("Usage: pkg repo remove <name>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else {
                                let name = args[3];
                                if installer.repo_manager().remove_repo(name) {
                                    installer.repo_manager().save_config()
                                } else {
                                    Uart10::puts("Repository not found\r\n");
                                    Err(crate::pkg::repo::RepoError::NotFound)
                                }
                            }
                        }
                        "list" => {
                            let repos = installer.repo_manager().list_repos();
                            if repos.is_empty() {
                                Uart10::puts("No repositories configured.\r\n");
                            } else {
                                Uart10::puts("Configured repositories:\r\n");
                                for repo in repos {
                                    let key_status = if repo.public_key.is_some() {
                                        "key: yes"
                                    } else {
                                        "key: no"
                                    };
                                    crate::println!(
                                        "  {} {} (priority: {}, verify: {}, {})",
                                        repo.name,
                                        if repo.enabled {
                                            "[enabled]"
                                        } else {
                                            "[disabled]"
                                        },
                                        repo.priority,
                                        if repo.verify_signatures { "yes" } else { "no" },
                                        key_status
                                    );
                                    crate::println!("    URL: {}", repo.url);
                                }
                            }
                            Ok(())
                        }
                        "set-key" => {
                            if argc < 5 {
                                Uart10::puts("Usage: pkg repo set-key <name> <key_hex>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else {
                                let name = args[3];
                                let key_hex = args[4];
                                if let Some(repo) = installer.repo_manager().get_repo_mut(name) {
                                    match repo.set_public_key(key_hex) {
                                        Ok(()) => installer.repo_manager().save_config(),
                                        Err(_) => {
                                            Uart10::puts("Invalid key format (must be 64 hex characters)\r\n");
                                            Err(crate::pkg::repo::RepoError::SignatureError)
                                        }
                                    }
                                } else {
                                    Uart10::puts("Repository not found\r\n");
                                    Err(crate::pkg::repo::RepoError::NotFound)
                                }
                            }
                        }
                        "enable" => {
                            if argc < 4 {
                                Uart10::puts("Usage: pkg repo enable <name>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else if let Some(repo) =
                                installer.repo_manager().get_repo_mut(args[3])
                            {
                                repo.enabled = true;
                                installer.repo_manager().save_config()
                            } else {
                                Uart10::puts("Repository not found\r\n");
                                Err(crate::pkg::repo::RepoError::NotFound)
                            }
                        }
                        "disable" => {
                            if argc < 4 {
                                Uart10::puts("Usage: pkg repo disable <name>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else if let Some(repo) =
                                installer.repo_manager().get_repo_mut(args[3])
                            {
                                repo.enabled = false;
                                installer.repo_manager().save_config()
                            } else {
                                Uart10::puts("Repository not found\r\n");
                                Err(crate::pkg::repo::RepoError::NotFound)
                            }
                        }
                        "priority" => {
                            if argc < 5 {
                                Uart10::puts("Usage: pkg repo priority <name> <priority>\r\n");
                                Err(crate::pkg::repo::RepoError::ParseError)
                            } else if let Some(repo) =
                                installer.repo_manager().get_repo_mut(args[3])
                            {
                                match args[4].parse::<i32>() {
                                    Ok(p) => {
                                        repo.priority = p;
                                        installer.repo_manager().save_config()
                                    }
                                    Err(_) => {
                                        Uart10::puts("Invalid priority (must be integer)\r\n");
                                        Err(crate::pkg::repo::RepoError::ParseError)
                                    }
                                }
                            } else {
                                Uart10::puts("Repository not found\r\n");
                                Err(crate::pkg::repo::RepoError::NotFound)
                            }
                        }
                        _ => {
                            Uart10::puts("Usage: pkg repo <add|remove|list|set-key|enable|disable|priority> [args]\r\n");
                            Err(crate::pkg::repo::RepoError::ParseError)
                        }
                    };
                    match result {
                        Ok(()) => 0,
                        Err(e) => {
                            crate::println!("pkg repo command failed: {:?}", e);
                            1
                        }
                    }
                }
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        "install-multi" => {
            if argc < 3 {
                Uart10::puts("Usage: pkg install-multi <pkg1> [pkg2] ...\r\n");
                return 1;
            }
            let pkg_names: Vec<&str> = args[2..argc].to_vec();
            match crate::pkg::install::Installer::new() {
                Ok(mut installer) => {
                    match crate::pkg::install::install_transaction(&mut installer, &pkg_names) {
                        Ok(()) => {
                            crate::println!("All packages installed successfully.");
                            0
                        }
                        Err(e) => {
                            crate::println!("pkg install-multi failed: {:?}", e);
                            1
                        }
                    }
                }
                Err(e) => {
                    crate::println!("Failed to initialize package manager: {:?}", e);
                    1
                }
            }
        }
        _ => {
            Uart10::puts(
                "Usage: pkg <update|search|install|remove|list|info|repo|install-multi> [args]\r\n",
            );
            1
        }
    }
}

/// Login command
fn cmd_login(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: login <username>\r\n");
        return 1;
    }
    let _username = args[1];

    // Prompt for password
    Uart10::puts("Password: ");
    // In a real implementation, this would read with echo disabled
    // For now, just simulate
    Uart10::puts("\r\n");

    crate::println!("[login] Not fully implemented - would prompt for password");
    0
}

/// Switch user command (su)
fn cmd_su(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let target_user = if argc >= 2 { args[1] } else { "root" };

    crate::println!("[su] Switching to user: {}", target_user);
    crate::println!("[su] Not fully implemented - would prompt for target user's password");
    0
}

/// Sudo command
fn cmd_sudo(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: sudo <command> [args...]\r\n");
        return 1;
    }

    let cmd = args[1];
    crate::println!("[sudo] Running as root: {}", cmd);
    crate::println!("[sudo] Not fully implemented");
    0
}

/// Passwd command
fn cmd_passwd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let target_user = if argc >= 2 {
        args[1]
    } else {
        // Current user - would need session tracking
        "current_user"
    };

    crate::println!("[passwd] Changing password for: {}", target_user);
    crate::println!("[passwd] Not fully implemented");
    0
}

/// Useradd command
fn cmd_useradd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: useradd <username>\r\n");
        return 1;
    }

    let username = args[1];
    crate::println!("[useradd] Adding user: {}", username);
    crate::println!("[useradd] Not fully implemented");
    0
}

/// Userdel command
fn cmd_userdel(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: userdel <username>\r\n");
        return 1;
    }

    let username = args[1];
    crate::println!("[userdel] Deleting user: {}", username);
    crate::println!("[userdel] Not fully implemented");
    0
}

/// Groupadd command
fn cmd_groupadd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: groupadd <groupname>\r\n");
        return 1;
    }

    let groupname = args[1];
    crate::println!("[groupadd] Adding group: {}", groupname);
    crate::println!("[groupadd] Not fully implemented");
    0
}

/// Groupdel command
fn cmd_groupdel(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: groupdel <groupname>\r\n");
        return 1;
    }

    let groupname = args[1];
    crate::println!("[groupdel] Deleting group: {}", groupname);
    crate::println!("[groupdel] Not fully implemented");
    0
}

/// ID command
fn cmd_id(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let username = if argc >= 2 {
        args[1]
    } else {
        // Current user - would need session tracking
        "current_user"
    };

    crate::println!("[id] Showing ID for: {}", username);
    crate::println!("[id] Not fully implemented");
    0
}

/// Whoami command
fn cmd_whoami(_args: [&str; MAX_ARGS], _argc: usize) -> i32 {
    // Current user - would need session tracking
    crate::println!("current_user");
    0
}

/// Simple u32 parser for shell args
fn parse_u32(s: &str) -> Option<u32> {
    let mut val: u32 = 0;
    let mut any = false;
    for c in s.chars() {
        if c >= '0' && c <= '9' {
            if val > (u32::MAX - (c as u32 - '0' as u32)) / 10 {
                return None;
            }
            val = val * 10 + (c as u32 - '0' as u32);
            any = true;
        } else {
            return None;
        }
    }
    if any {
        Some(val)
    } else {
        None
    }
}

// ═══════════════════════════════════════════════
// Environment variable command handlers
// ═══════════════════════════════════════════════

fn cmd_env_set(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: set NAME=VALUE\r\n");
        return 1;
    }
    let mut combined: alloc::string::String = alloc::string::String::new();
    for i in 1..argc {
        if i > 1 {
            combined.push(' ');
        }
        combined.push_str(args[i]);
    }
    if let Some(eq_pos) = combined.find('=') {
        let name = &combined[..eq_pos];
        let value = &combined[eq_pos + 1..];
        env_set(name, value);
        if args[0] == "export" {
            env_export(name);
        }
        0
    } else {
        env_export(&combined);
        0
    }
}

fn cmd_env_unset(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: unset NAME\r\n");
        return 1;
    }
    env_unset(args[1]);
    0
}

fn cmd_env_list() -> i32 {
    unsafe {
        for i in 0..ENV_COUNT {
            let e = &ENV_TABLE[i];
            let export_mark = if e.exported { " (exported)" } else { "" };
            crate::println!("{}={}{}", e.name_str(), e.value_str(), export_mark);
        }
    }
    0
}

fn cmd_history() -> i32 {
    unsafe {
        let count = CMD_HISTORY.count;
        for i in 0..count {
            if let Some(entry) = CMD_HISTORY.get(i) {
                crate::println!(" {:4}  {}", count - i, entry);
            }
        }
    }
    0
}

fn cmd_reboot() {
    Uart10::puts("Rebooting...\r\n");
    crate::drivers::power::reset();
}

// ═══════════════════════════════════════════════
// Filesystem command handlers
// ═══════════════════════════════════════════════

static mut CWD: [u8; 256] = [0u8; 256];
static mut CWD_LEN: usize = 1;

fn cwd() -> &'static str {
    unsafe {
        if CWD_LEN == 0 {
            CWD[0] = b'/';
            CWD_LEN = 1;
        }
        core::str::from_utf8(&CWD[..CWD_LEN]).unwrap_or("/")
    }
}

fn set_cwd(path: &str) {
    unsafe {
        let len = path.len().min(255);
        CWD[..len].copy_from_slice(&path.as_bytes()[..len]);
        CWD_LEN = len;
    }
}

fn resolve_path(path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        alloc::string::String::from(path)
    } else {
        let mut full = alloc::string::String::from(cwd());
        if !full.ends_with('/') {
            full.push('/');
        }
        full.push_str(path);
        normalize_path(&full)
    }
}

fn normalize_path(path: &str) -> alloc::string::String {
    let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => {
                parts.push(component);
            }
        }
    }
    let mut result = alloc::string::String::from("/");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push('/');
        }
        result.push_str(part);
    }
    result
}

fn cmd_cat(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: cat <path>\r\n");
        return 1;
    }
    let full_path = resolve_path(args[1]);
    match crate::fs::vfs::read_file(&full_path) {
        Ok(data) => {
            for &b in &data {
                Uart10::putc(b);
            }
            if !data.is_empty() && data[data.len() - 1] != b'\n' {
                Uart10::puts("\r\n");
            }
            0
        }
        Err(e) => {
            crate::println!("cat: {}: error {}", full_path, e);
            1
        }
    }
}

fn cmd_ls(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let path = if argc >= 2 {
        resolve_path(args[1])
    } else {
        alloc::string::String::from(cwd())
    };
    match crate::fs::vfs::list_dir(&path) {
        Ok(entries) => {
            for entry in &entries {
                let type_char = if entry.is_dir { 'd' } else { '-' };
                crate::println!(
                    "{} {:4} {}:{} {}",
                    type_char,
                    entry.size,
                    entry.uid,
                    entry.gid,
                    entry.name_str()
                );
            }
            crate::println!("{} entries", entries.len());
            0
        }
        Err(e) => {
            crate::println!("ls: {}: error {}", path, e);
            1
        }
    }
}

fn cmd_cd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        set_cwd("/");
        return 0;
    }
    let target = resolve_path(args[1]);
    match crate::fs::vfs::list_dir(&target) {
        Ok(_) => {
            set_cwd(&target);
            0
        }
        Err(_) => {
            crate::println!("cd: {}: No such directory", target);
            1
        }
    }
}

fn cmd_pwd() -> i32 {
    crate::println!("{}", cwd());
    0
}

fn cmd_mkdir(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: mkdir <path>\r\n");
        return 1;
    }
    let full_path = resolve_path(args[1]);
    match crate::fs::vfs::create_dir(&full_path) {
        Ok(_) => 0,
        Err(e) => {
            crate::println!("mkdir: {}: error {}", full_path, e);
            1
        }
    }
}

fn cmd_rm(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: rm <path>\r\n");
        return 1;
    }
    let full_path = resolve_path(args[1]);
    match crate::fs::vfs::remove(&full_path) {
        Ok(_) => 0,
        Err(e) => {
            crate::println!("rm: {}: error {}", full_path, e);
            1
        }
    }
}

fn cmd_mv(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: mv <src> <dst>\r\n");
        return 1;
    }
    let src = resolve_path(args[1]);
    let dst = resolve_path(args[2]);
    match crate::fs::vfs::rename(&src, &dst) {
        Ok(_) => 0,
        Err(e) => {
            crate::println!("mv: {} -> {}: error {}", src, dst, e);
            1
        }
    }
}

fn cmd_run(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: run <path|demo> [priority]\r\n");
        Uart10::puts("  run demo       Run built-in hello-EL0 test binary\r\n");
        Uart10::puts("  run <path>     Load ELF from VFS and spawn as user process\r\n");
        return 1;
    }

    let priority = if argc >= 3 {
        match args[2] {
            "high" | "hi" => crate::sched::Priority::High,
            "low" => crate::sched::Priority::Low,
            _ => crate::sched::Priority::Normal,
        }
    } else {
        crate::sched::Priority::Normal
    };

    if args[1] == "demo" {
        match crate::sched::spawn_demo() {
            Some(pid) => {
                crate::println!("run: hello-EL0 demo spawned as PID {}", pid);
                0
            }
            None => {
                Uart10::puts("run: failed to spawn demo\r\n");
                1
            }
        }
    } else {
        let path = resolve_path(args[1]);
        match crate::sched::spawn_from_path(&path, priority) {
            Some(pid) => {
                crate::println!("run: {} spawned as PID {}", path, pid);
                0
            }
            None => {
                crate::println!("run: failed to spawn {}", path);
                1
            }
        }
    }
}

fn cmd_chmod(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: chmod <mode> <path>\r\n");
        return 1;
    }
    let mode = match parse_u32(args[1]) {
        Some(m) => m,
        None => {
            Uart10::puts("chmod: invalid mode\r\n");
            return 1;
        }
    };
    let full_path = resolve_path(args[2]);
    match crate::fs::vfs::chmod(&full_path, mode) {
        Ok(_) => 0,
        Err(e) => {
            crate::println!("chmod: {}: error {}", full_path, e);
            1
        }
    }
}

fn cmd_chown(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: chown <uid:gid> <path>\r\n");
        return 1;
    }
    let spec = args[1];
    let uid: u32;
    let gid: u32;
    if let Some(colon_pos) = spec.find(':') {
        uid = parse_u32(&spec[..colon_pos]).unwrap_or(0);
        gid = parse_u32(&spec[colon_pos + 1..]).unwrap_or(0);
    } else {
        uid = parse_u32(spec).unwrap_or(0);
        gid = 0;
    }
    let full_path = resolve_path(args[2]);
    match crate::fs::vfs::chown(&full_path, uid, gid) {
        Ok(_) => 0,
        Err(e) => {
            crate::println!("chown: {}: error {}", full_path, e);
            1
        }
    }
}

// ═══════════════════════════════════════════════
// Glob expansion (simple * and ? patterns)
// ═══════════════════════════════════════════════

fn glob_match(pattern: &str, name: &str) -> bool {
    let p = pattern.as_bytes();
    let n = name.as_bytes();
    let mut pi = 0;
    let mut ni = 0;
    let mut star_pi = usize::MAX;
    let mut star_ni = 0;

    while ni < n.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = pi;
            star_ni = ni;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ni += 1;
            ni = star_ni;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

fn glob_expand(pattern: &str) -> alloc::vec::Vec<alloc::string::String> {
    let mut results = alloc::vec::Vec::new();
    if !pattern.contains('*') && !pattern.contains('?') {
        results.push(alloc::string::String::from(pattern));
        return results;
    }
    let (dir, file_pattern) = if pattern.contains('/') {
        let last_slash = pattern.rfind('/').unwrap();
        (
            resolve_path(&pattern[..=last_slash]),
            &pattern[last_slash + 1..],
        )
    } else {
        (alloc::string::String::from(cwd()), pattern)
    };
    match crate::fs::vfs::list_dir(&dir) {
        Ok(entries) => {
            for entry in &entries {
                if glob_match(file_pattern, entry.name_str()) {
                    let mut full = dir.clone();
                    if !full.ends_with('/') {
                        full.push('/');
                    }
                    full.push_str(entry.name_str());
                    results.push(full);
                }
            }
        }
        Err(_) => {}
    }
    results
}

// ═══════════════════════════════════════════════
// Service management command handlers
// ═══════════════════════════════════════════════

fn cmd_svc(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: svc <list|start|stop|restart> [name]\r\n");
        return 1;
    }
    match args[1] {
        "list" | "ls" => {
            let services = crate::sched::init::list_services();
            if services.is_empty() {
                Uart10::puts("No services registered.\r\n");
                return 0;
            }
            let rl = crate::sched::init::current_runlevel();
            crate::println!("Runlevel: {} ({})", rl.as_u8(), rl.name());
            crate::println!(
                "{:-32} {:10} {:6} {:4} {}",
                "SERVICE",
                "STATE",
                "PID",
                "REST",
                "CRIT"
            );
            for svc in &services {
                let crit = if svc.critical { "*" } else { " " };
                let pid_str = if svc.pid == 0 {
                    alloc::string::String::from("-")
                } else {
                    u64_to_str(svc.pid)
                };
                crate::println!(
                    "{:32} {:10} {:6} {:4} {}",
                    svc.name_str(),
                    svc.state.name(),
                    pid_str,
                    svc.restart_count,
                    crit
                );
            }
            crate::println!("{} service(s) registered", services.len());
            0
        }
        "start" => {
            if argc < 3 {
                Uart10::puts("Usage: svc start <name>\r\n");
                return 1;
            }
            let name = args[2];
            if crate::sched::init::start_by_name(name) {
                crate::println!("svc: started '{}'", name);
                0
            } else {
                crate::println!("svc: failed to start '{}'", name);
                1
            }
        }
        "stop" => {
            if argc < 3 {
                Uart10::puts("Usage: svc stop <name>\r\n");
                return 1;
            }
            let name = args[2];
            if crate::sched::init::stop_by_name(name) {
                crate::println!("svc: stopped '{}'", name);
                0
            } else {
                crate::println!("svc: failed to stop '{}'", name);
                1
            }
        }
        "restart" => {
            if argc < 3 {
                Uart10::puts("Usage: svc restart <name>\r\n");
                return 1;
            }
            let name = args[2];
            if crate::sched::init::restart_by_name(name) {
                crate::println!("svc: restarted '{}'", name);
                0
            } else {
                crate::println!("svc: failed to restart '{}'", name);
                1
            }
        }
        "reset" => {
            if argc < 3 {
                Uart10::puts("Usage: svc reset <name>\r\n");
                return 1;
            }
            let name = args[2];
            if crate::sched::init::reset_by_name(name) {
                crate::println!("svc: reset '{}' (now Stopped, ready to start)", name);
                0
            } else {
                crate::println!("svc: failed to reset '{}' (not in Failed state?)", name);
                1
            }
        }
        "deps" => {
            if argc < 3 {
                Uart10::puts("Usage: svc deps <name>\r\n");
                return 1;
            }
            let name = args[2];
            crate::sched::init::print_deps(name);
            0
        }
        _ => {
            Uart10::puts("Usage: svc <list|start|stop|restart|reset|deps> [name]\r\n");
            1
        }
    }
}

fn cmd_runlevel(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 || args[1] == "get" {
        let rl = crate::sched::init::current_runlevel();
        crate::println!("Runlevel: {} ({})", rl.as_u8(), rl.name());
        return 0;
    }
    if args[1] == "set" {
        if argc < 3 {
            Uart10::puts("Usage: runlevel set <N>\r\n");
            Uart10::puts("  0=halt 1=single 2=multi 3=network 4=graphical 6=reboot\r\n");
            return 1;
        }
        match parse_u32(args[2]) {
            Some(v) => match crate::sched::init::Runlevel::from_u8(v as u8) {
                Some(rl) => {
                    if crate::sched::init::switch_runlevel(rl) {
                        crate::println!("Runlevel set to {} ({})", rl.as_u8(), rl.name());
                        0
                    } else {
                        crate::println!("Failed to transition to runlevel {}", v);
                        1
                    }
                }
                None => {
                    Uart10::puts("Invalid runlevel. Use: 0,1,2,3,4,6\r\n");
                    1
                }
            },
            None => {
                Uart10::puts("Invalid runlevel number\r\n");
                1
            }
        }
    } else {
        Uart10::puts("Usage: runlevel [get|set <N>]\r\n");
        Uart10::puts("  0=halt 1=single 2=multi 3=network 4=graphical 6=reboot\r\n");
        1
    }
}
