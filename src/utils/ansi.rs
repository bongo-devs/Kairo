//! The ANSI escapes the console output shares, so the banner and notices read as one palette.

pub const ACCENT: &str = "\x1b[38;5;44m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const RESET: &str = "\x1b[0m";
