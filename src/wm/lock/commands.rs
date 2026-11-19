//! Serial-console commands for the lock screen, log out, the CSPRNG and
//! password-hash timing. The shell only dispatches here and prints the text.

use alloc::string::String;
use alloc::vec::Vec;

/// Iterations used by `authbench` when none is given.
const DEFAULT_BENCH_ITERATIONS: u32 = crate::users::password::PBKDF2_ITERATIONS;
/// Bytes shown by `random bytes`.
const SAMPLE_BYTES: usize = 16;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| alloc::format!("{b:02x}")).collect()
}

fn random_command(args: &[&str]) -> (i32, String) {
    match args.first().copied() {
        Some("status") => (
            0,
            alloc::format!("{}\r\n", crate::security::random::status()),
        ),
        Some("bytes") => {
            let mut buf = [0u8; SAMPLE_BYTES];
            crate::security::random::fill(&mut buf);
            (0, alloc::format!("{}\r\n", hex(&buf)))
        }
        _ => (1, String::from("Usage: random status|bytes\r\n")),
    }
}

fn bench_command(args: &[&str]) -> (i32, String) {
    let iterations = args
        .first()
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(DEFAULT_BENCH_ITERATIONS);
    let salt = [0x5Au8; crate::users::password::SALT_LEN];
    let start = crate::arch::timer::Timer::uptime_ms();
    let hash = crate::users::password::hash_with("authbench", iterations, &salt);
    let ms = crate::arch::timer::Timer::uptime_ms().saturating_sub(start);
    let ok = hash.starts_with("$pbkdf2-sha256$");
    (
        i32::from(!ok),
        alloc::format!("pbkdf2-sha256 {iterations} iterations: {ms} ms\r\n"),
    )
}

/// Run a session command; returns the exit code and the text to print.
pub fn run(cmd: &str, args: &[&str]) -> (i32, String) {
    match cmd {
        "lock" => {
            super::lock();
            (0, String::from("session locked\r\n"))
        }
        "logout" => {
            super::log_out();
            (0, String::from("logging out\r\n"))
        }
        "random" => random_command(args),
        "authbench" => bench_command(args),
        _ => (127, String::new()),
    }
}

/// Names handled by [`run`].
pub const NAMES: [&str; 4] = ["lock", "logout", "random", "authbench"];

/// Convenience for the shell: args after the command name.
pub fn run_with_args(cmd: &str, all_args: &[&str]) -> (i32, String) {
    let rest: Vec<&str> = all_args.iter().skip(1).copied().collect();
    run(cmd, &rest)
}
