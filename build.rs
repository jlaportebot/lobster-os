//! Embeds the git commit as `LOBSTER_GIT_REV` so Settings > About can show
//! which kernel build is running. Falls back to nothing outside a checkout.

use std::process::Command;

fn main() {
    let rev = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok());
    if let Some(rev) = rev {
        println!("cargo:rustc-env=LOBSTER_GIT_REV={}", rev.trim());
    }
}
