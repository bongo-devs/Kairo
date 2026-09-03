use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// The base64 encoded track data.
    pub encoded: String,
    pub info: TrackInfo,
    /// Additional track info provided by plugins, always present, default `{}`.
    #[serde(default)]
    pub plugin_info: Map<String, Value>,
    /// Additional track data provided via the player update `userData` field, default `{}`.
    #[serde(default)]
    pub user_data: Map<String, Value>,
}

impl Track {
    /// Build a track from an encoded string and its info, with empty plugin and user data.
    pub fn new(encoded: String, info: TrackInfo) -> Self {
        Self {
            encoded,
            info,
            plugin_info: Map::new(),
            user_data: Map::new(),
        }
    }

    /// Encode an engine track, with empty plugin and user data and position `0`.
    pub fn from_engine(
        manager: &player::AudioPlayerManager,
        track: &dyn player::AudioTrack,
    ) -> player::Result<Self> {
        let encoded = manager.encode_track(track)?;
        let info = TrackInfo::from_engine(track.info(), track.source_name(), 0);
        Ok(Self::new(encoded, info))
    }
}

/// Decoded metadata for a [`Track`], with `length` and `position` in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub identifier: String,
    pub is_seekable: bool,
    pub author: String,
    pub length: i64,
    pub is_stream: bool,
    pub position: i64,
    pub title: String,
    pub uri: Option<String>,
    /// The name of the source manager the track came from, such as `"http"` or `"local"`.
    pub source_name: String,
    pub artwork_url: Option<String>,
    pub isrc: Option<String>,
}

impl TrackInfo {
    /// Build track info from the engine's own, at `position` milliseconds. The engine's
    /// unknown-duration sentinel for live streams becomes `i64::MAX` on the wire.
    pub fn from_engine(info: &player::AudioTrackInfo, source_name: &str, position: i64) -> Self {
        let length = if info.length == u64::MAX {
            i64::MAX
        } else {
            info.length as i64
        };
        Self {
            identifier: info.identifier.clone(),
            is_seekable: !info.is_stream,
            author: info.author.clone(),
            length,
            is_stream: info.is_stream,
            position,
            title: info.title.clone(),
            uri: info.uri.clone(),
            source_name: source_name.to_string(),
            artwork_url: info.artwork_url.clone(),
            isrc: info.isrc.clone(),
        }
    }
}
