//! `lyrics.*`, the lyrics lookup feature.
//!
//! The provider settings live in the [`lyrics`](::lyrics) crate next to the providers themselves
//! and are flattened in here, so adding a provider never touches this crate.
//!
//! ```yaml
//! lyrics:
//!   enabled: true          # master switch, off means every lyrics endpoint answers 503
//!   lrclib: true           # per-provider toggles, all opt-in
//!   youtubemusic: true
//!   deezerProxy:           # optional proxy for the Deezer lyrics provider
//!     url: "http://host:3128"
//! ```

use std::sync::Arc;

use serde::Deserialize;

use ::lyrics::{LyricsConfig, LyricsService};

/// The master switch plus the per-provider [`LyricsConfig`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LyricsServerConfig {
    /// Master switch. With it off the lyrics service is never built and every lyrics endpoint
    /// answers `503 Service Unavailable`.
    pub enabled: bool,
    /// The per-provider configuration, flattened so `lyrics.lrclib` and `lyrics.deezerProxy` sit
    /// directly under `lyrics.*`.
    #[serde(flatten)]
    pub providers: LyricsConfig,
}

impl LyricsServerConfig {
    /// Build the [`LyricsService`], or `None` when lyrics are off or no provider is enabled. `None`
    /// makes every lyrics endpoint report the feature as unavailable.
    pub fn build_service(&self) -> Option<Arc<LyricsService>> {
        if !self.enabled {
            return None;
        }
        self.providers.build_service()
    }

    /// The names of the enabled providers, as logged at startup and reported by `GET /v4/info`.
    pub fn provider_names(&self) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        self.providers.provider_names()
    }
}
