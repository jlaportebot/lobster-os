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
use alloc::string::String;
use alloc::vec::Vec;

/// Shell prompt string
const PROMPT: &str = "lobster> ";

/// Maximum command line length
const MAX_CMD_LEN: usize = 512;

/// Maximum number of arguments per command
const MAX_ARGS: usize = 32;

/// Set by the line parser when a command ends with `&` (background).
/// The command handler consumes it (run spawns sliced; others ignore).
static BG_PENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

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

/// Quote-aware whitespace tokenizer (sh-style):
/// `a "b c" d` -> ["a", "b c", "d"]; surrounding quotes are stripped.
/// Quotes inside a token are preserved as literal characters.
fn split_args_shell(line: &str) -> alloc::vec::Vec<&str> {
    let mut out: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();
    // Leaked-arena approach is not available; we can only return subslices of
    // the input line, so quoted spans that need dequoting cannot be returned
    // as &str without allocation. Handle by returning the span INCLUDING the
    // quotes stripped only when they wrap the whole token — which requires an
    // owned buffer. To keep lifetimes simple we instead strip quotes in-place
    // is impossible; so build owned Strings only when a token contains quotes.
    while i < n {
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let start = i;
        let mut end;
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let q = bytes[i];
            i += 1;
            let inner_start = i;
            while i < n && bytes[i] != q {
                i += 1;
            }
            end = i.min(n);
            let inner = &line[inner_start..end];
            // Skip past closing quote
            if i < n {
                i += 1;
            }
            // If there is trailing non-space content glued to the quote,
            // extend to include it (rare; keep simple).
            while i < n && !bytes[i].is_ascii_whitespace() {
                i += 1;
                end = i;
            }
            out.push(inner);
            let _ = start;
        } else {
            while i < n && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let tok = &line[start..i];
            out.push(tok);
            end = i;
        }
        let _ = end;
    }
    out
}

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
                        #[cfg(target_arch = "aarch64")]
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

/// Poll the shell once (non-blocking). Called from the cooperative main loop.
/// Returns true if a command was executed.
pub fn poll() -> bool {
    // On QEMU, the UART RX interrupt (SPI 121) doesn't fire reliably.
    // Poll the UART directly for input bytes instead.
    let byte_avail = crate::drivers::uart::Uart10::has_data();

    if byte_avail {
        while let Some(b) = crate::drivers::uart::Uart10::read_byte_nb() {
            unsafe { crate::drivers::serial_input::feed_byte(b) };
        }
    }

    let mut line_buf = [0u8; MAX_CMD_LEN];
    let len = serial_input::read_line(&mut line_buf);
    if len > 0 {
        let cmd_str = core::str::from_utf8(&line_buf[..len]).unwrap_or("");
        let _result = execute_command(cmd_str.trim());
        // Re-print prompt after command (with CWD)
        let prompt = build_prompt();
        Uart10::puts(&prompt);
        true
    } else {
        false
    }
}

/// Build a dynamic prompt that shows CWD (like a real shell)
fn build_prompt() -> alloc::string::String {
    let dir = cwd();
    let short_dir = if dir == "/" {
        "/"
    } else {
        // Show last component for brevity, full path if short enough
        if dir.len() <= 20 {
            dir
        } else if let Some(idx) = dir.rfind('/') {
            &dir[idx..]
        } else {
            dir
        }
    };
    alloc::format!("lobster:{}# ", short_dir)
}

fn print_prompt(pty_slave_fd: Option<usize>) {
    let prompt = build_prompt();
    if let Some(fd) = pty_slave_fd {
        print_to_pty(fd, &prompt);
    } else {
        Uart10::puts(&prompt);
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
pub fn print_banner() {
    Uart10::puts("\r\n");
    Uart10::puts("  +======================================+\r\n");
    Uart10::puts("  |       LobsterOS v0.2.0               |\r\n");
    Uart10::puts("  |   AArch64 Bare-Metal Operating System |\r\n");
    Uart10::puts("  |        Raspberry Pi 5 / BCM2712       |\r\n");
    Uart10::puts("  +======================================+\r\n");
    Uart10::puts("\r\n");
    Uart10::puts("Type 'help' for a list of commands.\r\n\r\n");
}

/// Print the initial shell prompt (called once after banner)
pub fn print_initial_prompt() {
    let prompt = build_prompt();
    Uart10::puts(&prompt);
}

/// Parse and execute a command string (advanced: pipes, redirects, env vars)
/// Public GUI entry: execute a command line from the GUI shell input.
/// Output flows through println! (UART), which mirrors into the terminal
/// window's VT screen.
pub fn execute_for_gui(line: &str) -> i32 {
    let _ = execute_command(line);
    0
}

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
    // Publish the flag BEFORE dispatch so the command handler (cmd_run)
    // can consume it. Cleared unconditionally first so stale flags from
    // crashed lines never leak into the next command.
    BG_PENDING.store(background, core::sync::atomic::Ordering::Relaxed);

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
        // Tokenize with quote-awareness: split on whitespace, but keep
        // quoted runs together and strip the quotes (sh-style).
        let tokens: alloc::vec::Vec<&str> = split_args_shell(stage_str);
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

/// Output sink for builtin commands: honors `> file`, `>> file` and pipe
/// stages; falls back to the serial console. Builtins print through this
/// instead of touching Uart10 directly.
enum ShellOut {
    /// Write to serial console (no redirect/pipe)
    Console,
    /// Redirect to a file (fd, append flag already handled at open)
    File(usize),
}

impl ShellOut {
    /// Create the sink for the current pipeline stage.
    fn open(redir: &Redirect, pipe_out: Option<i32>) -> ShellOut {
        match redir {
            Redirect::Write(path) | Redirect::Append(path) => {
                use crate::fs::vfs;
                let flags = if matches!(redir, Redirect::Append(_)) {
                    vfs::O_WRONLY | vfs::O_CREAT | vfs::O_APPEND
                } else {
                    vfs::O_WRONLY | vfs::O_CREAT | vfs::O_TRUNC
                };
                match vfs::open(path.as_str(), flags) {
                    Ok(fd) => ShellOut::File(fd as usize),
                    Err(_) => ShellOut::Console,
                }
            }
            Redirect::None => {
                if let Some(fd) = pipe_out {
                    ShellOut::File(fd as usize)
                } else {
                    ShellOut::Console
                }
            }
            // stdin redirection doesn't affect the output sink
            Redirect::Read(_) => ShellOut::Console,
        }
    }

    fn puts(&self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    fn write_bytes(&self, b: &[u8]) {
        match self {
            ShellOut::Console => Uart10::puts_bytes(b),
            ShellOut::File(fd) => {
                let _ = crate::fs::vfs::write(*fd, b);
            }
        }
    }

    fn close(&self) {
        if let ShellOut::File(fd) = self {
            let _ = crate::fs::vfs::close(*fd);
        }
    }
}

/// Run a builtin command, returning an exit code (0 = success, 1 = failure)
fn run_builtin(
    cmd: &str,
    args: [&str; MAX_ARGS],
    argc: usize,
    _pipe_in: i32,
    stdout_redir: &Redirect,
    _stdin_redir: &Redirect,
    pipe_out: Option<i32>,
) -> i32 {
    // Sink honoring > file / >> file / pipe stages for output-producing builtins.
    let out = ShellOut::open(stdout_redir, pipe_out);
    let code = run_builtin_out(cmd, args, argc, &out);
    out.close();
    code
}

/// Builtin dispatch with the stage's output sink applied.
fn run_builtin_out(
    cmd: &str,
    args: [&str; MAX_ARGS],
    argc: usize,
    out: &ShellOut,
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
        "echo" => cmd_echo(args, argc, out),
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
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "ifconfig" => {
            cmd_ifconfig();
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "ping" => {
            cmd_ping(args, argc);
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "arp" => {
            cmd_arp(args, argc);
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "dhcp" => {
            cmd_dhcp(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "dhcp" => {
            Uart10::puts("DHCP not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "dns" => {
            cmd_dns(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "dns" => {
            Uart10::puts("DNS not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "net" => {
            cmd_net(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "net" => {
            Uart10::puts("Network commands not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "wm" => {
            cmd_wm(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "wm" => {
            Uart10::puts("Window manager not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "audio" => {
            cmd_audio(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "audio" => {
            Uart10::puts("Audio not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "gpu" => {
            cmd_gpu(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "gpu" => {
            Uart10::puts("GPU not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "wifi" => {
            cmd_wifi(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "wifi" => {
            Uart10::puts("WiFi not available on x86_64.\r\n");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "pkg" => cmd_pkg(args, argc),
        #[cfg(all(feature = "qemu", target_arch = "aarch64"))]
        "pkg" | "apt" => cmd_pkg(args, argc),
        #[cfg(target_arch = "x86_64")]
        "pkg" | "apt" => {
            Uart10::puts("Package manager not available on x86_64.\r\n");
            0
        }
        "gh" => cmd_gh(args, argc),
        "theme" => {
            #[cfg(target_arch = "aarch64")]
            {
                if argc >= 2 {
                    crate::wm::theme::toggle();
                } else {
                    crate::println!("theme: {} (run 'theme toggle' to switch)", crate::wm::theme::current_name());
                }
            }
            0
        }
        "wallpaper" => {
            #[cfg(target_arch = "aarch64")]
            {
                if argc >= 2 {
                    let ok = crate::wm::wallpaper::set_by_name(args[1]);
                    crate::println!("wallpaper: {}", if ok { args[1] } else { "not found" });
                } else {
                    crate::wm::wallpaper::next();
                    crate::println!("wallpaper: {}", crate::wm::wallpaper::active().name);
                }
            }
            0
        }
        "browser" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 {
                    let id = crate::apps::browser::open(args[1]);
                    crate::println!("browser: opened {} (win {})", args[1], id);
                } else {
                    crate::println!("browser: usage: browser <url>");
                }
            }
            0
        }
        "browsecmd" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 {
                    match args[1] {
                        "back" => { unsafe { crate::apps::browser::INSTANCE.as_mut().map(|b| b.back()); }; }
                        "fwd" => { unsafe { crate::apps::browser::INSTANCE.as_mut().map(|b| b.forward()); }; }
                        "down" => crate::apps::browser::scroll(true),
                        "up" => crate::apps::browser::scroll(false),
                        _ => {}
                    }
                }
            }
            0
        }
        "resolution" => {
            #[cfg(target_arch = "aarch64")]
            {
                let (w, h) = if argc >= 3 {
                    (args[1].parse::<u32>().unwrap_or(0), args[2].parse::<u32>().unwrap_or(0))
                } else {
                    (0, 0)
                };
                const MIN_W: u32 = 640;
                const MIN_H: u32 = 480;
                const MAX_W: u32 = 2560;
                const MAX_H: u32 = 1600;
                if w < MIN_W || h < MIN_H || w > MAX_W || h > MAX_H {
                    crate::println!("usage: resolution <{}-{}> <{}-{}>", MIN_W, MAX_W, MIN_H, MAX_H);
                    return 0;
                }
                match crate::display::fb::init(w, h, 32) {
                    Ok(info) => {
                        crate::display::console::init(info);
                        // Repaint the desktop onto the new surface and
                        // pull all windows back on-screen.
                        crate::wm::wallpaper::paint_to_screen();
                        let count = crate::wm::window::count();
                        for id in 1..=count as u32 {
                            if !crate::wm::window::exists(id) {
                                continue;
                            }
                            if let Some((x, y, ww, hh)) = crate::wm::window::get_info(id) {
                                let nx = x.min(info.width.saturating_sub(64));
                                let ny = y.min(info.height.saturating_sub(64));
                                if nx != x || ny != y {
                                    crate::wm::window::move_window(id, nx, ny);
                                }
                            }
                        }
                        crate::wm::dirty::mark();
                        crate::println!("resolution: now {}x{}", info.width, info.height);
                    }
                    Err(e) => crate::println!("resolution: failed: {:?}", e),
                }
            }
            0
        }
        "beep" => {
            #[cfg(target_arch = "aarch64")]
            {
                let freq = if argc >= 2 { args[1].parse::<u64>().unwrap_or(440_000) } else { 440_000 };
                let ms = if argc >= 3 { args[2].parse::<u64>().unwrap_or(300) } else { 300 };
                if !crate::drivers::lobster_audio::detect() {
                    crate::println!("beep: audio device absent");
                } else {
                    let n = crate::drivers::lobster_audio::beep(freq, ms);
                    crate::println!("beep: played {} bytes of PCM", n);
                }
            }
            0
        }
        "play" => {
            #[cfg(target_arch = "aarch64")]
            {
                if argc < 2 {
                    crate::println!("usage: play <16-bit mono 8kHz raw pcm file>");
                    return 0;
                }
                if !crate::drivers::lobster_audio::detect() {
                    crate::println!("play: audio device absent");
                    return 0;
                }
                match crate::fs::vfs::read_file(args[1]) {
                    Ok(data) => {
                        let n = crate::drivers::lobster_audio::play_pcm(&data);
                        crate::println!("play: {} bytes", n);
                    }
                    Err(e) => crate::println!("play: read {} failed: {:?}", args[1], e),
                }
            }
            0
        }
        "share" => {
            match if argc >= 2 { Some(args[1]) } else { None } {
                None => {
                    if crate::drivers::lobster_share::detect() {
                        crate::println!("share: device present (host LBST_SHARE)");
                        let mut n = 0;
                        let _ = crate::drivers::lobster_share::list_dir("", |name| {
                            crate::println!("  {}", name);
                            n += 1;
                        });
                        crate::println!("share: {} entries", n);
                    } else {
                        crate::println!("share: device absent");
                    }
                    0
                }
                Some("cat") if argc >= 3 => {
                    let name = args[2];
                    match crate::drivers::lobster_share::read_file(name) {
                        Some(data) => {
                            if let Ok(s) = core::str::from_utf8(&data) {
                                crate::println!("{}", s.trim_end());
                            } else {
                                crate::println!("share: {} is binary ({} bytes)", name, data.len());
                            }
                            0
                        }
                        None => {
                            crate::println!("share: not found: {}", name);
                            0
                        }
                    }
                }
                Some("cp") if argc >= 4 => {
                    let (from, to) = (args[2], args[3]);
                    match crate::drivers::lobster_share::read_file(from) {
                        Some(data) => {
                            use crate::fs::vfs;
                            let fd = match vfs::open(to, vfs::O_WRONLY | vfs::O_CREAT | vfs::O_TRUNC) {
                                Ok(fd) => fd,
                                Err(e) => {
                                    crate::println!("share: cannot create {}: {:?}", to, e);
                                    return 0;
                                }
                            };
                            let _ = vfs::write(fd, &data);
                            vfs::close(fd);
                            crate::println!("share: {} -> {} ({} bytes)", from, to, data.len());
                            0
                        }
                        None => {
                            crate::println!("share: not found: {}", from);
                            0
                        }
                    }
                }
                Some(other) => {
                    crate::println!("share: unknown subcommand '{}'", other);
                    0
                }
            }
        }
        "wins" => {
            #[cfg(target_arch = "aarch64")]
            {
                let count = crate::wm::window::count();
                crate::println!("wins: {} windows (focused id={})", count, crate::wm::window::focused_id());
                for id in 1..=count as u32 {
                    if crate::wm::window::exists(id) {
                        if let Some((x, y, w, h)) = crate::wm::window::get_info(id) {
                            crate::println!("wins: id={} '{}' at ({},{}) {}x{}", id, crate::wm::window::get_title(id), x, y, w, h);
                        }
                    }
                }
            }
            0
        }
        "image" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 {
                    match crate::apps::image_viewer::open_image(args[1]) {
                        Ok(()) => crate::println!("image: loaded {}", args[1]),
                        Err(e) => crate::println!("image: {}: {}", args[1], e),
                    }
                } else {
                    crate::println!("image: usage: image <bmp-path>");
                }
            }
            0
        }
        "accent" => {
            #[cfg(target_arch = "aarch64")]
            {
                if argc >= 2 {
                    if args[1] == "next" {
                        crate::wm::theme::cycle_accent_name();
                        crate::println!("accent: {:#010x}", crate::wm::theme::accent_color());
                    } else if crate::wm::theme::set_accent_name(args[1]) {
                        crate::println!("accent: {} set", args[1]);
                    } else {
                        crate::println!("accent: unknown '{}'", args[1]);
                    }
                } else {
                    crate::println!("accent: {:#010x} (try next or a preset name)", crate::wm::theme::accent_color());
                }
            }
            0
        }
        "bookmark" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                crate::apps::browser::init();
                crate::apps::browser::toggle_bookmark();
                crate::println!("bookmark: {} bookmarks (current URL toggled)",
                    crate::apps::browser::bookmark_count());
            }
            0
        }
        "fm" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                let fm = crate::apps::file_manager::get_file_manager();
                if argc >= 2 {
                    match args[1] {
                        "nav" if argc >= 3 => {
                            fm.cd(args[2]);
                            crate::println!("fm: cwd {}", fm.current_path());
                        }
                        "up" => {
                            fm.cd_up();
                            crate::println!("fm: cwd {}", fm.current_path());
                        }
                        "refresh" => {
                            fm.refresh();
                            crate::println!("fm: {} entries", fm.entry_count());
                        }
                        "sel" if argc >= 3 => {
                            match args[2].parse::<usize>() {
                                Ok(n) => {
                                    fm.selected_index = n;
                                    crate::println!("fm: selected {}", n);
                                }
                                Err(_) => crate::println!("fm: bad index"),
                            }
                        }
                        "copy" if argc >= 3 => {
                            match fm.copy_selected(args[2]) {
                                Ok(()) => crate::println!("fm: copied to {}", args[2]),
                                Err(e) => crate::println!("fm: {}", e),
                            }
                        }
                        "move" if argc >= 3 => {
                            match fm.move_selected(args[2]) {
                                Ok(()) => crate::println!("fm: moved to {}", args[2]),
                                Err(e) => crate::println!("fm: {}", e),
                            }
                        }
                        "del" => {
                            match fm.delete_selected() {
                                Ok(()) => crate::println!("fm: deleted (persisted)"),
                                Err(e) => crate::println!("fm: {}", e),
                            }
                        }
                        "rename" if argc >= 3 => {
                            match fm.rename_selected(args[2]) {
                                Ok(()) => crate::println!("fm: renamed to {}", args[2]),
                                Err(e) => crate::println!("fm: {}", e),
                            }
                        }
                        "ls" => {
                            crate::println!("fm: cwd {} ({} entries)", fm.current_path(), fm.entry_count());
                        }
                        _ => crate::println!("fm: unknown subcommand"),
                    }
                } else {
                    crate::println!("fm: usage: fm <nav|up|refresh|sel|del|rename|ls> [...]");
                }
            }
            0
        }
        "iconsel" => {
            #[cfg(target_arch = "aarch64")]
            {
                if argc >= 2 {
                    match args[1].parse::<usize>() {
                        Ok(n) => { crate::wm::desktop_icons::select(Some(n)); crate::println!("iconsel: {}", n); }
                        Err(_) => { crate::wm::desktop_icons::select(None); crate::println!("iconsel: cleared"); }
                    }
                }
            }
            0
        }
        "gkey" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 {
                    // Route as if typed: chars, or special names enter/back
                    let arg = args[1];
                    if arg == "enter" {
                        crate::wm::gui_shell::handle_key(0x28, None);
                    } else if arg == "back" {
                        crate::wm::gui_shell::handle_key(0x2A, None);
                    } else {
                        let c = arg.chars().next().unwrap_or(' ');
                        crate::wm::gui_shell::handle_key(0x04, Some(c));
                    }
                    crate::println!("gkey: fed '{}'", arg);
                }
            }
            0
        }
        "settings" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 && args[1] == "next" {
                    crate::apps::settings::cycle_page();
                    crate::println!("settings: page cycled");
                } else {
                    let id = crate::apps::settings::open_window();
                    crate::println!("settings: window {}", id);
                }
            }
            0
        }
        "notes" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                match argc {
                    1 => {
                        crate::apps::notes::init();
                        let id = crate::wm::window::find_by_title("Notes")
                            .unwrap_or_else(|| crate::wm::window::create(260, 140, 700, 480, "Notes").unwrap_or(0));
                        crate::println!("notes: {} notes (win {})", crate::apps::notes::note_count(), id);
                    }
                    _ => match args[1] {
                        "new" => { crate::apps::notes::init(); crate::apps::notes::new_note(); crate::println!("notes: created"); }
                        "type" if argc >= 3 => {
                            crate::apps::notes::init();
                            crate::apps::notes::type_char(args[2].chars().next().unwrap_or(' '));
                            crate::println!("notes: typed");
                        }
                        _ => {}
                    },
                }
            }
            0
        }
        "edit" => {
            #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
            {
                if argc >= 2 {
                    let id = crate::apps::editor::open_file(args[1]);
                    crate::println!("edit: opened {} (win {})", args[1], id);
                } else {
                    crate::println!("edit: usage: edit <path>");
                }
            }
            0
        }
        "alttab" => {
            #[cfg(target_arch = "aarch64")]
            {
                if !crate::wm::alt_tab::is_visible() {
                    crate::wm::alt_tab::open();
                    crate::println!("alttab: switcher open");
                } else {
                    crate::wm::alt_tab::close(true);
                    crate::println!("alttab: switcher closed, focused");
                }
            }
            0
        }
        "tray" => {
            #[cfg(target_arch = "aarch64")]
            {
                use crate::wm::tray_popups::PopupKind;
                let kind = match argc {
                    2 => match args[1] {
                        "clock" => PopupKind::Clock,
                        "network" => PopupKind::Network,
                        "volume" => PopupKind::Volume,
                        "battery" => PopupKind::Battery,
                        "close" => PopupKind::None,
                        _ => PopupKind::Clock,
                    },
                    _ => PopupKind::None,
                };
                if kind == PopupKind::None {
                    crate::wm::tray_popups::close();
                    crate::println!("tray: closed");
                } else {
                    crate::wm::tray_popups::open(kind);
                    crate::println!("tray: opened");
                }
            }
            0
        }
        "notify" => {
            #[cfg(target_arch = "aarch64")]
            {
                let title = if argc >= 2 { args[1] } else { "Test" };
                let body = if argc >= 3 { args[2] } else { "Notification from the shell" };
                crate::wm::notifications::info(title, body);
                crate::println!("notify: posted '{}'", title);
            }
            #[cfg(not(target_arch = "aarch64"))]
            crate::println!("notify: aarch64 only");
            0
        }
        "app" => {
            if argc >= 2 {
                if crate::wm::start_menu::launch_by_name(args[1]) {
                    crate::println!("app: launched {}", args[1]);
                } else {
                    crate::println!("app: no such app: {}", args[1]);
                }
            } else {
                crate::println!("app: usage: app <name> (terminal|files|editor|calculator|...)");
            }
            0
        }
        "menu" => {
            #[cfg(target_arch = "aarch64")]
            {
                crate::wm::start_menu::toggle();
                let state = if crate::wm::start_menu::is_open() { "open" } else { "closed" };
                crate::println!("menu: {}", state);
            }
            #[cfg(not(target_arch = "aarch64"))]
            crate::println!("menu: aarch64 only");
            0
        }
        #[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
        "panel" => {
            cmd_panel(args, argc);
            0
        }
        #[cfg(target_arch = "x86_64")]
        "panel" => {
            Uart10::puts("Panel not available on x86_64.\r\n");
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
        // Additional file commands
        "touch" => cmd_touch(args, argc),
        "cp" => cmd_cp(args, argc),
        "head" => cmd_head(args, argc),
        "tail" => cmd_tail(args, argc),
        "wc" => cmd_wc(args, argc),
        "du" => cmd_du(args, argc),
        "df" => cmd_df(),
        "free" => cmd_free(),
        "which" => cmd_which(args, argc),
        "uname" => cmd_uname(),
        "date" => cmd_date(),
        "find" => cmd_find(args, argc),
        // Network commands
        "curl" => cmd_curl(args, argc),
        "wget" => cmd_wget(args, argc),
        "ip" => cmd_ip(args, argc),
        "ifconfig" => cmd_ifconfig(args, argc),
        "ping" => cmd_ping(args, argc),
        "host" => cmd_host(args, argc),
        "netstat" => cmd_netstat(),
        "ipconfig" => cmd_ifconfig(args, argc),
        // User process execution
        "run" => cmd_run(args, argc),
        "cc" => cmd_cc(args, argc),
        // Python: run the embedded MicroPython interpreter directly,
        // with optional script path (python [script.py]).
        "python" | "python3" => cmd_python(args, argc),
        // S3: toggle architectural-timer preemption
        "preempt" => cmd_preempt(args, argc),
        // Text editor
        "vi" | "edit" | "ed" | "nano" | "code" => cmd_editor(args, argc),
        // Text processing
        "grep" => cmd_grep(args, argc),
        "sort" => cmd_sort(args, argc),
        "cut" => cmd_cut(args, argc),
        "tr" => cmd_tr(args, argc),
        "uniq" => cmd_uniq(args, argc),
        "seq" => cmd_seq(args, argc),
        "yes" => cmd_yes(args, argc),
        "tee" => cmd_tee(args, argc),
        // File info
        "stat" => cmd_stat(args, argc),
        "file" => cmd_file(args, argc),
        "ln" => cmd_ln(args, argc),
        "basename" => cmd_basename(args, argc),
        "dirname" => cmd_dirname(args, argc),
        // Misc
        "sleep" => cmd_sleep(args, argc),
        "man" => cmd_man(args, argc),
        "who" => cmd_who(),
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
    #[cfg(not(feature = "qemu_raspi5b"))]
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
    Uart10::puts("  useradd [opts] <user> Add user (-m -p <pass> -s <shell>)\r\n");
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
    Uart10::puts("  ls [-l|-a] [path] List directory contents\r\n");
    Uart10::puts("  cd [path]       Change working directory\r\n");
    Uart10::puts("  pwd             Print working directory\r\n");
    Uart10::puts("  mkdir <path>    Create directory\r\n");
    Uart10::puts("  rm <path>       Remove file or directory\r\n");
    Uart10::puts("  mv <src> <dst>  Move/rename file\r\n");
    Uart10::puts("  cp <src> <dst>  Copy file\r\n");
    Uart10::puts("  touch <file>    Create empty file\r\n");
    Uart10::puts("  chmod <mode> <path> Change file permissions\r\n");
    Uart10::puts("  chown <uid:gid> <path> Change file ownership\r\n");
    Uart10::puts("  cat <path>      Display file contents\r\n");
    Uart10::puts("  head [-n N] <file> Show first N lines of file\r\n");
    Uart10::puts("  tail [-n N] <file> Show last N lines of file\r\n");
    Uart10::puts("  wc <file>       Count lines/words/bytes\r\n");
    Uart10::puts("  du [path]       Estimate file space usage\r\n");
    Uart10::puts("  df              Show filesystem disk space\r\n");
    Uart10::puts("  free            Show free/used memory\r\n");
    Uart10::puts("  find <path> -name <pat> Search for files\r\n");
    Uart10::puts("  curl <url>       Download from URL (HTTP GET)\r\n");
    Uart10::puts("  wget <url>       Download file from URL\r\n");
    Uart10::puts("  ip / ifconfig    Show network interface info\r\n");
    Uart10::puts("  ping <host>      Send ICMP echo to host\r\n");
    Uart10::puts("  host <name>       DNS lookup\r\n");
    Uart10::puts("  netstat          Show network socket status\r\n");
    Uart10::puts("  pkg <cmd>        Package manager (install/remove/list, update/search)\r\n");
    Uart10::puts("  gh <cmd>         GitHub CLI: auth login|status|logout, api <endpoint>, repo list, user\r\n");
    Uart10::puts("  which <cmd>     Locate a command\r\n");
    Uart10::puts("  uname           Show system information\r\n");
    Uart10::puts("  date            Show system time/uptime\r\n");
    Uart10::puts(" run <path|demo> Run ELF binary as user process\r\n");
    Uart10::puts(" cc <file.c>     Compile C source to ELF (via lobster-net)\r\n");
    Uart10::puts(" vi <file>       Simple text editor (line-based)\r\n");
    Uart10::puts(" grep <pat> <file> Search for pattern in file\r\n");
    Uart10::puts(" sort <file>     Sort lines of file\r\n");
    Uart10::puts(" cut <field> <file> Extract fields from lines\r\n");
    Uart10::puts(" tr <set1> <set2> Translate characters\r\n");
    Uart10::puts(" seq <n>         Print number sequence\r\n");
    Uart10::puts(" stat <path>     Show file stats\r\n");
    Uart10::puts(" file <path>     Show file type\r\n");
    Uart10::puts(" ln <src> <dst>  Create hard link\r\n");
    Uart10::puts(" sleep <sec>     Sleep for N seconds\r\n");
    Uart10::puts(" man <cmd>       Show manual for command\r\n");
    Uart10::puts(" who             Show logged-in users\r\n");
    Uart10::puts(" basename <path> Strip directory from path\r\n");
    Uart10::puts(" dirname <path>  Strip filename from path\r\n");
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
    let ms = crate::arch::ArchTimer::uptime_ms();
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
    // stats() returns (total, allocated, first_free); free = total - allocated
    let (total_frames, used_frames, _first_free) = crate::memory::frame_alloc::stats();
    let free_frames = total_frames.saturating_sub(used_frames);
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

fn cmd_echo(args: [&str; MAX_ARGS], argc: usize, out: &ShellOut) -> i32 {
    for i in 1..argc {
        if i > 1 {
            out.write_bytes(b" ");
        }
        out.puts(args[i]);
    }
    out.write_bytes(b"\n");
    0
}

fn cmd_version() {
    Uart10::puts("LobsterOS v0.2.0\r\n");
    Uart10::puts("Target: AArch64 (BCM2712 / Raspberry Pi 5)\r\n");
    Uart10::puts("Kernel: 0x80000000\r\n");
    Uart10::puts("Scheduler: Round-robin preemptive\r\n");
}

#[cfg(target_arch = "aarch64")]
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

#[cfg(not(target_arch = "aarch64"))]
fn cmd_cpu() {
    crate::println!("CPU info not available on this architecture");
}

fn cmd_timer() {
    let freq = crate::arch::ArchTimer::frequency();
    let uptime = crate::arch::ArchTimer::uptime_ms();
    let sys_time = crate::arch::ArchTimer::system_time_us();

    crate::println!("Timer frequency: {} Hz", freq);
    crate::println!("System time: {} us ({} ms)", sys_time, sys_time / 1000);
    crate::println!("Uptime: {} ms", uptime);
}

fn cmd_halt() {
    Uart10::puts("System halting...\r\n");
    #[cfg(target_arch = "aarch64")]
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
                #[cfg(target_arch = "aarch64")]
                crate::drivers::gpio::set_pin_mode(pin, crate::drivers::gpio::PinMode::Output);
                #[cfg(target_arch = "aarch64")]
                crate::drivers::gpio::set_pin(pin);
                crate::println!("GPIO {} set HIGH", pin);
            }
            "low" => {
                #[cfg(target_arch = "aarch64")]
                crate::drivers::gpio::set_pin_mode(pin, crate::drivers::gpio::PinMode::Output);
                #[cfg(target_arch = "aarch64")]
                crate::drivers::gpio::clear_pin(pin);
                crate::println!("GPIO {} set LOW", pin);
            }
            _ => Uart10::puts("Use 'high' or 'low'\r\n"),
        }
    } else {
        #[cfg(target_arch = "aarch64")]
        let level = crate::drivers::gpio::read_pin(pin);
        #[cfg(target_arch = "aarch64")]
        crate::println!("GPIO {} = {}", pin, if level { "HIGH" } else { "LOW" });
        #[cfg(target_arch = "x86_64")]
        crate::println!("GPIO {} = (not available on x86_64)", pin);
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
#[cfg(not(feature = "qemu_raspi5b"))]
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

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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

#[cfg(target_arch = "x86_64")]
fn cmd_ifconfig() {
    crate::println!("Network interfaces not available on x86_64");
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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
    #[cfg(target_arch = "aarch64")]
    crate::net::icmp::send_echo_request(dst, 1);
    // Poll for replies for a few iterations
    for _ in 0..100 {
        #[cfg(target_arch = "aarch64")]
        crate::net::poll();
    }
    crate::println!("PING done.");
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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
    #[cfg(target_arch = "aarch64")]
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
    #[cfg(target_arch = "x86_64")]
    crate::println!("  -> ARP not available on x86_64");
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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
    #[cfg(target_arch = "aarch64")]
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
    #[cfg(target_arch = "x86_64")]
    {
        Uart10::puts("DHCP not available on x86_64\r\n");
    }
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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

#[cfg(not(feature = "qemu_raspi5b"))]
fn cmd_dns(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: dns <domain>\r\n");
        Uart10::puts("Example: dns example.com\r\n");
        return;
    }
    let domain = args[1];
    crate::println!("Resolving {} via DNS...", domain);
    #[cfg(target_arch = "aarch64")]
    match crate::net::dns::resolve(domain) {
        Some(ip) => crate::println!("{} -> {}.{}.{}.{}", domain, ip[0], ip[1], ip[2], ip[3]),
        None => crate::println!("Failed to resolve {}", domain),
    }
    #[cfg(target_arch = "x86_64")]
    crate::println!("DNS not available on x86_64");
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
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
#[cfg(not(feature = "qemu_raspi5b"))]
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

#[cfg(not(feature = "qemu_raspi5b"))]
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

#[cfg(not(feature = "qemu_raspi5b"))]
fn cmd_gpu(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: gpu <info|status|clock|blank|mode>\r\n");
        return;
    }
    match args[1] {
        "info" => {
            #[cfg(target_arch = "aarch64")]
            let info = crate::drivers::gpu::gpu_info();
            crate::println!("GPU Info:");
            #[cfg(target_arch = "aarch64")]
            crate::println!(" Firmware: {:#010x}", info.firmware_rev);
            #[cfg(target_arch = "aarch64")]
            crate::println!(
                " Board: model={:#x} rev={:#x}",
                info.board_model,
                info.board_rev
            );
            #[cfg(target_arch = "aarch64")]
            crate::println!(
                " VC mem: {:#x} ({} MB)",
                info.vc_mem_base,
                info.vc_mem_size / (1024 * 1024)
            );
            #[cfg(target_arch = "aarch64")]
            crate::println!(
                " ARM mem: {:#x} ({} MB)",
                info.arm_mem_base,
                info.arm_mem_size / (1024 * 1024)
            );
            #[cfg(target_arch = "aarch64")]
            crate::println!(" V3D clock: {} MHz", info.v3d_clock / 1_000_000);
            #[cfg(target_arch = "aarch64")]
            crate::println!(" Core clock: {} MHz", info.core_clock / 1_000_000);
            #[cfg(target_arch = "x86_64")]
            crate::println!(" GPU info not available on x86_64");
        }
        "status" => {
            #[cfg(target_arch = "aarch64")]
            crate::drivers::gpu::print_status();
            #[cfg(target_arch = "x86_64")]
            crate::println!(" GPU status not available on x86_64");
        }
        "clock" => {
            if argc < 3 {
                Uart10::puts("Usage: gpu clock <v3d|core|pixel|emmc>\r\n");
                return;
            }
            match args[2] {
                "v3d" => {
                    #[cfg(target_arch = "aarch64")]
                    let clock = crate::drivers::gpu::get_v3d_clock() / 1_000_000;
                    #[cfg(target_arch = "x86_64")]
                    let clock = 0;
                    crate::println!("V3D clock: {} MHz", clock);
                }
                "core" => {
                    #[cfg(target_arch = "aarch64")]
                    let clock = crate::drivers::gpu::get_core_clock() / 1_000_000;
                    #[cfg(target_arch = "x86_64")]
                    let clock = 0;
                    crate::println!("Core clock: {} MHz", clock);
                }
                "pixel" => {
                    #[cfg(target_arch = "aarch64")]
                    let clock = crate::drivers::gpu::get_pixel_clock() / 1_000_000;
                    #[cfg(target_arch = "x86_64")]
                    let clock = 0;
                    crate::println!("Pixel clock: {} MHz", clock);
                }
                "emmc" => {
                    #[cfg(target_arch = "aarch64")]
                    let clock = crate::drivers::gpu::get_emmc_clock() / 1_000_000;
                    #[cfg(target_arch = "x86_64")]
                    let clock = 0;
                    crate::println!("eMMC clock: {} MHz", clock);
                }
                _ => Uart10::puts("Unknown clock. Use: v3d, core, pixel, emmc\r\n"),
            }
        }
        "blank" => {
            #[cfg(target_arch = "aarch64")]
            crate::drivers::gpu::blank_screen(true);
            Uart10::puts("Screen blanked.\r\n");
            #[cfg(target_arch = "x86_64")]
            Uart10::puts("Screen blank not available on x86_64.\r\n");
        }
        "mode" => {
            if argc < 5 {
                Uart10::puts("Usage: gpu mode <width> <height> <bpp>\r\n");
                return;
            }
            let w = parse_u32(args[2]).unwrap_or(640);
            let h = parse_u32(args[3]).unwrap_or(480);
            let bpp = parse_u32(args[4]).unwrap_or(32);
            #[cfg(target_arch = "aarch64")]
            if crate::drivers::gpu::configure_display(w, h, bpp, true) {
                crate::println!("Mode set: {}x{} @ {}bpp", w, h, bpp);
            } else {
                Uart10::puts("Failed to set mode.\r\n");
            }
            #[cfg(target_arch = "x86_64")]
            Uart10::puts("Display mode not available on x86_64.\r\n");
        }
        _ => Uart10::puts("Usage: gpu <info|status|clock|blank|mode>\r\n"),
    }
}

#[cfg(all(not(feature = "qemu"), target_arch = "aarch64"))]
fn cmd_wifi(args: [&str; MAX_ARGS], argc: usize) {
    if argc < 2 {
        Uart10::puts("Usage: wifi <scan|connect|save|disconnect|status|bg_scan|roam_threshold|power_save|rssi>\r\n");
        Uart10::puts("  wifi scan                    - Scan for available networks\r\n");
        Uart10::puts("  wifi connect <ssid> [pass]   - Connect to network (WPA2/WPA3)\r\n");
        Uart10::puts("  wifi save <ssid> [pass]      - Save network for auto-connect\r\n");
        Uart10::puts("  wifi disconnect              - Disconnect from current network\r\n");
        Uart10::puts("  wifi status                  - Show connection status\r\n");
        Uart10::puts("  wifi bg_scan <on|off> [ms]   - Enable/disable background scan\r\n");
        Uart10::puts("  wifi roam_threshold <db>     - Set roam threshold in dB\r\n");
        Uart10::puts("  wifi power_save <on|off>     - Enable/disable WMM power save\r\n");
        Uart10::puts("  wifi rssi                    - Show current AP RSSI\r\n");
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
                    // Wait for 4-way handshake to complete (link_up = true)
                    Uart10::puts("Waiting for handshake...\r\n");
                    let mut handshake_timeout = 5_000_000u64; // ~50 seconds
                    while !crate::drivers::wifi::link_up() {
                        if handshake_timeout == 0 {
                            Uart10::puts("Handshake timeout\r\n");
                            break;
                        }
                        handshake_timeout -= 1;
                        crate::arch::cpu::delay_us(10);
                        crate::drivers::wifi::poll();
                    }
                    if crate::drivers::wifi::link_up() {
                        crate::println!("Handshake complete, link up!");
                    }
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
            // Save network for auto-connect on boot
            if let Err(e) = crate::drivers::wifi::save_network(ssid, password) {
                crate::println!("Warning: failed to save network: {:?}", e);
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
        "bg_scan" => {
            if argc < 3 {
                Uart10::puts("Usage: wifi bg_scan <on|off> [interval_ms]\r\n");
                return;
            }
            let enabled = match args[2] {
                "on" | "enable" | "true" | "1" => true,
                "off" | "disable" | "false" | "0" => false,
                _ => {
                    Uart10::puts("Invalid value. Use 'on' or 'off'.\r\n");
                    return;
                }
            };
            crate::drivers::wifi::wifi_enable_background_scan(enabled);
            if enabled && argc >= 4 {
                if let Ok(interval) = args[3].parse::<u32>() {
                    crate::drivers::wifi::wifi_set_bg_scan_interval(interval);
                }
            }
        }
        "roam_threshold" => {
            if argc < 3 {
                Uart10::puts("Usage: wifi roam_threshold <db>\r\n");
                return;
            }
            if let Ok(threshold) = args[2].parse::<i8>() {
                crate::drivers::wifi::wifi_set_roam_threshold(threshold);
            } else {
                Uart10::puts("Invalid threshold value.\r\n");
            }
        }
        "power_save" => {
            if argc < 3 {
                Uart10::puts("Usage: wifi power_save <on|off>\r\n");
                return;
            }
            let enabled = match args[2] {
                "on" | "enable" | "true" | "1" => true,
                "off" | "disable" | "false" | "0" => false,
                _ => {
                    Uart10::puts("Invalid value. Use 'on' or 'off'.\r\n");
                    return;
                }
            };
            match crate::drivers::wifi::wifi_set_power_save(enabled) {
                Ok(()) => crate::println!("Power save {}", if enabled { "enabled" } else { "disabled" }),
                Err(e) => crate::println!("Failed to set power save: {:?}", e),
            }
        }
        "rssi" => {
            let rssi = crate::drivers::wifi::wifi_current_ap_rssi();
            crate::println!("Current AP RSSI: {} dBm", rssi);
        }
        _ => Uart10::puts("Usage: wifi <scan|connect|save|disconnect|status|bg_scan|roam_threshold|power_save|rssi>\r\n"),
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
    let username = args[1];

    // Prompt for password with echo disabled using blocking read
    Uart10::puts("Password: ");
    let old_echo = crate::drivers::serial_input::echo_enabled();
    crate::drivers::serial_input::set_echo(false);
    let mut passwd_buf = [0u8; 128];
    let passwd_len = crate::drivers::serial_input::read_line_blocking(&mut passwd_buf);
    crate::drivers::serial_input::set_echo(old_echo);
    Uart10::puts("\r\n");

    let password = core::str::from_utf8(&passwd_buf[..passwd_len]).unwrap_or("");
    let password = password.trim();

    // Verify credentials
    match crate::users::auth::login_user(username, password) {
        Ok(user) => {
            crate::println!("[login] Welcome, {}!", user.username);

            // Allocate session
            let tty = "tty1";
            // PID from scheduler
            let pid = crate::sched::current_pid();
            match crate::users::session::allocate_session(
                crate::users::session::SessionType::Login,
                tty,
                &user.username,
                pid,
            ) {
                Ok(_) => crate::println!("[login] Session started on {}", tty),
                Err(e) => crate::println!("[login] Warning: failed to allocate session: {}", e),
            }
            0
        }
        Err(e) => {
            crate::println!("[login] Login failed: {}", e);
            1
        }
    }
}

/// Switch user command (su)
fn cmd_su(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let target_user = if argc >= 2 { args[1] } else { "root" };

    // Prompt for target user's password with echo disabled using blocking read
    crate::println!("[su] Switching to user: {}", target_user);
    Uart10::puts("Password: ");
    let old_echo = crate::drivers::serial_input::echo_enabled();
    crate::drivers::serial_input::set_echo(false);
    let mut passwd_buf = [0u8; 128];
    let passwd_len = crate::drivers::serial_input::read_line_blocking(&mut passwd_buf);
    crate::drivers::serial_input::set_echo(old_echo);
    Uart10::puts("\r\n");

    let password = core::str::from_utf8(&passwd_buf[..passwd_len]).unwrap_or("");
    let password = password.trim();

    // Verify credentials for target user
    match crate::users::auth::login_user(target_user, password) {
        Ok(user) => {
            crate::println!("[su] Switched to user: {}", user.username);

            // Allocate new session for target user
            let tty = "tty1";
            let pid = crate::sched::current_pid();
            match crate::users::session::allocate_session(
                crate::users::session::SessionType::Login,
                tty,
                &user.username,
                pid,
            ) {
                Ok(_) => crate::println!("[su] Session started on {}", tty),
                Err(e) => crate::println!("[su] Warning: failed to allocate session: {}", e),
            }
            0
        }
        Err(e) => {
            crate::println!("[su] Authentication failed: {}", e);
            1
        }
    }
}

/// Sudo command
fn cmd_sudo(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: sudo <command> [args...]\r\n");
        return 1;
    }

    // Get current user from session (simplified - would need actual session tracking)
    let current_user = "root"; // TODO: get from session

    // Check if current user can sudo
    if !crate::users::auth::can_sudo(current_user) {
        crate::println!(
            "[sudo] User '{}' is not authorized to use sudo",
            current_user
        );
        return 1;
    }

    let cmd = args[1];
    let target_user = if argc >= 3 && args[2] == "-u" {
        if argc >= 4 {
            args[3]
        } else {
            Uart10::puts("Usage: sudo [-u user] <command> [args...]\r\n");
            return 1;
        }
    } else {
        "root"
    };

    // Get target UID
    let target_uid = match crate::users::auth::sudo_uid(current_user, Some(target_user)) {
        Ok(uid) => uid,
        Err(e) => {
            crate::println!("[sudo] Failed to get target UID: {}", e);
            return 1;
        }
    };

    crate::println!(
        "[sudo] Running as user '{}' (UID {}): {}",
        target_user,
        target_uid,
        cmd
    );

    // Build command line from remaining args
    let mut cmd_line = String::new();
    let start_idx = if argc >= 3 && args[2] == "-u" { 4 } else { 2 };
    for i in start_idx..argc {
        if i > start_idx {
            cmd_line.push(' ');
        }
        cmd_line.push_str(args[i]);
    }

    // Execute command (simplified - would need process spawning with UID)
    crate::println!(
        "[sudo] Command execution not fully implemented - would spawn process with UID {}",
        target_uid
    );
    crate::println!("[sudo] Would execute: {}", cmd_line);
    0
}

/// Passwd command — set or change a user's password
fn cmd_passwd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    // Default to root if no user specified (initial setup scenario)
    let target_user = if argc >= 2 { args[1] } else { "root" };

    // Check if target user exists
    if !crate::users::passwd::user_exists(target_user) {
        crate::println!("passwd: user '{}' does not exist", target_user);
        crate::println!("passwd: use 'useradd -m <username>' to create a new user first");
        return 1;
    }

    crate::println!("Changing password for {}", target_user);

    // Prompt for new password using blocking read
    Uart10::puts("New password: ");
    let saved_echo = crate::drivers::serial_input::echo_enabled();
    crate::drivers::serial_input::set_echo(false);
    let mut new_passwd_buf = [0u8; 128];
    let new_passwd_len = crate::drivers::serial_input::read_line_blocking(&mut new_passwd_buf);
    crate::drivers::serial_input::set_echo(saved_echo);

    let new_password =
        core::str::from_utf8(&new_passwd_buf[..new_passwd_len]).unwrap_or("").trim();

    // Confirm new password
    Uart10::puts("Retype new password: ");
    crate::drivers::serial_input::set_echo(false);
    let mut confirm_passwd_buf = [0u8; 128];
    let confirm_passwd_len =
        crate::drivers::serial_input::read_line_blocking(&mut confirm_passwd_buf);
    crate::drivers::serial_input::set_echo(saved_echo);

    let confirm_password =
        core::str::from_utf8(&confirm_passwd_buf[..confirm_passwd_len])
            .unwrap_or("")
            .trim();

    if new_password != confirm_password {
        crate::println!("passwd: passwords do not match");
        return 1;
    }

    if new_password.is_empty() {
        crate::println!("passwd: password cannot be empty");
        return 1;
    }

    // Change password directly (bypass old password check for admin)
    match crate::users::auth::set_password(target_user, new_password) {
        Ok(()) => {
            crate::println!(
                "passwd: password updated successfully for '{}'",
                target_user
            );
            0
        }
        Err(e) => {
            crate::println!("passwd: failed to update password: {}", e);
            1
        }
    }
}

/// Useradd command
fn cmd_useradd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: useradd [options] <username>\r\n");
        Uart10::puts("  -m              Create home directory\r\n");
        Uart10::puts("  -s <shell>      Login shell (default /bin/sh)\r\n");
        Uart10::puts("  -p <password>   Set password (plaintext, stored hashed)\r\n");
        Uart10::puts("  -u <uid>        Specify UID manually\r\n");
        Uart10::puts("  -g <gid>        Primary GID (default = new GID matching UID)\r\n");
        Uart10::puts("  --help          Show this help\r\n");
        Uart10::puts("\r\nExample: useradd -m -p secret alice\r\n");
        return 1;
    }

    // Parse flags
    let mut username: Option<&str> = None;
    let mut create_home = false;
    let mut shell = "/bin/sh";
    let mut password: Option<&str> = None;
    let mut custom_uid: Option<u32> = None;
    let mut custom_gid: Option<u32> = None;
    let mut i = 1;
    while i < argc {
        match args[i] {
            "-m" => {
                create_home = true;
            }
            "-s" => {
                if i + 1 < argc {
                    shell = args[i + 1];
                    i += 1;
                }
            }
            "-p" => {
                if i + 1 < argc {
                    password = Some(args[i + 1]);
                    i += 1;
                }
            }
            "-u" => {
                if i + 1 < argc {
                    custom_uid = parse_u32(args[i + 1]);
                    i += 1;
                }
            }
            "-g" => {
                if i + 1 < argc {
                    custom_gid = parse_u32(args[i + 1]);
                    i += 1;
                }
            }
            "--help" => {
                Uart10::puts("Usage: useradd [options] <username>\r\n");
                return 0;
            }
            arg if !arg.starts_with('-') => {
                username = Some(arg);
            }
            _ => {
                crate::println!("useradd: unknown option '{}'", args[i]);
                return 1;
            }
        }
        i += 1;
    }

    let username = match username {
        Some(u) => u,
        None => {
            Uart10::puts("useradd: missing username\r\n");
            return 1;
        }
    };

    // Validate username
    if username.is_empty() || username.len() > 32 {
        Uart10::puts("useradd: invalid username length\r\n");
        return 1;
    }
    for c in username.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            crate::println!("useradd: invalid character '{}' in username", c);
            return 1;
        }
    }

    // Check if user already exists
    if crate::users::passwd::user_exists(username) {
        crate::println!("useradd: user '{}' already exists", username);
        return 1;
    }

    // Find next available UID (start from 1000, skip system UIDs)
    let next_uid = if let Some(uid) = custom_uid {
        // Check it's not already taken
        let users = crate::users::passwd::get_all_users();
        for user in &users {
            if user.uid == uid {
                crate::println!("useradd: UID {} already in use", uid);
                return 1;
            }
        }
        uid
    } else {
        let mut uid = 1000;
        let users = crate::users::passwd::get_all_users();
        for user in &users {
            // Only consider UIDs in the normal user range (1000+)
            if user.uid >= uid && user.uid < 65534 {
                uid = user.uid + 1;
            }
        }
        uid
    };

    // Find next available GID (start from 1000, skip system GIDs)
    let next_gid = if let Some(gid) = custom_gid {
        gid
    } else {
        let mut gid = 1000;
        let groups = crate::users::group::get_all_groups();
        for group in &groups {
            if group.gid >= gid && group.gid < 65534 {
                gid = group.gid + 1;
            }
        }
        gid
    };

    // Create user entry
    let home = alloc::format!("/home/{}", username);
    let user_entry =
        crate::users::passwd::PasswdEntry::new(username, next_uid, next_gid, &home, shell);

    // Create shadow entry — use provided password or locked ("!")
    let shadow_password = password.unwrap_or("!");
    let shadow_entry = crate::users::shadow::ShadowEntry::new(username, shadow_password);

    // Create user's primary group
    let group_entry = crate::users::group::GroupEntry::new(username, next_gid);

    // Write passwd file
    let mut users = crate::users::passwd::read_passwd_file().unwrap_or_default();
    users.push(user_entry);
    if let Err(e) = crate::users::passwd::write_passwd_file(&users) {
        crate::println!("useradd: failed to write /etc/passwd: {:?}", e);
        return 1;
    }

    // Write shadow file
    let mut shadows = crate::users::shadow::read_shadow_file().unwrap_or_default();
    shadows.push(shadow_entry);
    if let Err(e) = crate::users::shadow::write_shadow_file(&shadows) {
        crate::println!("useradd: failed to write /etc/shadow: {:?}", e);
        return 1;
    }

    // Write group file
    let mut groups = crate::users::group::read_group_file().unwrap_or_default();
    groups.push(group_entry);
    if let Err(e) = crate::users::group::write_group_file(&groups) {
        crate::println!("useradd: failed to write /etc/group: {:?}", e);
        return 1;
    }

    // Create home directory if requested or by default
    let _ = crate::fs::vfs::create_dir("/home");
    let _ = crate::fs::vfs::create_dir(&alloc::format!("/home/{}", username));

    if create_home {
        crate::println!(
            "useradd: created user '{}' (UID={}, GID={}) with home /home/{}",
            username,
            next_uid,
            next_gid,
            username
        );
    } else {
        crate::println!(
            "useradd: created user '{}' (UID={}, GID={})",
            username,
            next_uid,
            next_gid
        );
    }

    if password.is_some() {
        crate::println!("useradd: password set for '{}'", username);
    } else {
        crate::println!(
            "useradd: account locked - use 'passwd {}' to set password",
            username
        );
    }

    0
}

/// Userdel command
fn cmd_userdel(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: userdel <username>\r\n");
        return 1;
    }

    let username = args[1];

    // Check if user exists
    if !crate::users::passwd::user_exists(username) {
        crate::println!("[userdel] User '{}' does not exist", username);
        return 1;
    }

    // Remove from passwd
    let mut users = crate::users::passwd::read_passwd_file().unwrap_or_default();
    let user_idx = users.iter().position(|u| u.username == username);
    if let Some(idx) = user_idx {
        users.remove(idx);
        if let Err(e) = crate::users::passwd::write_passwd_file(&users) {
            crate::println!("[userdel] Failed to write /etc/passwd: {:?}", e);
            return 1;
        }
    }

    // Remove from shadow
    let mut shadows = crate::users::shadow::read_shadow_file().unwrap_or_default();
    let shadow_idx = shadows.iter().position(|s| s.username == username);
    if let Some(idx) = shadow_idx {
        shadows.remove(idx);
        if let Err(e) = crate::users::shadow::write_shadow_file(&shadows) {
            crate::println!("[userdel] Failed to write /etc/shadow: {:?}", e);
            return 1;
        }
    }

    // Remove from group (user's primary group and memberships)
    let mut groups = crate::users::group::read_group_file().unwrap_or_default();
    // Remove user from group memberships
    for group in &mut groups {
        group.remove_member(username);
    }
    // Remove user's primary group if it exists and has no other members
    let user_primary_group_idx = groups.iter().position(|g| g.name == username);
    if let Some(idx) = user_primary_group_idx {
        // Only remove if no other members
        if groups[idx].members.is_empty() {
            groups.remove(idx);
        }
    }
    if let Err(e) = crate::users::group::write_group_file(&groups) {
        crate::println!("[userdel] Failed to write /etc/group: {:?}", e);
        return 1;
    }

    // Remove home directory (optional - would need recursive rm)
    // let _ = crate::fs::vfs::remove(&alloc::format!("/home/{}", username));

    crate::println!("[userdel] User '{}' deleted", username);
    0
}

/// Groupadd command
fn cmd_groupadd(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: groupadd <groupname>\r\n");
        return 1;
    }

    let groupname = args[1];

    // Check if group already exists
    if crate::users::group::group_exists(groupname) {
        crate::println!("[groupadd] Group '{}' already exists", groupname);
        return 1;
    }

    // Find next available GID (start from 1000)
    let mut next_gid = 1000;
    let groups = crate::users::group::get_all_groups();
    for group in &groups {
        if group.gid >= next_gid {
            next_gid = group.gid + 1;
        }
    }

    // Create group entry
    let group_entry = crate::users::group::GroupEntry::new(groupname, next_gid);

    // Add to group
    let mut groups = crate::users::group::read_group_file().unwrap_or_default();
    groups.push(group_entry);
    if let Err(e) = crate::users::group::write_group_file(&groups) {
        crate::println!("[groupadd] Failed to write /etc/group: {:?}", e);
        return 1;
    }

    crate::println!("[groupadd] Group '{}' added (GID={})", groupname, next_gid);
    0
}

/// Groupdel command
fn cmd_groupdel(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: groupdel <groupname>\r\n");
        return 1;
    }

    let groupname = args[1];

    // Check if group exists
    if !crate::users::group::group_exists(groupname) {
        crate::println!("[groupdel] Group '{}' does not exist", groupname);
        return 1;
    }

    // Check if group is a user's primary group
    let users = crate::users::passwd::get_all_users();
    for user in &users {
        if user.gid
            == crate::users::group::find_group(groupname)
                .map(|g| g.gid)
                .unwrap_or(0)
        {
            crate::println!(
                "[groupdel] Cannot delete group '{}': it is the primary group of user '{}'",
                groupname,
                user.username
            );
            return 1;
        }
    }

    // Remove from group
    let mut groups = crate::users::group::read_group_file().unwrap_or_default();
    let group_idx = groups.iter().position(|g| g.name == groupname);
    if let Some(idx) = group_idx {
        groups.remove(idx);
        if let Err(e) = crate::users::group::write_group_file(&groups) {
            crate::println!("[groupdel] Failed to write /etc/group: {:?}", e);
            return 1;
        }
    }

    crate::println!("[groupdel] Group '{}' deleted", groupname);
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
    #[cfg(target_arch = "aarch64")]
    crate::drivers::power::reset();
}

// ═══════════════════════════════════════════════
// Filesystem command handlers
// ═══════════════════════════════════════════════

static mut CWD: [u8; 256] = [b'/'; 256];
static mut CWD_LEN: usize = 1;

pub fn cwd() -> &'static str {
    unsafe {
        if CWD_LEN == 0 || CWD[0] == 0 {
            CWD[0] = b'/';
            CWD_LEN = 1;
        }
        core::str::from_utf8(&CWD[..CWD_LEN]).unwrap_or("/")
    }
}

pub fn set_cwd(path: &str) {
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
    let mut long_format = false;
    let mut show_all = false;
    let mut path_args: alloc::vec::Vec<&str> = alloc::vec::Vec::new();

    let mut i = 1;
    while i < argc {
        if args[i].starts_with('-') && args[i].len() > 1 {
            for c in args[i][1..].chars() {
                match c {
                    'l' => long_format = true,
                    'a' => show_all = true,
                    _ => {}
                }
            }
        } else {
            path_args.push(args[i]);
        }
        i += 1;
    }

    let paths: alloc::vec::Vec<alloc::string::String> = if path_args.is_empty() {
        alloc::vec![alloc::string::String::from(cwd())]
    } else {
        path_args.iter().map(|p| resolve_path(p)).collect()
    };

    let mut total_entries = 0;
    for path in &paths {
        match crate::fs::vfs::list_dir(path) {
            Ok(entries) => {
                let filtered: alloc::vec::Vec<_> = entries
                    .iter()
                    .filter(|e| show_all || !e.name_str().starts_with('.'))
                    .collect();

                if paths.len() > 1 {
                    crate::println!("\n{}:", path);
                }

                if long_format {
                    crate::println!("total {}", filtered.len());
                    for entry in &filtered {
                        let type_char = if entry.is_dir { 'd' } else { '-' };
                        let dir_marker = if entry.is_dir { "/" } else { "" };
                        crate::println!(
                            "{}rw-r--r--  {:4} {}:{} {}{}",
                            type_char,
                            entry.size,
                            entry.uid,
                            entry.gid,
                            entry.name_str(),
                            dir_marker
                        );
                    }
                } else {
                    // Simple listing — just names, columns
                    let mut line = alloc::string::String::new();
                    for entry in &filtered {
                        let name = if entry.is_dir {
                            alloc::format!("{}/  ", entry.name_str())
                        } else {
                            alloc::format!("{}   ", entry.name_str())
                        };
                        line.push_str(&name);
                        if line.len() > 60 {
                            crate::println!("{}", line.trim_end());
                            line.clear();
                        }
                    }
                    if !line.is_empty() {
                        crate::println!("{}", line.trim_end());
                    }
                }
                total_entries += filtered.len();
            }
            Err(e) => {
                crate::println!("ls: {}: error {:?}", path, e);
            }
        }
    }
    if total_entries == 0 && !long_format {
        // Empty directory - print nothing (like real ls)
    }
    0
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

/// `preempt on|off|status` — S3: control the architectural-timer
/// (PPI 30) preemptive tick. `on` enables the GIC PPI + arms CNTP with
/// the tick interval and unmasked DAIF.I.
fn cmd_preempt(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    #[cfg(all(target_arch = "aarch64", feature = "qemu_raspi5b"))]
    {
        unsafe {
            let action = if argc >= 2 { args[1] } else { "status" };
            match action {
                "on" => {
                    // bcm2712 wiring: CNTP (EL1 physical timer) → PPI 29.
                    // Enable + priority + edge config.
                    crate::arch::gic::Gic::enable_ppi(29);
                    crate::arch::gic::Gic::set_priority(29, 0x00);
                    crate::arch::gic::Gic::set_config(29, 0b10);
                    // CRITICAL: QEMU's GIC resets PMR=0 which blocks ALL
                    // delivery (prio < PMR required). Allow all.
                    crate::arch::gic::Gic::set_priority_mask(0xFF);
                    if argc >= 3 && args[2] == "virt" {
                        // Virtual timer (CNTV, PPI 30) — Linux's choice.
                        crate::arch::gic::Gic::enable_ppi(30);
                        crate::arch::gic::Gic::set_priority(30, 0x00);
                        crate::arch::gic::Gic::set_config(30, 0b10);
                        crate::arch::gic::Gic::set_priority_mask(0xFF);
                        crate::arch::timer::Timer::enable_virt_timer(10);
                        crate::sched::set_preempt(true);
                        crate::arch::cpu::enable_irqs();
                        Uart10::puts("preempt: VIRTUAL timer ON (PPI 30, 10ms)\r\n");
                    } else if argc >= 3 && args[2] == "notimer" {
                        // GIC + PMR only — for isolating vector issues from
                        // the arch timer itself.
                        Uart10::puts("preempt: GIC+PMR on, timer NOT armed\r\n");
                    } else {
                        // Arm the EL1 physical timer, 10ms period
                        crate::arch::timer::Timer::enable_arch_timer(10);
                        Uart10::puts("preempt: architectural timer ON (PPI 29, 10ms)\r\n");
                    }
                    // Unmask IRQs + record mode for the main loop
                    crate::sched::set_preempt(true);
                    crate::arch::cpu::enable_irqs();
                }
                "off" => {
                    // Mask the timer IRQ source: CTL Imask=1, Enable=0
                    core::arch::asm!("msr CNTP_CTL_EL0, xzr");
                    crate::sched::set_preempt(false);
                    Uart10::puts("preempt: architectural timer OFF\r\n");
                }
                _ => {
                    let ctl: u64;
                    core::arch::asm!("mrs {}, CNTP_CTL_EL0", out(reg) ctl);
                    let isen0 = crate::arch::gic::Gic::dist_read32(0x100);
                    crate::println!(
                        "preempt: CNTFRQ={} CTL={:#x} ISENABLER0={:#010x} (PPI29={})",
                        crate::arch::timer::Timer::cntfrq(),
                        ctl,
                        isen0,
                        (isen0 >> 29) & 1
                    );
                }
            }
        }
        0
    }
    #[cfg(not(all(target_arch = "aarch64", feature = "qemu_raspi5b")))]
    {
        let _ = (args, argc);
        Uart10::puts("preempt: not available on this build\r\n");
        1
    }
}

/// `python` / `python3` — run the embedded MicroPython interpreter.
/// v1: interactive REPL (serial stdin). A script argument is planned
/// once argv passing is wired into spawn (see C8 in the desktop plan).
fn cmd_python(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc >= 2 {
        Uart10::puts("python: scripts coming soon (argv not wired); starting REPL\r\n");
    }
    match crate::sched::spawn_from_path("/bin/python", crate::sched::Priority::Normal) {
        Some(pid) => {
            crate::println!("python: MicroPython REPL spawned as PID {}", pid);
            let exit_code = unsafe { crate::sched::run_and_wait(pid as u64) };
            crate::println!("python: interpreter exited with code {}", exit_code);
            0
        }
        None => {
            Uart10::puts("python: /bin/python not found\r\n");
            1
        }
    }
}

fn cmd_run(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: run <path|demo> [priority] [&]\r\n");
        Uart10::puts("  run demo       Run built-in hello-EL0 test binary\r\n");
        Uart10::puts("  run <path>     Load ELF from VFS and spawn as user process\r\n");
        Uart10::puts("  run <path> &   Spawn in the background — shell stays live\r\n");
        return 1;
    }

    // `run <path> &` — background (sliced) execution.
    let bg = args[argc - 1] == "&";
    let argc_eff = if bg { argc - 1 } else { argc };
    // The tokenizer strips a trailing `&` into BG_PENDING (see
    // execute pipeline); honor it for both run forms.
    let bg = bg || BG_PENDING.swap(false, core::sync::atomic::Ordering::Relaxed);

    let priority = if argc_eff >= 3 {
        match args[2] {
            "high" | "hi" => crate::sched::Priority::High,
            "low" => crate::sched::Priority::Low,
            _ => crate::sched::Priority::Normal,
        }
    } else {
        crate::sched::Priority::Normal
    };

    if bg {
        if args[1] == "demo" {
            match crate::sched::spawn_demo() {
                Some(pid) => {
                    crate::sched::mark_bg(pid);
                    crate::println!("run: hello-EL0 demo spawned as PID {} (background)", pid);
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
                    crate::sched::mark_bg(pid);
                    crate::println!("run: {} spawned as PID {} (background)", path, pid);
                    0
                }
                None => {
                    crate::println!("run: failed to spawn {}", path);
                    1
                }
            }
        }
    } else if args[1] == "demo" {
        match crate::sched::spawn_demo() {
            Some(pid) => {
                crate::println!("run: hello-EL0 demo spawned as PID {}", pid);
                let exit_code = unsafe { crate::sched::run_and_wait(pid as u64) };
                crate::println!("run: process exited with code {}", exit_code);
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
                let exit_code = unsafe { crate::sched::run_and_wait(pid as u64) };
                crate::println!("run: process exited with code {}", exit_code);
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

// === Additional file/system commands ===

/// touch - create empty file or update timestamp
fn cmd_touch(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: touch <file>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);
    match crate::fs::vfs::open(&path, crate::fs::vfs::O_WRONLY | crate::fs::vfs::O_CREAT) {
        Ok(fd) => {
            crate::fs::vfs::close(fd).ok();
            0
        }
        Err(e) => {
            crate::println!("touch: {}: error {:?}", path, e);
            1
        }
    }
}

/// cp - copy file
fn cmd_cp(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: cp <src> <dst>\r\n");
        return 1;
    }
    let src = resolve_path(args[1]);
    let dst = resolve_path(args[2]);

    // Read source
    let data = match crate::fs::vfs::read_file(&src) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("cp: {}: error {:?}", src, e);
            return 1;
        }
    };

    // Write destination
    let fd = match crate::fs::vfs::open(
        &dst,
        crate::fs::vfs::O_WRONLY | crate::fs::vfs::O_CREAT | crate::fs::vfs::O_TRUNC,
    ) {
        Ok(fd) => fd,
        Err(e) => {
            crate::println!("cp: {}: error {:?}", dst, e);
            return 1;
        }
    };

    if let Err(e) = crate::fs::vfs::write(fd, &data) {
        crate::println!("cp: write error: {:?}", e);
        crate::fs::vfs::close(fd).ok();
        return 1;
    }
    crate::fs::vfs::close(fd).ok();
    0
}

/// head - display first N lines of a file
fn cmd_head(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let mut n: usize = 10;
    let mut path_idx: Option<usize> = None;

    let mut i = 1;
    while i < argc {
        if (args[i] == "-n" || args[i] == "--lines") && i + 1 < argc {
            n = parse_usize(args[i + 1]).unwrap_or(10);
            i += 2;
            continue;
        }
        if let Some(rest) = args[i].strip_prefix("-n") {
            n = parse_usize(rest).unwrap_or(10);
            i += 1;
            continue;
        }
        path_idx = Some(i);
        i += 1;
    }

    let idx = match path_idx {
        Some(i) => i,
        None => {
            Uart10::puts("Usage: head [-n N] <file>\r\n");
            return 1;
        }
    };

    let path = resolve_path(args[idx]);
    match crate::fs::vfs::read_file(&path) {
        Ok(data) => {
            let text = core::str::from_utf8(&data).unwrap_or("");
            let mut count = 0;
            for line in text.lines() {
                if count >= n {
                    break;
                }
                crate::println!("{}", line);
                count += 1;
            }
            0
        }
        Err(e) => {
            crate::println!("head: {}: error {:?}", path, e);
            1
        }
    }
}

/// tail - display last N lines of a file
fn cmd_tail(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let mut n: usize = 10;
    let mut path_idx: Option<usize> = None;

    let mut i = 1;
    while i < argc {
        if (args[i] == "-n" || args[i] == "--lines") && i + 1 < argc {
            n = parse_usize(args[i + 1]).unwrap_or(10);
            i += 2;
            continue;
        }
        if let Some(rest) = args[i].strip_prefix("-n") {
            n = parse_usize(rest).unwrap_or(10);
            i += 1;
            continue;
        }
        path_idx = Some(i);
        i += 1;
    }

    let idx = match path_idx {
        Some(i) => i,
        None => {
            Uart10::puts("Usage: tail [-n N] <file>\r\n");
            return 1;
        }
    };

    let path = resolve_path(args[idx]);
    match crate::fs::vfs::read_file(&path) {
        Ok(data) => {
            let text = core::str::from_utf8(&data).unwrap_or("");
            let lines: alloc::vec::Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(n);
            for line in &lines[start..] {
                crate::println!("{}", line);
            }
            0
        }
        Err(e) => {
            crate::println!("tail: {}: error {:?}", path, e);
            1
        }
    }
}

/// wc - count lines, words, and bytes
fn cmd_wc(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: wc <file>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);
    match crate::fs::vfs::read_file(&path) {
        Ok(data) => {
            let text = core::str::from_utf8(&data).unwrap_or("");
            let lines = text.lines().count();
            let words = text.split_whitespace().count();
            let bytes = data.len();
            crate::println!("{:4} {:4} {:4} {}", lines, words, bytes, args[1]);
            0
        }
        Err(e) => {
            crate::println!("wc: {}: error {:?}", path, e);
            1
        }
    }
}

/// du - estimate file space usage (returns file sizes in this simple VFS)
fn cmd_du(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let path = if argc >= 2 {
        resolve_path(args[1])
    } else {
        alloc::string::String::from(cwd())
    };
    match crate::fs::vfs::list_dir(&path) {
        Ok(entries) => {
            let mut total: u64 = 0;
            for entry in &entries {
                crate::println!("{:8}  {}/{}", entry.size, path, entry.name_str());
                total += entry.size;
            }
            crate::println!("{:8}  {} (total)", total, path);
            0
        }
        Err(e) => {
            crate::println!("du: {}: error {:?}", path, e);
            1
        }
    }
}

/// df - report filesystem disk space usage
fn cmd_df() -> i32 {
    crate::println!("Filesystem     Size   Used   Free  Use%  Mount");
    crate::println!("LobsterVFS     16M    ???    ???   ??%  /");
    crate::println!("(In-memory VFS - physical space not tracked)");
    0
}

/// free - display amount of free and used memory
fn cmd_free() -> i32 {
    cmd_mem();
    0
}

/// which - locate a command
fn cmd_which(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: which <command>\r\n");
        return 1;
    }
    // All commands are built-in shell commands
    let builtin = [
        "help", "clear", "uptime", "mem", "ps", "kill", "echo", "version",
        "cpu", "timer", "gpio", "login", "su", "sudo", "passwd", "useradd",
        "userdel", "groupadd", "groupdel", "id", "whoami", "hexdump", "yield",
        "test", "set", "export", "unset", "env", "printenv", "history",
        "cat", "ls", "cd", "pwd", "mkdir", "rm", "mv", "chmod", "chown",
        "touch", "cp", "head", "tail", "wc", "du", "df", "free", "which",
        "uname", "date", "find", "run", "svc", "runlevel", "reboot", "halt",
        "cc", "curl", "wget", "pkg", "gh", "vi", "edit", "ed", "nano",
        "grep", "sort", "cut", "tr", "uniq", "seq", "yes", "tee",
        "stat", "file", "ln", "basename", "dirname", "sleep", "man", "who",
    ];
    if builtin.contains(&args[1]) {
        crate::println!("{}: shell built-in command", args[1]);
        0
    } else {
        crate::println!("{}: not found", args[1]);
        1
    }
}

/// uname - print system information
fn cmd_uname() -> i32 {
    crate::println!("LobsterOS aarch64 BCM2712 rpi5b");
    crate::println!("Kernel: v0.2.0");
    crate::println!("Architecture: aarch64");
    crate::println!("Hardware: Raspberry Pi 5 (BCM2712)");
    crate::println!("CPU: Cortex-A72 @ 2.0GHz x4");
    0
}

/// date - display current time (from system timer)
fn cmd_date() -> i32 {
    let uptime_ms = crate::arch::timer::Timer::uptime_ms();
    let seconds = uptime_ms / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    crate::println!(
        "Uptime: {}d {}h {}m {}s (no RTC - timer-based)",
        days,
        hours % 24,
        minutes % 60,
        seconds % 60
    );
    0
}

/// find - find files by name in directory tree
fn cmd_find(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let mut start_path = alloc::string::String::from(cwd());
    let mut name_pattern: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        if args[i] == "-name" && i + 1 < argc {
            name_pattern = Some(args[i + 1]);
            i += 2;
            continue;
        }
        if !args[i].starts_with('-') {
            start_path = resolve_path(args[i]);
        }
        i += 1;
    }

    find_recursive(&start_path, name_pattern, 0);
    0
}

/// Recursive helper for find
fn find_recursive(path: &str, pattern: Option<&str>, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Ok(entries) = crate::fs::vfs::list_dir(path) {
        for entry in &entries {
            let full = if path == "/" {
                alloc::format!("/{}", entry.name_str())
            } else {
                alloc::format!("{}/{}", path, entry.name_str())
            };

            let matches = match pattern {
                Some(p) => {
                    if p.starts_with('*') {
                        let suffix = &p[1..];
                        entry.name_str().ends_with(suffix)
                    } else if p.ends_with('*') {
                        let prefix = &p[..p.len() - 1];
                        entry.name_str().starts_with(prefix)
                    } else {
                        entry.name_str() == p
                    }
                }
                None => true,
            };

            if matches {
                crate::println!("{}", full);
            }

            if entry.is_dir {
                find_recursive(&full, pattern, depth + 1);
            }
        }
    }
}

/// Parse usize from string
fn parse_usize(s: &str) -> Option<usize> {
    let mut result: usize = 0;
    for c in s.chars() {
        if let Some(d) = c.to_digit(10) {
            result = result.checked_mul(10)?.checked_add(d as usize)?;
        } else {
            return None;
        }
    }
    Some(result)
}

// =================================================================
// Network Commands
// =================================================================

/// Check if networking is available on this platform
fn net_available() -> bool {
    #[cfg(feature = "qemu_raspi5b")]
    {
        crate::drivers::lobster_net::is_available()
    }
    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        true
    }
}

/// Print network-not-available message
fn net_unavailable_msg(cmd: &str) {
    crate::println!(
        "{}: networking is not available in QEMU mode.",
        cmd
    );
    crate::println!(
        "{}: The QEMU raspi5b machine does not emulate the BCM2712 GENET Ethernet controller.",
        cmd
    );
    crate::println!("  Network commands (curl, wget, ping, host) work on real Raspberry Pi 5 hardware.");
    crate::println!("  The full TCP/IP + DNS + HTTP stack is compiled in and ready.");
}

/// curl - transfer data from or to a server using HTTP
fn cmd_curl(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: curl [options] <url>\r\n");
        Uart10::puts("  -o <file>   Write output to file instead of stdout\r\n");
        Uart10::puts("  -s          Silent mode (suppress progress)\r\n");
        Uart10::puts("  -I          Fetch headers only (HEAD-ish)\r\n");
        Uart10::puts("\r\nExample: curl http://example.com/page\r\n");
        return 1;
    }

    if !net_available() {
        net_unavailable_msg("curl");
        return 1;
    }

    // Parse common args
    let mut output_file: Option<&str> = None;
    let mut silent = false;
    let mut url: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        match args[i] {
            "-o" | "--output" => {
                if i + 1 < argc {
                    output_file = Some(args[i + 1]);
                    i += 1;
                }
            }
            "-s" | "--silent" => {
                silent = true;
            }
            "-I" | "--head" => {
                // For now just do GET and show headers
            }
            "-h" | "--help" => {
                Uart10::puts("Usage: curl [options] <url>\r\n");
                return 0;
            }
            _ => {
                if url.is_none() && !args[i].starts_with('-') {
                    url = Some(args[i]);
                }
            }
        }
        i += 1;
    }

    let url = match url {
        Some(u) => u,
        None => {
            crate::println!("curl: no URL specified");
            return 1;
        }
    };

    if !silent {
        crate::println!("curl: connecting to {}", url);
    }

    // Fetch via appropriate backend
    #[cfg(not(feature = "qemu_raspi5b"))]
    let body_opt = crate::net::http::http_get(url).map(|resp| {
        if !silent {
            crate::println!(
                "curl: HTTP {} ({} bytes)",
                resp.status_code, resp.body_len
            );
        }
        (resp.status_code, alloc::vec::Vec::from(&resp.body[..resp.body_len]))
    });

    #[cfg(feature = "qemu_raspi5b")]
    let body_opt = crate::drivers::lobster_net::http_fetch(url).map(|body| {
        let len = body.len();
        if !silent {
            crate::println!("curl: HTTP 200 ({} bytes)", len);
        }
        (200, body)
    });

    match body_opt {
        Some((status_code, body)) => {
            if let Some(filename) = output_file {
                // Write body to file
                use crate::fs::vfs;
                let fd = match vfs::open(
                    filename,
                    vfs::O_WRONLY | vfs::O_CREAT | vfs::O_TRUNC,
                ) {
                    Ok(fd) => fd,
                    Err(e) => {
                        crate::println!("curl: cannot open {} for writing: {:?}", filename, e);
                        return 1;
                    }
                };
                let _ = vfs::write(fd, &body);
                vfs::close(fd);
                if !silent {
                    crate::println!("curl: saved {} bytes to {}", body.len(), filename);
                }
            } else {
                // Print body to stdout
                let body_str = core::str::from_utf8(&body)
                    .unwrap_or("<binary data>");
                crate::println!("{}", body_str);
            }
            0
        }
        None => {
            crate::println!("curl: failed to connect or no response");
            1
        }
    }
}

/// wget - download files from the web
fn cmd_wget(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: wget [options] <url>\r\n");
        Uart10::puts("  -O <file>   Write to specified filename\r\n");
        Uart10::puts("  -q          Quiet mode\r\n");
        Uart10::puts("\r\nExample: wget http://example.com/file.txt\r\n");
        return 1;
    }

    if !net_available() {
        net_unavailable_msg("wget");
        return 1;
    }

    // Parse common args
    let mut output_file: Option<&str> = None;
    let mut quiet = false;
    let mut url: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        match args[i] {
            "-O" | "--output-document" => {
                if i + 1 < argc {
                    output_file = Some(args[i + 1]);
                    i += 1;
                }
            }
            "-q" | "--quiet" => {
                quiet = true;
            }
            _ => {
                if url.is_none() && !args[i].starts_with('-') {
                    url = Some(args[i]);
                }
            }
        }
        i += 1;
    }

    let url = match url {
        Some(u) => u,
        None => {
            crate::println!("wget: no URL specified");
            return 1;
        }
    };

    // Determine output filename from URL if not specified
    let filename = if let Some(f) = output_file {
        f
    } else {
        // Extract filename from URL
        if let Some(idx) = url.rfind('/') {
            &url[idx + 1..]
        } else {
            "index.html"
        }
    };

    if !quiet {
        crate::println!("wget: downloading {}", url);
        crate::println!("wget: saving to {}", filename);
    }

    // Fetch via appropriate backend
    #[cfg(not(feature = "qemu_raspi5b"))]
    let body_opt = crate::net::http::http_get(url).map(|resp| {
        (resp.status_code, alloc::vec::Vec::from(&resp.body[..resp.body_len]))
    });

    #[cfg(feature = "qemu_raspi5b")]
    let body_opt = crate::drivers::lobster_net::http_fetch(url).map(|body| (200, body));

    match body_opt {
        Some((status_code, body)) => {
            if status_code != 200 {
                crate::println!("wget: server returned HTTP {}", status_code);
                return 1;
            }

            use crate::fs::vfs;
            let fd = match vfs::open(filename, vfs::O_WRONLY | vfs::O_CREAT | vfs::O_TRUNC) {
                Ok(fd) => fd,
                Err(e) => {
                    crate::println!("wget: cannot create {}: {:?}", filename, e);
                    return 1;
                }
            };
            let _ = vfs::write(fd, &body);
            vfs::close(fd);

            if !quiet {
                crate::println!(
                    "wget: saved {} bytes to {}",
                    body.len(), filename
                );
            }
            0
        }
        None => {
            crate::println!("wget: download failed");
            1
        }
    }
}

/// ip - show/manipulate routing, devices, policy routing and tunnels
fn cmd_ip(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        // Default: show interfaces
        return cmd_ifconfig(args, argc);
    }

    match args[1] {
        "addr" | "a" | "address" => cmd_ifconfig(args, argc),
        "link" | "l" => {
            if !net_available() {
                net_unavailable_msg("ip link");
                return 1;
            }
            #[cfg(not(feature = "qemu_raspi5b"))]
            {
                let mac = crate::net::genet::mac_addr();
                crate::println!(
                    "eth0: <BROADCAST,MULTICAST,UP> mtu 1500 qdisc pfifo_fast state UP",
                );
                crate::println!("    link/ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
            }
            0
        }
        "route" | "r" => {
            if !net_available() {
                net_unavailable_msg("ip route");
                return 1;
            }
            #[cfg(not(feature = "qemu_raspi5b"))]
            {
                crate::println!("default via {} dev eth0", "?.?.?.?"); // gateway
                crate::println!("{} dev eth0 proto kernel scope link src ?.?.?.?", "?.?.?.?/24");
            }
            0
        }
        "-h" | "--help" | "help" => {
            Uart10::puts("Usage: ip <command> [options]\r\n");
            Uart10::puts("  ip addr       Show addresses\r\n");
            Uart10::puts("  ip link       Show link info\r\n");
            Uart10::puts("  ip route      Show routing table\r\n");
            0
        }
        _ => {
            crate::println!("ip: unknown subcommand '{}'", args[1]);
            1
        }
    }
}

/// ifconfig - configure network interface
fn cmd_ifconfig(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let _ = argc;

    if !net_available() {
        net_unavailable_msg("ifconfig");
        crate::println!();
        crate::println!("In QEMU mode, the network stack is compiled but inactive.");
        crate::println!("Available net modules: genet, tcp, udp, dns, dhcp, http, icmp, arp");
        crate::println!();
        crate::println!("To test networking, run on real Raspberry Pi 5 hardware.");
        return 0;
    }

    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        let mac = crate::net::genet::mac_addr();

        crate::println!("eth0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500");
        crate::println!("        inet {}.{}.{}.{}  netmask 255.255.255.0",
            0, 0, 0, 0); // would show real IP
        crate::println!("        ether {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
        crate::println!("        RX packets:0  TX packets:0");
        crate::println!("        RX bytes:0  TX bytes:0");
        crate::println!();
        crate::println!("lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536");
        crate::println!("        inet 127.0.0.1  netmask 255.0.0.0");
        crate::println!("        loop  (Local loopback)");
    }
    0
}

/// ping - send ICMP echo requests
fn cmd_ping(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: ping [-c count] <host>\r\n");
        Uart10::puts("  -c <n>   Stop after n echoes (default 4)\r\n");
        return 1;
    }

    if !net_available() {
        net_unavailable_msg("ping");
        return 1;
    }

    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        let mut count: u32 = 4;
        let mut host: Option<&str> = None;

        let mut i = 1;
        while i < argc {
            match args[i] {
                "-c" => {
                    if i + 1 < argc {
                        if let Some(n) = parse_usize(args[i + 1]) {
                            count = n as u32;
                        }
                        i += 1;
                    }
                }
                _ => {
                    if host.is_none() && !args[i].starts_with('-') {
                        host = Some(args[i]);
                    }
                }
            }
            i += 1;
        }

        let host = match host {
            Some(h) => h,
            None => {
                crate::println!("ping: no host specified");
                return 1;
            }
        };

        crate::println!("PING {} ...", host);

        match crate::net::dns::resolve(host) {
            Some(ip) => {
                crate::println!(
                    "resolved {} to {}.{}.{}.{}",
                    host, ip[0], ip[1], ip[2], ip[3]
                );

                for seq in 0..count {
                    crate::net::icmp::send_echo_request(ip, seq as u16);
                    crate::println!(
                        "icmp_seq={} sent to {}.{}.{}.{}",
                        seq, ip[0], ip[1], ip[2], ip[3]
                    );
                    for _ in 0..1_000_000 {
                        crate::arch::cpu::wfe();
                    }
                }
                crate::println!("--- {} ping statistics ---", host);
                crate::println!("{} packets transmitted", count);
                0
            }
            None => {
                crate::println!("ping: cannot resolve {}", host);
                1
            }
        }
    }
    #[cfg(feature = "qemu_raspi5b")]
    {
        // In QEMU mode, use lobster-net HTTP fetch as a connectivity check
        if argc < 2 {
            Uart10::puts("Usage: ping [-c count] <host>\r\n");
            return 1;
        }

        let mut host: Option<&str> = None;
        let mut count: u32 = 4;
        let mut i = 1;
        while i < argc {
            match args[i] {
                "-c" => {
                    if i + 1 < argc {
                        if let Some(n) = parse_usize(args[i + 1]) {
                            count = n as u32;
                        }
                        i += 1;
                    }
                }
                _ => {
                    if host.is_none() && !args[i].starts_with('-') {
                        host = Some(args[i]);
                    }
                }
            }
            i += 1;
        }

        let host = match host {
            Some(h) => h,
            None => {
                crate::println!("ping: no host specified");
                return 1;
            }
        };

        let url = if host.starts_with("http://") || host.starts_with("https://") {
            alloc::string::String::from(host)
        } else {
            alloc::format!("http://{}/", host)
        };

        crate::println!("PING {} (via lobster-net HTTP fetch)", host);

        let mut transmitted = 0u32;
        let mut received = 0u32;

        for seq in 0..count {
            transmitted += 1;
            match crate::net::fetch_url(&url) {
                Some((status, body)) => {
                    received += 1;
                    crate::println!(
                        "64 bytes from {}: icmp_seq={} HTTP {} ({} bytes)",
                        host, seq, status, body.len()
                    );
                }
                None => {
                    crate::println!("no response from {}: icmp_seq={}", host, seq);
                }
            }
            for _ in 0..5_000_000 {
                crate::arch::cpu::wfe();
            }
        }

        crate::println!("--- {} ping statistics ---", host);
        let loss_pct = if transmitted > 0 {
            ((transmitted - received) as f64 / transmitted as f64) * 100.0
        } else {
            100.0
        };
        crate::println!(
            "{} packets transmitted, {} received, {:.0}% packet loss",
            transmitted, received, loss_pct
        );
        0
    }
}

/// host - DNS lookup
fn cmd_host(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: host <hostname>\r\n");
        return 1;
    }

    if !net_available() {
        net_unavailable_msg("host");
        return 1;
    }

    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        let hostname = args[1];
        crate::println!("Host {}:", hostname);

        match crate::net::dns::resolve(hostname) {
            Some(ip) => {
                crate::println!("  {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                crate::println!("  {}", hostname);
                0
            }
            None => {
                crate::println!("host: not found: {}", hostname);
                1
            }
        }
    }
    #[cfg(feature = "qemu_raspi5b")]
    {
        // In QEMU mode, do a simple HTTP connectivity check
        let hostname = args[1];
        crate::println!("Host {}:", hostname);

        let url = if hostname.starts_with("http://") || hostname.starts_with("https://") {
            alloc::string::String::from(hostname)
        } else {
            alloc::format!("http://{}/", hostname)
        };

        match crate::net::fetch_url(&url) {
            Some((status, body)) => {
                crate::println!("  {} is reachable (HTTP {}, {} bytes)", hostname, status, body.len());
                crate::println!("  (resolved via lobster-net MMIO proxy)");
                0
            }
            None => {
                crate::println!("host: cannot reach {}", hostname);
                1
            }
        }
    }
}

/// netstat - show network status
fn cmd_netstat() -> i32 {
    if !net_available() {
        net_unavailable_msg("netstat");
        return 0;
    }

    #[cfg(not(feature = "qemu_raspi5b"))]
    {
        crate::println!("Active Internet connections (w/o servers)");
        crate::println!("Proto Recv-Q Send-Q Local Address     Foreign Address    State");
        crate::println!("Active UNIX domain sockets (w/o servers)");
        crate::println!("Proto RefCnt Flags   Type    State    I-Node  Path");
    }

    #[cfg(feature = "qemu_raspi5b")]
    {
        crate::println!("Active Internet connections (w/o servers)");
        crate::println!("Proto Recv-Q Send-Q Local Address     Foreign Address    State");
        crate::println!("  lobster-net MMIO proxy at 0x7D009000  http://*          ESTABLISHED");
        crate::println!("  (all HTTP traffic proxied via host curl)");
        crate::println!();
        crate::println!("Active UNIX domain sockets (w/o servers)");
        crate::println!("Proto RefCnt Flags   Type    State    I-Node  Path");
    }
    0
}


// ============================================================
// NEW COMMANDS — Text Editor, Text Processing, File Utils
// ============================================================

/// Text editor (vi/ed/edit/nano) — simple line-based editor
/// GitHub CLI (gh) — lightweight GitHub API client over the lobster-net proxy.
/// Token is stored in /etc/github_token (a VFS file). Because the lobster-net
/// device terminates TLS on the host via curl, HTTPS to api.github.com works.
/// Commands: auth login|status|logout, api <endpoint>, repo list [owner], user
const GH_TOKEN_PATH: &str = "/etc/github_token";

fn gh_read_token() -> Option<alloc::string::String> {
    match crate::fs::vfs::read_file(GH_TOKEN_PATH) {
        Ok(data) => {
            let trimmed = data.iter().copied()
                .take_while(|&b| b != b'\n' && b != b'\r' && b != b' ' && b != b'\t')
                .collect::<alloc::vec::Vec<u8>>();
            if trimmed.is_empty() {
                None
            } else {
                alloc::string::String::from_utf8(trimmed).ok()
            }
        }
        Err(_) => None,
    }
}

fn gh_write_token(token: &str) -> bool {
    crate::fs::vfs::write_file(GH_TOKEN_PATH, token.as_bytes()).is_ok()
}

/// GET an api.github.com endpoint through the lobster-net proxy, using the
/// stored token if present. Prints status and returns the raw body.
fn gh_get(endpoint: &str) -> Option<alloc::vec::Vec<u8>> {
    let url = alloc::format!("https://api.github.com{}", endpoint);
    if let Some(token) = gh_read_token() {
        crate::drivers::lobster_net::http_fetch_auth(&url, &token)
    } else {
        crate::drivers::lobster_net::http_fetch(&url)
    }
}

fn cmd_gh(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 || args[1] == "help" {
        Uart10::puts("Usage: gh <command> [args]\r\n");
        Uart10::puts("  gh auth login     Save a GitHub Personal Access Token\r\n");
        Uart10::puts("  gh auth status    Show login state\r\n");
        Uart10::puts("  gh auth logout    Remove the saved token\r\n");
        Uart10::puts("  gh api <endpoint> GET https://api.github.com<endpoint>\r\n");
        Uart10::puts("  gh repo list [owner]  List an owner's public repos\r\n");
        Uart10::puts("  gh user           Show the authenticated user (needs token)\r\n");
        return if argc < 2 { 1 } else { 0 };
    }

    if !net_available() {
        net_unavailable_msg("gh");
        return 1;
    }

    match args[1] {
        "auth" => {
            if argc < 3 {
                Uart10::puts("Usage: gh auth <login|status|logout>\r\n");
                return 1;
            }
            match args[2] {
                "login" => {
                    Uart10::puts("Paste your GitHub Personal Access Token (SCAN / repo scope): ");
                    let mut buf = [0u8; 256];
                    let saved_echo = crate::drivers::serial_input::echo_enabled();
                    crate::drivers::serial_input::set_echo(false);
                    let len = crate::drivers::serial_input::read_line_blocking(&mut buf);
                    crate::drivers::serial_input::set_echo(saved_echo);
                    let token = core::str::from_utf8(&buf[..len]).unwrap_or("").trim();
                    if token.is_empty() {
                        crate::println!("gh: no token entered");
                        return 1;
                    }
                    // Validate token charset matches the device allowlist
                    let ok = token.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
                    });
                    if !ok {
                        crate::println!("gh: token contains unsupported characters");
                        return 1;
                    }
                    if gh_write_token(token) {
                        crate::println!("");
                        crate::println!("✓ Authenticated. Token saved to /etc/github_token");
                        crate::println!("  Test with: gh api /user  or  gh user");
                        0
                    } else {
                        crate::println!("gh: failed to save token to /etc/github_token");
                        1
                    }
                }
                "status" => {
                    if gh_read_token().is_some() {
                        crate::println!("Logged in to GitHub (token present in /etc/github_token)");
                    } else {
                        crate::println!("Not logged in. Run: gh auth login");
                    }
                    0
                }
                "logout" => {
                    if crate::fs::vfs::remove(GH_TOKEN_PATH).is_ok() {
                        crate::println!("Logged out. Removed /etc/github_token");
                    } else {
                        crate::println!("No token to revoke (already logged out)");
                    }
                    0
                }
                _ => {
                    crate::println!("gh auth: unknown subcommand '{}'", args[2]);
                    1
                }
            }
        }
        "user" => {
            match gh_get("/user") {
                Some(body) => {
                    let s = core::str::from_utf8(&body).unwrap_or("<binary>");
                    crate::println!("{}", s);
                    0
                }
                None => {
                    crate::println!("gh: request failed (are you logged in? see 'gh auth status')");
                    1
                }
            }
        }
        "api" => {
            if argc < 3 {
                Uart10::puts("Usage: gh api <endpoint>   e.g. gh api /repos/jlaportebot/lobster-os\r\n");
                return 1;
            }
            let mut endpoint = alloc::string::String::from(args[2]);
            if !endpoint.starts_with('/') {
                endpoint.insert(0, '/');
            }
            match gh_get(&endpoint) {
                Some(body) => {
                    let s = core::str::from_utf8(&body).unwrap_or("<binary>");
                    crate::println!("{}", s);
                    0
                }
                None => {
                    crate::println!("gh: request failed (check endpoint / token)");
                    1
                }
            }
        }
        "repo" => {
            if argc < 3 || args[2] != "list" {
                Uart10::puts("Usage: gh repo list [owner]  (defaults to /user/repos)\r\n");
                return 1;
            }
            let endpoint = if argc >= 4 {
                alloc::format!("/users/{}/repos", args[3])
            } else {
                alloc::string::String::from("/user/repos")
            };
            match gh_get(&endpoint) {
                Some(body) => {
                    let s = core::str::from_utf8(&body).unwrap_or("<binary>");
                    crate::println!("{}", s);
                    0
                }
                None => {
                    crate::println!("gh: failed to list repos");
                    1
                }
            }
        }
        _ => {
            crate::println!("gh: unknown command '{}' (see 'gh help')", args[1]);
            1
        }
    }
}

/// Usage: vi <file>    — opens file for editing
/// In editor mode, each line typed is appended to the buffer.
/// Commands: . = save & quit, :w = save, :q = quit, :wq = save & quit
///           :l = list contents, :d <N> = delete line N, :i <N> = insert before line N
fn cmd_editor(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: vi <file>\r\n");
        Uart10::puts("  Line-based editor. Commands:\r\n");
        Uart10::puts("    .         Save and quit\r\n");
        Uart10::puts("    :w        Save\r\n");
        Uart10::puts("    :q        Quit without save\r\n");
        Uart10::puts("    :wq       Save and quit\r\n");
        Uart10::puts("    :l        List all lines\r\n");
        Uart10::puts("    :d <N>    Delete line N\r\n");
        Uart10::puts("    :i <N>    Insert before line N (end with .)\r\n");
        Uart10::puts("    <text>    Append line to buffer\r\n");
        return 1;
    }

    let path = resolve_path(args[1]);

    // Load existing content
    let mut buffer: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    match crate::fs::vfs::read_file(&path) {
        Ok(data) => {
            buffer = data;
            if !buffer.is_empty() {
                Uart10::puts(&alloc::format!("Editing {} ({} bytes)\r\n", path, buffer.len()));
                // Show existing lines
                let content = core::str::from_utf8(&buffer).unwrap_or("");
                for (i, line) in content.lines().enumerate() {
                    crate::println!("  {:>3}  {}", i + 1, line);
                }
            } else {
                Uart10::puts(&alloc::format!("New file: {}\r\n", path));
            }
        }
        Err(_) => {
            Uart10::puts(&alloc::format!("New file: {}\r\n", path));
        }
    }

    Uart10::puts("Editor mode. Type lines to append. '.' to save & quit, ':q' to quit.\r\n");
    Uart10::puts("> ");

    loop {
        let mut line_buf = [0u8; 512];
        let len = crate::drivers::serial_input::read_line_blocking(&mut line_buf);
        let line = core::str::from_utf8(&line_buf[..len]).unwrap_or("");

        match line.trim() {
            "." | ":wq" => {
                // Save and quit
                match crate::fs::vfs::write_file(&path, &buffer) {
                    Ok(n) => crate::println!("Saved {} bytes to {}", n, path),
                    Err(e) => crate::println!("Error saving: {:?}", e),
                }
                Uart10::puts("\r\n");
                return 0;
            }
            ":w" => {
                match crate::fs::vfs::write_file(&path, &buffer) {
                    Ok(n) => crate::println!("Saved {} bytes to {}", n, path),
                    Err(e) => crate::println!("Error saving: {:?}", e),
                }
            }
            ":q" => {
                Uart10::puts("\r\n");
                return 0;
            }
            ":l" => {
                let content = core::str::from_utf8(&buffer).unwrap_or("");
                for (i, l) in content.lines().enumerate() {
                    crate::println!("  {:>3}  {}", i + 1, l);
                }
            }
            s if s.starts_with(":d ") => {
                if let Ok(n) = s[3..].trim().parse::<usize>() {
                    let content = core::str::from_utf8(&buffer).unwrap_or("");
                    let lines: alloc::vec::Vec<&str> = content.lines().collect();
                    if n > 0 && n <= lines.len() {
                        let mut new_buf = alloc::vec::Vec::new();
                        for (i, l) in lines.iter().enumerate() {
                            if i + 1 != n {
                                new_buf.extend_from_slice(l.as_bytes());
                                new_buf.push(b'\n');
                            }
                        }
                        buffer = new_buf;
                        crate::println!("Deleted line {}", n);
                    } else {
                        Uart10::puts("Invalid line number\r\n");
                    }
                }
            }
            s if s.starts_with(":i ") => {
                if let Ok(n) = s[3..].trim().parse::<usize>() {
                    // Read lines to insert
                    Uart10::puts("Inserting (end with .):\r\n");
                    let mut insert_lines: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
                    loop {
                        Uart10::puts(">> ");
                        let mut ibuf = [0u8; 512];
                        let ilen = crate::drivers::serial_input::read_line_blocking(&mut ibuf);
                        let iline = core::str::from_utf8(&ibuf[..ilen]).unwrap_or("");
                        if iline.trim() == "." {
                            break;
                        }
                        insert_lines.push(alloc::string::String::from(iline));
                    }
                    // Rebuild buffer with inserted lines
                    let content = core::str::from_utf8(&buffer).unwrap_or("");
                    let lines: alloc::vec::Vec<&str> = content.lines().collect();
                    let mut new_buf = alloc::vec::Vec::new();
                    for (i, l) in lines.iter().enumerate() {
                        if i + 1 == n {
                            for il in &insert_lines {
                                new_buf.extend_from_slice(il.as_bytes());
                                new_buf.push(b'\n');
                            }
                        }
                        new_buf.extend_from_slice(l.as_bytes());
                        new_buf.push(b'\n');
                    }
                    if n > lines.len() {
                        for il in &insert_lines {
                            new_buf.extend_from_slice(il.as_bytes());
                            new_buf.push(b'\n');
                        }
                    }
                    buffer = new_buf;
                    crate::println!("Inserted {} lines at {}", insert_lines.len(), n);
                }
            }
            _ => {
                // Append line to buffer
                buffer.extend_from_slice(line.as_bytes());
                buffer.push(b'\n');
            }
        }
        Uart10::puts("> ");
    }
}

/// cc — Compile C source to ELF using lobster-net compile proxy
/// Usage: cc <file.c> [-o output] [-run]
fn cmd_cc(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: cc <file.c> [-o output] [-run]\r\n");
        Uart10::puts("  Compiles C source via lobster-net MMIO compile proxy.\r\n");
        Uart10::puts("  -o <path>  Output path (default: /bin/<name without .c>)\r\n");
        Uart10::puts("  -run       Run after compiling\r\n");
        Uart10::puts("\r\nExample: cc hello.c -run\r\n");
        return 1;
    }

    let mut source_path: Option<&str> = None;
    let mut output_path: Option<&str> = None;
    let mut run_after = false;

    let mut i = 1;
    while i < argc {
        match args[i] {
            "-o" if i + 1 < argc => {
                output_path = Some(args[i + 1]);
                i += 1;
            }
            "-run" => run_after = true,
            s if !s.starts_with('-') => {
                if source_path.is_none() {
                    source_path = Some(s);
                }
            }
            _ => {}
        }
        i += 1;
    }

    let source_path = match source_path {
        Some(p) => p,
        None => {
            crate::println!("cc: no source file specified");
            return 1;
        }
    };

    let full_source = resolve_path(source_path);

    // Read source file from VFS
    let source_data = match crate::fs::vfs::read_file(&full_source) {
        Ok(data) => data,
        Err(e) => {
            crate::println!("cc: cannot read {}: {:?}", full_source, e);
            return 1;
        }
    };

    if source_data.is_empty() {
        crate::println!("cc: {} is empty", full_source);
        return 1;
    }

    crate::println!("cc: compiling {} ({} bytes)...", full_source, source_data.len());

    // Determine output path
    let default_output = {
        let name = source_path.rsplit('/').next().unwrap_or(source_path);
        let stem = name.strip_suffix(".c").unwrap_or(name);
        alloc::string::String::from("/bin/") + stem
    };
    let output = output_path
        .map(|p| resolve_path(p))
        .unwrap_or(default_output);

    // Compile via lobster-net device
    let source_str = match core::str::from_utf8(&source_data) {
        Ok(s) => s,
        Err(_) => {
            crate::println!("cc: source file is not valid UTF-8");
            return 1;
        }
    };

    match crate::drivers::lobster_net::compile(source_str) {
        Some(elf) => {
            crate::println!("cc: compiled OK ({} bytes ELF)", elf.len());

            // Ensure /bin directory exists
            let _ = crate::fs::vfs::create_dir("/bin");

            match crate::fs::vfs::write_file(&output, &elf) {
                Ok(n) => crate::println!("cc: saved {} bytes to {}", n, output),
                Err(e) => {
                    crate::println!("cc: failed to write {}: {:?}", output, e);
                    return 1;
                }
            }

            if run_after {
                crate::println!("cc: running {}...", output);
                match crate::sched::spawn_from_path(&output, crate::sched::Priority::Normal) {
                    Some(pid) => {
                        crate::println!("cc: spawned PID {}", pid);
                        let exit_code = unsafe { crate::sched::run_and_wait(pid as u64) };
                        crate::println!("cc: process exited with code {}", exit_code);
                        // sh semantics: propagate the program's exit status
                        // through $? (and as cc's own exit status).
                        unsafe {
                            LAST_EXIT_CODE = exit_code as i32;
                        }
                        return exit_code as i32;
                    }
                    None => crate::println!("cc: failed to spawn process"),
                }
            }
            0
        }
        None => {
            crate::println!("cc: compilation failed (no response from compile proxy)");
            crate::println!("cc: ensure compile_server.py or lobster-net CMD=3 is available");
            1
        }
    }
}

/// grep — search for pattern in file
fn cmd_grep(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: grep <pattern> <file>\r\n");
        return 1;
    }
    let pattern = args[1];
    let path = resolve_path(args[2]);

    let data = match crate::fs::vfs::read_file(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("grep: {}: {:?}", path, e);
            return 1;
        }
    };

    let content = core::str::from_utf8(&data).unwrap_or("");
    let mut count = 0;
    for (i, line) in content.lines().enumerate() {
        if line.contains(pattern) {
            crate::println!("{}: {}", i + 1, line);
            count += 1;
        }
    }

    if count == 0 {
        crate::println!("grep: no matches");
    }
    0
}

/// sort — sort lines of a file
fn cmd_sort(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: sort <file>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);

    let data = match crate::fs::vfs::read_file(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("sort: {}: {:?}", path, e);
            return 1;
        }
    };

    let content = core::str::from_utf8(&data).unwrap_or("");
    let mut lines: alloc::vec::Vec<&str> = content.lines().collect();
    lines.sort();
    for line in &lines {
        crate::println!("{}", line);
    }
    0
}

/// cut — extract fields from lines (delim default: space, field default: 1)
fn cmd_cut(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: cut [-d <delim>] [-f <N>] <file>\r\n");
        return 1;
    }

    let mut delim = ' ';
    let mut field = 1usize;
    let mut file_path: Option<&str> = None;

    let mut i = 1;
    while i < argc {
        match args[i] {
            "-d" if i + 1 < argc => {
                let d = args[i + 1];
                if !d.is_empty() {
                    delim = d.chars().next().unwrap();
                }
                i += 1;
            }
            "-f" if i + 1 < argc => {
                field = args[i + 1].parse().unwrap_or(1);
                i += 1;
            }
            s if !s.starts_with('-') => {
                file_path = Some(s);
            }
            _ => {}
        }
        i += 1;
    }

    let path = match file_path {
        Some(p) => resolve_path(p),
        None => {
            Uart10::puts("cut: no file specified\r\n");
            return 1;
        }
    };

    let data = match crate::fs::vfs::read_file(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("cut: {}: {:?}", path, e);
            return 1;
        }
    };

    let content = core::str::from_utf8(&data).unwrap_or("");
    for line in content.lines() {
        let parts: alloc::vec::Vec<&str> = line.split(delim).collect();
        if field > 0 && field <= parts.len() {
            crate::println!("{}", parts[field - 1]);
        }
    }
    0
}

/// tr — translate characters (simplified: maps set1 to set2 char-by-char)
fn cmd_tr(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: tr <set1> <set2> <file>\r\n");
        Uart10::puts("  Replaces each char in set1 with corresponding char in set2\r\n");
        return 1;
    }
    let set1 = args[1];
    let set2 = args[2];
    let path = resolve_path(args[3]);

    let data = match crate::fs::vfs::read_file(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("tr: {}: {:?}", path, e);
            return 1;
        }
    };

    let content = core::str::from_utf8(&data).unwrap_or("");
    let s1: alloc::vec::Vec<char> = set1.chars().collect();
    let s2: alloc::vec::Vec<char> = set2.chars().collect();
    let result: alloc::string::String = content.chars().map(|c| {
        for (i, &sc) in s1.iter().enumerate() {
            if c == sc && i < s2.len() {
                return s2[i];
            }
        }
        c
    }).collect();
    crate::println!("{}", result);
    0
}

/// uniq — filter adjacent matching lines (keep only first of each run)
fn cmd_uniq(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: uniq <file>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);

    let data = match crate::fs::vfs::read_file(&path) {
        Ok(d) => d,
        Err(e) => {
            crate::println!("uniq: {}: {:?}", path, e);
            return 1;
        }
    };

    let content = core::str::from_utf8(&data).unwrap_or("");
    let mut prev: Option<&str> = None;
    let mut count = 0;
    for line in content.lines() {
        if Some(line) != prev {
            crate::println!("{}", line);
            count += 1;
        }
        prev = Some(line);
    }
    crate::println!("({} unique lines)", count);
    0
}

/// seq — print a sequence of numbers
fn cmd_seq(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let (start, end) = match argc {
        2 => (1, args[1].parse::<i64>().unwrap_or(0)),
        3 => {
            let s = args[1].parse::<i64>().unwrap_or(0);
            let e = args[2].parse::<i64>().unwrap_or(0);
            (s, e)
        }
        _ => {
            Uart10::puts("Usage: seq <n>  or  seq <start> <end>\r\n");
            return 1;
        }
    };

    // Fast path: convert numbers into a 2KB batch buffer flushed through the
    // bulk-serial window (when present) — one MMIO write per 2048 bytes
    // instead of one MMIO write per byte via the PL011.
    let mut num = [0u8; 24];
    let mut batch = [0u8; 2048];
    let mut batch_len = 0usize;

    let emit_into = |n: i64, num: &mut [u8; 24], batch: &mut [u8; 2048], blen: &mut usize| {
        let mut pos = num.len();
        let mut v = n.unsigned_abs();
        if v == 0 {
            pos -= 1;
            num[pos] = b'0';
        }
        while v > 0 {
            pos -= 1;
            num[pos] = b'0' + (v % 10) as u8;
            v /= 10;
        }
        if n < 0 {
            pos -= 1;
            num[pos] = b'-';
        }
        let digits = &num[pos..];
        if *blen + digits.len() + 2 > batch.len() {
            Uart10::puts_bytes(&batch[..*blen]);
            *blen = 0;
        }
        batch[*blen..*blen + digits.len()].copy_from_slice(digits);
        *blen += digits.len();
        batch[*blen] = b'\r';
        batch[*blen + 1] = b'\n';
        *blen += 2;
    };

    if start <= end {
        for n in start..=end {
            emit_into(n, &mut num, &mut batch, &mut batch_len);
        }
    } else {
        for n in (end..=start).rev() {
            emit_into(n, &mut num, &mut batch, &mut batch_len);
        }
    }
    if batch_len > 0 {
        Uart10::puts_bytes(&batch[..batch_len]);
    }
    0
}

/// yes — repeat a string forever (or until interrupted)
fn cmd_yes(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    let text = if argc >= 2 { args[1] } else { "y" };
    for _ in 0..100 {
        crate::println!("{}", text);
    }
    Uart10::puts("(yes: stopped at 100 lines for safety)\r\n");
    0
}

/// tee — read from stdin (not implemented, just copy file to stdout + write to file)
fn cmd_tee(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: tee <file>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);
    let mut buf = [0u8; 512];
    let len = crate::drivers::serial_input::read_line_blocking(&mut buf);
    let text = core::str::from_utf8(&buf[..len]).unwrap_or("");
    crate::println!("{}", text);
    let _ = crate::fs::vfs::write_file(&path, text.as_bytes());
    crate::println!("tee: wrote {} bytes to {}", len, path);
    0
}

/// stat — show file statistics
fn cmd_stat(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: stat <path>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);
    match crate::fs::vfs::stat(&path) {
        Some(meta) => {
            crate::println!("  File: {}", path);
            let type_str = match meta.file_type {
                crate::fs::vfs::FileType::RegularFile => "regular file",
                crate::fs::vfs::FileType::Directory => "directory",
                crate::fs::vfs::FileType::Symlink => "symlink",
                crate::fs::vfs::FileType::Device => "device",
            };
            crate::println!("  Type: {}", type_str);
            crate::println!("  Size: {} bytes", meta.size);
            crate::println!("  Inode: {}", meta.inode_id);
            crate::println!("  Perm: {:#o}", meta.permissions);
            crate::println!("  UID: {}  GID: {}", meta.uid, meta.gid);
            0
        }
        None => {
            crate::println!("stat: {}: not found", path);
            1
        }
    }
}

/// file — show file type
fn cmd_file(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: file <path>\r\n");
        return 1;
    }
    let path = resolve_path(args[1]);

    match crate::fs::vfs::read_file(&path) {
        Ok(data) => {
            // Check magic bytes
            let ftype = if data.len() >= 4 {
                let magic = &data[..4];
                if magic == b"\x7fELF" {
                    "ELF executable (AArch64)"
                } else if &data[..2] == b"#!" {
                    "script"
                } else if data.starts_with(b"<") {
                    "XML/HTML"
                } else if data.starts_with(b"{") || data.starts_with(b"[") {
                    "JSON"
                } else if data.iter().all(|&b| b.is_ascii() || b == b'\n' || b == b'\r' || b == b'\t') {
                    "ASCII text"
                } else {
                    "binary data"
                }
            } else if data.is_empty() {
                "empty"
            } else {
                "ASCII text"
            };
            crate::println!("{}: {} ({} bytes)", path, ftype, data.len());
        }
        Err(_) => {
            // Check if it's a directory
            match crate::fs::vfs::list_dir(&path) {
                Ok(_) => crate::println!("{}: directory", path),
                Err(_) => crate::println!("{}: not found", path),
            }
        }
    }
    0
}

/// ln — create a link (simplified: copies file to new name)
fn cmd_ln(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 3 {
        Uart10::puts("Usage: ln <source> <target>\r\n");
        return 1;
    }
    let src = resolve_path(args[1]);
    let dst = resolve_path(args[2]);

    match crate::fs::vfs::read_file(&src) {
        Ok(data) => {
            match crate::fs::vfs::write_file(&dst, &data) {
                Ok(n) => crate::println!("ln: linked {} -> {} ({} bytes)", src, dst, n),
                Err(e) => crate::println!("ln: {:?}", e),
            }
            0
        }
        Err(e) => {
            crate::println!("ln: {}: {:?}", src, e);
            1
        }
    }
}

/// basename — strip directory and suffix from path
fn cmd_basename(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: basename <path> [suffix]\r\n");
        return 1;
    }
    let path = args[1];
    let suffix = if argc >= 3 { Some(args[2]) } else { None };
    let name = path.rsplit('/').next().unwrap_or(path);
    let result = match suffix {
        Some(s) => name.strip_suffix(s).unwrap_or(name),
        None => name,
    };
    crate::println!("{}", result);
    0
}

/// dirname — strip filename from path, leaving directory
fn cmd_dirname(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: dirname <path>\r\n");
        return 1;
    }
    let path = args[1];
    match path.rfind('/') {
        Some(0) => crate::println!("/"),
        Some(i) => crate::println!("{}", &path[..i]),
        None => crate::println!("."),
    }
    0
}

/// sleep — sleep for N seconds (using WFE)
fn cmd_sleep(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: sleep <seconds>\r\n");
        return 1;
    }
    let secs = args[1].parse::<u64>().unwrap_or(0);
    for _ in 0..secs {
        for _ in 0..1_000_000 {
            crate::arch::cpu::wfe();
        }
    }
    crate::println!("slept {} seconds", secs);
    0
}

/// man — show manual page for a command
fn cmd_man(args: [&str; MAX_ARGS], argc: usize) -> i32 {
    if argc < 2 {
        Uart10::puts("Usage: man <command>\r\n");
        return 1;
    }
    let cmd = args[1];
    let docs = [
        ("vi", "vi <file> — Line-based text editor.\nCommands: . save&quit, :w save, :q quit, :l list, :d<N> delete, :i<N> insert"),
        ("cc", "cc <file.c> [-o out] [-run] — Compile C source via lobster-net.\nThe C source is sent to the host compiler (aarch64-linux-gnu-gcc) and the resulting ELF is loaded into VFS."),
        ("curl", "curl [-o file] [-s] <url> — HTTP GET via lobster-net MMIO.\nThe request is proxied to the host's curl binary by the QEMU device."),
        ("run", "run <path|demo> — Load ELF from VFS and execute as EL0 user process."),
        ("grep", "grep <pattern> <file> — Search for pattern in file (substring match)."),
        ("pkg", "pkg <install|remove|list> — Package manager (uses lobster-net fetch)."),
        ("ls", "ls [-l|-a] [path] — List directory contents."),
        ("cat", "cat <file> — Display file contents."),
        ("rm", "rm <path> — Remove file or directory (recursive)."),
        ("cp", "cp <src> <dst> — Copy file in VFS."),
        ("echo", "echo <text> — Print text to console."),
        ("ps", "ps — List running processes (tasks)."),
        ("help", "help — Show all available commands."),
    ];

    for (name, doc) in &docs {
        if *name == cmd {
            crate::println!("{}\n{}", name.to_uppercase() , doc);
            return 0;
        }
    }
    crate::println!("man: no manual for {}", cmd);
    1
}

/// who — show logged-in users
fn cmd_who() -> i32 {
    let pid = crate::sched::current_pid();
    crate::println!("USER      TTY      PID     LOGIN@");
    crate::println!("root      tty1     1       (boot)");
    crate::println!("(current PID: {})", pid);
    0
}

