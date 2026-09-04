//! Track and track info as they appear on the wire.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A loaded, base64 encoded track plus its decoded metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    /// The base64 encoded track data.
    pub encoded: String,
    /// Decoded track information.
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

    /// Encode an engine track, with the source's own plugin info, empty user data and position
    /// `0`.
    pub fn from_engine(
        manager: &player::AudioPlayerManager,
        track: &dyn player::AudioTrack,
    ) -> player::Result<Self> {
        let encoded = manager.encode_track(track)?;
        let info = TrackInfo::from_engine(track.info(), track.source_name(), 0);
        Ok(Self {
            plugin_info: track.plugin_info(),
            ..Self::new(encoded, info)
        })
    }
}

/// Decoded metadata for a [`Track`], with `length` and `position` in milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    /// The track identifier, specific to the source it came from.
    pub identifier: String,
    /// Whether the track is seekable.
    pub is_seekable: bool,
    /// The track author.
    pub author: String,
    /// The track length in milliseconds.
    pub length: i64,
    /// Whether the track is a live stream.
    pub is_stream: bool,
    /// The current track position in milliseconds.
    pub position: i64,
    /// The track title.
    pub title: String,
    /// The track URI, if any.
    pub uri: Option<String>,
    /// The name of the source manager the track came from, such as `"http"` or `"local"`.
    pub source_name: String,
    /// The track artwork URL, if any.
    pub artwork_url: Option<String>,
    /// The track ISRC, if any.
    pub isrc: Option<String>,
}

impl TrackInfo {
    /// Build track info from the engine's own, at `position` milliseconds.
    ///
    /// The engine's unknown-duration sentinel for live streams becomes `i64::MAX` on the wire.
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

#[cfg(test)]
mod tests {
    use super::*;

    // A source's `pluginInfo` is the whole point of the field, and an empty map still has to
    // serialize as `{}` rather than vanish.
    #[test]
    fn plugin_info_survives_serialization() {
        let info = player::AudioTrackInfo::builder("id").title("t").build();
        let mut track = Track::new(
            "encoded".to_string(),
            TrackInfo::from_engine(&info, "deezer", 0),
        );
        assert!(serde_json::to_string(&track)
            .unwrap()
            .contains(r#""pluginInfo":{}"#));

        track
            .plugin_info
            .insert("albumName".into(), Value::from("Some Album"));
        let json = serde_json::to_string(&track).unwrap();
        assert!(
            json.contains(r#""pluginInfo":{"albumName":"Some Album"}"#),
            "{json}"
        );
    }
}
