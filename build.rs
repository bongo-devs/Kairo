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
    let player_version = locked_version("player").unwrap_or_else(|| "unknown".into());
    let sources_version = locked_version("sources").unwrap_or_else(|| "unknown".into());
    let lyrics_version = locked_version("lyrics").unwrap_or_else(|| "unknown".into());
    let voice_version = locked_version("voice").unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=KAIRO_GIT_BRANCH={branch}");
    println!("cargo:rustc-env=KAIRO_GIT_COMMIT={commit}");
    println!("cargo:rustc-env=KAIRO_GIT_COMMIT_TIME={commit_time_ms}");
    println!("cargo:rustc-env=KAIRO_BUILD_TIME={build_time_ms}");
    println!("cargo:rustc-env=KAIRO_PLAYER_VERSION={player_version}");
    println!("cargo:rustc-env=KAIRO_SOURCES_VERSION={sources_version}");
    println!("cargo:rustc-env=KAIRO_LYRICS_VERSION={lyrics_version}");
    println!("cargo:rustc-env=KAIRO_VOICE_VERSION={voice_version}");

    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=Cargo.lock");
}

/// The resolved version of a locked dependency. A git-pinned crate's version is not exposed to its
/// dependents at compile time, so the lockfile is the one place to read the version actually built
/// without the crate having to export it itself.
fn locked_version(package: &str) -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let lock = std::fs::read_to_string(format!("{manifest_dir}/Cargo.lock")).ok()?;
    let mut in_target = false;
    for line in lock.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix("name = \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            in_target = name == package;
        } else if in_target {
            if let Some(version) = line
                .strip_prefix("version = \"")
                .and_then(|s| s.strip_suffix('"'))
            {
                return Some(version.to_string());
            }
        }
    }
    None
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
