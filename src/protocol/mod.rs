//! The Lavalink v4 wire protocol; types match a v4 node's JSON field casing, so clients work unchanged.
//! Patch payloads use [`Omissible`](omissible::Omissible): absent leaves unchanged, `null` clears.

pub mod filters;
pub mod info;
pub mod load_result;
pub mod lyrics;
pub mod message;
pub mod omissible;
pub mod player;
pub mod routeplanner;
pub mod search;
pub mod session;
pub mod stats;
pub mod track;

pub use filters::Filters;
pub use info::{Git, Info, Plugin, Version};
pub use load_result::{Exception, LoadResult, Playlist, PlaylistInfo, Severity};
pub use lyrics::{Line, Lyrics};
pub use message::{EmittedEvent, Message, TrackEndReason};
pub use omissible::Omissible;
pub use player::{
    CrossfadeCurve, CrossfadeSettings, Player, PlayerState, PlayerUpdate, PlayerUpdateTrack,
    VoiceState,
};
pub use search::{SearchResult, SearchText};
pub use session::{Session, SessionUpdate};
pub use stats::{Cpu, FrameStats, Memory, Stats};
pub use track::{Track, TrackInfo};
