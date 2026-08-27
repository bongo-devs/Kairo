use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let commit_time_ms = git(&["log", "-1", "--format=%ct"])
        .and_then(|s| s.parse::<i64>().ok())
        .map(|secs| secs * 1000)
        .unwrap_or(0);
    let build_time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    println!("cargo:rustc-env=KAIRO_GIT_BRANCH={branch}");
    println!("cargo:rustc-env=KAIRO_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=KAIRO_GIT_COMMIT_TIME={commit_time_ms}");
    println!("cargo:rustc-env=KAIRO_BUILD_TIME={build_time_ms}");

    println!("cargo:rerun-if-changed=.git/HEAD");
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
