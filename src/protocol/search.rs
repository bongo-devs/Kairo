//! The LavaSearch `GET /v4/loadsearch` result.
//!
//! The payload carries `tracks`, `albums`, `artists`, `playlists`, `texts` and a `plugin` object.
//! Albums, artists and playlists come back as [`Playlist`]s with empty `tracks` and
//! `selectedTrack: -1`, so a search stays light and a client loads the contents on demand.

use serde::Serialize;
use serde_json::{Map, Value};

use player::search::AudioSearchResult;

use crate::protocol::load_result::Playlist;
use crate::protocol::track::Track;

/// A free-text search suggestion.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchText {
    /// The suggestion text.
    pub text: String,
    /// Plugin provided data, always an empty object here.
    #[serde(default)]
    pub plugin: Map<String, Value>,
}

/// The result of a categorised search.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Matched tracks.
    pub tracks: Vec<Track>,
    /// Matched albums, metadata only.
    pub albums: Vec<Playlist>,
    /// Matched artists, metadata only.
    pub artists: Vec<Playlist>,
    /// Matched playlists, metadata only.
    pub playlists: Vec<Playlist>,
    /// Free-text suggestions.
    pub texts: Vec<SearchText>,
    /// Plugin provided data, always an empty object here.
    #[serde(default)]
    pub plugin: Map<String, Value>,
}

impl SearchResult {
    /// Build the wire result from an engine [`AudioSearchResult`], encoding every matched track.
    pub fn from_engine(
        manager: &player::AudioPlayerManager,
        result: AudioSearchResult,
    ) -> player::Result<Self> {
        let tracks = result
            .tracks
            .iter()
            .map(|track| Track::from_engine(manager, track.as_ref()))
            .collect::<player::Result<Vec<_>>>()?;

        let albums = result
            .albums
            .into_iter()
            .map(playlist_to_metadata)
            .collect();
        let artists = result
            .artists
            .into_iter()
            .map(playlist_to_metadata)
            .collect();
        let playlists = result
            .playlists
            .into_iter()
            .map(playlist_to_metadata)
            .collect();

        let texts = result
            .texts
            .into_iter()
            .map(|text| SearchText {
                text,
                plugin: Map::new(),
            })
            .collect();

        Ok(Self {
            tracks,
            albums,
            artists,
            playlists,
            texts,
            plugin: Map::new(),
        })
    }
}

// A search hit carries playlist metadata without any tracks, so the wire copy is empty.
fn playlist_to_metadata(playlist: player::track::playlist::AudioPlaylist) -> Playlist {
    Playlist {
        info: crate::protocol::load_result::PlaylistInfo {
            name: playlist.name,
            selected_track: -1,
        },
        plugin_info: playlist.plugin_info,
        tracks: Vec::new(),
    }
}
