//! Sentry client setup. The `tracing` layer that feeds it lives in [`super::logging`].

use std::borrow::Cow;

use crate::config::SentryConfig;

/// Start the Sentry client, or return `None` when no DSN is configured. The guard flushes pending
/// events when dropped.
pub fn init_sentry(cfg: &SentryConfig) -> Option<sentry::ClientInitGuard> {
    if !cfg.is_enabled() {
        return None;
    }

    // Sentry expects `name@version`, and the build's commit distinguishes two builds of one version.
    let commit = env!("KAIRO_GIT_COMMIT");
    let release: Cow<'static, str> = if commit.is_empty() {
        concat!("kairo@", env!("CARGO_PKG_VERSION")).into()
    } else {
        format!("kairo@{}+{}", env!("CARGO_PKG_VERSION"), commit).into()
    };
    let environment = (!cfg.environment.is_empty()).then(|| Cow::Owned(cfg.environment.clone()));

    let options = sentry::ClientOptions {
        release: Some(release),
        environment,
        ..Default::default()
    };
    let guard = sentry::init((cfg.dsn.clone(), options));

    if !cfg.tags.is_empty() {
        let tags = cfg.tags.clone();
        sentry::configure_scope(|scope| {
            for (key, value) in &tags {
                scope.set_tag(key, value);
            }
        });
    }

    Some(guard)
}
