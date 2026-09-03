//! Lyrics response types: outbound only, so [`Serialize`] alone. The `plugin` field is part of the
//! wire format and always carries an empty object here.

use serde::Serialize;

use ::lyrics::LyricsData;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The source the track came from, such as `youtube` or `spotify`.
    pub source_name: String,
    /// The provider that produced this result, such as `lrclib`.
    pub provider: Option<String>,
    pub text: Option<String>,
    /// Timed lyric lines, if the provider returned synced lyrics.
    pub lines: Option<Vec<Line>>,
    pub plugin: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// Timestamp of the line, in milliseconds.
    pub timestamp: u64,
    /// How long the line is shown, in milliseconds, when known.
    pub duration: Option<u64>,
    pub line: String,
    pub plugin: serde_json::Value,
}

impl Line {
    pub fn from_engine(line: &::lyrics::LyricsLine) -> Self {
        Line {
            timestamp: line.timestamp,
            // A zero duration means unknown, which is what an unsynced parse yields.
            duration: (line.duration != 0).then_some(line.duration),
            line: line.text.clone(),
            plugin: serde_json::json!({}),
        }
    }
}

impl Lyrics {
    pub fn from_data(data: &LyricsData) -> Self {
        Lyrics {
            source_name: data.source_name.clone(),
            provider: Some(data.provider.clone()),
            text: (!data.text.is_empty()).then(|| data.text.clone()),
            lines: data
                .lines
                .as_ref()
                .map(|lines| lines.iter().map(Line::from_engine).collect()),
            plugin: serde_json::json!({}),
        }
    }
}
