//! The result of loading an identifier.
//!
//! The wire shape is `{ "loadType": <tag>, "data": <payload> }`, and the payload differs per tag: a
//! track, a playlist, an array of tracks, `null`, or an exception. [`Serialize`] is written by hand
//! so `empty` emits `"data": null` instead of dropping the key.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::protocol::track::Track;

/// The result of loading an identifier.
#[derive(Debug, Clone)]
pub enum LoadResult {
    /// A single track was loaded.
    Track(Track),
    /// A playlist was loaded.
    Playlist(Playlist),
    /// A search returned a list of tracks.
    Search(Vec<Track>),
    /// Nothing matched the identifier.
    Empty,
    /// Loading failed.
    Error(Exception),
}

impl LoadResult {
    fn load_type(&self) -> &'static str {
        match self {
            LoadResult::Track(_) => "track",
            LoadResult::Playlist(_) => "playlist",
            LoadResult::Search(_) => "search",
            LoadResult::Empty => "empty",
            LoadResult::Error(_) => "error",
        }
    }
}

impl Serialize for LoadResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("LoadResult", 2)?;
        state.serialize_field("loadType", self.load_type())?;
        match self {
            LoadResult::Track(track) => state.serialize_field("data", track)?,
            LoadResult::Playlist(playlist) => state.serialize_field("data", playlist)?,
            LoadResult::Search(tracks) => state.serialize_field("data", tracks)?,
            LoadResult::Empty => state.serialize_field("data", &Option::<()>::None)?,
            LoadResult::Error(exception) => state.serialize_field("data", exception)?,
        }
        state.end()
    }
}

/// A loaded playlist.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    /// Playlist metadata.
    pub info: PlaylistInfo,
    /// Plugin provided info, default `{}`.
    #[serde(default)]
    pub plugin_info: Map<String, Value>,
    /// The tracks in the playlist.
    pub tracks: Vec<Track>,
}

/// Playlist metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    /// The playlist name.
    pub name: String,
    /// The index of the selected track, or `-1` if none.
    pub selected_track: i32,
}

/// A load or playback error.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exception {
    /// A user-facing message, if any.
    pub message: Option<String>,
    /// The severity.
    pub severity: Severity,
    /// The technical cause.
    pub cause: String,
    /// The cause stack trace, which may be the same text as `cause`.
    pub cause_stack_trace: String,
}

impl Exception {
    pub fn from_friendly(error: &player::FriendlyException) -> Self {
        let cause = error.cause.clone().unwrap_or_else(|| error.message.clone());
        Self {
            message: Some(error.message.clone()),
            severity: Severity::from_player(error.severity),
            cause: cause.clone(),
            cause_stack_trace: cause,
        }
    }
}

/// Exception severity, serialised lowercase.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Known and expected, with nothing wrong on this side.
    Common,
    /// Possibly caused by outside factors, such as an unexpected upstream response.
    Suspicious,
    /// Probably a bug here, or the cause is unknown.
    Fault,
}

impl Severity {
    pub fn from_player(severity: player::Severity) -> Self {
        match severity {
            player::Severity::Common => Severity::Common,
            player::Severity::Suspicious => Severity::Suspicious,
            player::Severity::Fault => Severity::Fault,
        }
    }
}
