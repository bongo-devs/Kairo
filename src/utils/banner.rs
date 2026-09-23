//! The startup banner printed to the console before logging begins.

use crate::config::{LogFormat, LoggingConfig};
use crate::utils::ansi::{ACCENT, BOLD, DIM, RESET};

const LOGO: &str = "\
██╗  ██╗ █████╗ ██╗██████╗  ██████╗
██║ ██╔╝██╔══██╗██║██╔══██╗██╔═══██╗
█████╔╝ ███████║██║██████╔╝██║   ██║
██╔═██╗ ██╔══██║██║██╔══██╗██║   ██║
██║  ██╗██║  ██║██║██║  ██║╚██████╔╝
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝╚═╝  ╚═╝ ╚═════╝";

const TAGLINE: &str = "Standalone Discord audio sending node · Lavalink v4 compatible";

/// Print the logo, version and a short facts line to stdout.
///
/// Written directly rather than through `tracing` so it lands before the subscriber is installed
/// and never carries a log prefix. Skipped when the banner is turned off, and always when the log
/// format is JSON: stdout is a machine-readable stream then, and a banner would corrupt it.
pub fn print(cfg: &LoggingConfig) {
    if !cfg.banner || cfg.format == LogFormat::Json {
        return;
    }

    let (accent, bold, dim, reset) = if cfg.color {
        (ACCENT, BOLD, DIM, RESET)
    } else {
        ("", "", "", "")
    };

    let cores = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let facts = format!(
        "v{}  ·  {} cores  ·  {}/{}",
        env!("CARGO_PKG_VERSION"),
        cores,
        std::env::consts::OS,
        std::env::consts::ARCH,
    );

    println!("\n{accent}{bold}{LOGO}{reset}\n");
    println!("  {dim}{TAGLINE}{reset}");
    println!("  {dim}{facts}{reset}\n");
}
