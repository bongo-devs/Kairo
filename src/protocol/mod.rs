//! The Lavalink v4 JSON wire protocol.
//!
//! Every type here serialises to the same JSON a Lavalink v4 node produces, down to field casing
//! (`#[serde(rename_all = "camelCase")]` plus explicit renames where they differ), so existing
//! clients work unchanged.
//!
//! Payloads that patch existing state use [`Omissible`](omissible::Omissible) to tell an absent
//! field, meaning leave unchanged, from a `null` one, meaning clear.

pub mod filters;
pub mod omissible;
pub mod track;

pub use filters::Filters;
pub use omissible::Omissible;
pub use track::{Track, TrackInfo};
