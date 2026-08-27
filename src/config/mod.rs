//! Configuration parsed from `application.yml` at startup.

mod filters;
mod lavalink;

pub use filters::FiltersToggleConfig;
pub use lavalink::{
    HttpConfig, LavalinkConfig, LavalinkServerConfig, RatelimitConfig, RatelimitStrategy,
    ResamplingQuality,
};
