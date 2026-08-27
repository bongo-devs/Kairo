//! The `tracing` subscriber: a console sink, an optional rolling file, and the level filter.

use std::path::Path;

use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter, Layer, Registry};

use crate::config::{LogFileConfig, LogFormat, LogRotation, LoggingConfig};

// Local wall-clock time, without a date.
const TIME_FORMAT: &str = "%H:%M:%S";

/// Install the global subscriber. The returned guard flushes the file sink when dropped, so hold
/// it for as long as the process should keep logging.
pub fn init(cfg: &LoggingConfig, sentry_enabled: bool) -> Option<WorkerGuard> {
    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();

    layers.push(fmt_layer(cfg, cfg.color, std::io::stdout));

    // The file sink is never colorized, whatever the console does.
    let guard = cfg.file.as_ref().map(|file| {
        let (writer, guard) = tracing_appender::non_blocking(make_appender(file));
        layers.push(fmt_layer(cfg, false, writer));
        guard
    });

    // Only added when Sentry is configured, to keep the per-log cost off everyone else.
    if sentry_enabled {
        use sentry::integrations::tracing::EventFilter;
        let sentry_layer = sentry::integrations::tracing::layer().event_filter(|metadata| {
            match *metadata.level() {
                Level::ERROR | Level::WARN => EventFilter::Event,
                Level::INFO => EventFilter::Breadcrumb,
                _ => EventFilter::Ignore,
            }
        });
        layers.push(sentry_layer.boxed());
    }

    tracing_subscriber::registry()
        .with(layers)
        .with(build_filter(cfg))
        .init();

    guard
}

// `RUST_LOG` wins if set, otherwise the global level plus each per-target override, as in
// `info,voice=debug,player=debug`.
fn build_filter(cfg: &LoggingConfig) -> EnvFilter {
    if let Ok(env) = std::env::var("RUST_LOG") {
        if !env.trim().is_empty() {
            return EnvFilter::builder().parse_lossy(env);
        }
    }
    let mut directives = cfg.level.clone();
    for (target, level) in &cfg.levels {
        directives.push(',');
        directives.push_str(target);
        directives.push('=');
        directives.push_str(level);
    }
    EnvFilter::builder().parse_lossy(directives)
}

fn fmt_layer<W>(
    cfg: &LoggingConfig,
    ansi: bool,
    writer: W,
) -> Box<dyn Layer<Registry> + Send + Sync>
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let timer = ChronoLocal::new(TIME_FORMAT.to_owned());
    let base = fmt::layer()
        .with_writer(writer)
        .with_ansi(ansi)
        .with_target(cfg.show_target);

    // Every format and timestamp combination is a distinct type, so box at each leaf.
    match cfg.format {
        LogFormat::Compact => {
            let l = base.compact();
            if cfg.timestamps {
                l.with_timer(timer).boxed()
            } else {
                l.without_time().boxed()
            }
        }
        LogFormat::Pretty => {
            let l = base.pretty();
            if cfg.timestamps {
                l.with_timer(timer).boxed()
            } else {
                l.without_time().boxed()
            }
        }
        LogFormat::Json => {
            let l = base.json();
            if cfg.timestamps {
                l.with_timer(timer).boxed()
            } else {
                l.without_time().boxed()
            }
        }
    }
}

fn make_appender(file: &LogFileConfig) -> RollingFileAppender {
    let path = Path::new(&file.path);
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let prefix = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("kairo.log");
    // If the directory cannot be created the appender surfaces the error itself.
    let _ = std::fs::create_dir_all(dir);

    let rotation = match file.rotation {
        LogRotation::Daily => Rotation::DAILY,
        LogRotation::Hourly => Rotation::HOURLY,
        LogRotation::Never => Rotation::NEVER,
    };
    RollingFileAppender::new(rotation, dir, prefix)
}
