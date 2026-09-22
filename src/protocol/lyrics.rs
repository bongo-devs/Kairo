//! Lyrics response types.
//!
//! These only ever travel to the client, so they derive [`Serialize`] alone. The `plugin` field is
//! part of the wire format. A Lavalink v4 client ignores it, so word timings and backing vocals
//! ride there without breaking one: a line with either carries
//! `plugin.kairo.words` (`[{timestamp, duration, text}]`) and/or `plugin.kairo.background` (a nested
//! line of the same shape). A line with neither leaves `plugin` an empty object, as before.

use serde::Serialize;

use ::lyrics::{LyricsData, LyricsWord};

/// A resolved lyrics result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The source the track came from, such as `youtube` or `spotify`.
    pub source_name: String,
    /// The provider that produced this result, such as `lrclib`.
    pub provider: Option<String>,
    /// The full plain text lyrics, if available.
    pub text: Option<String>,
    /// Timed lyric lines, if the provider returned synced lyrics.
    pub lines: Option<Vec<Line>>,
    /// Plugin metadata. Always an empty object at the top level.
    pub plugin: serde_json::Value,
}

/// A lyric line, timed when the provider gave timings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// Timestamp of the line, in milliseconds.
    pub timestamp: u64,
    /// How long the line is shown, in milliseconds, when known.
    pub duration: Option<u64>,
    /// The line text.
    pub line: String,
    /// Plugin metadata. Carries `kairo.words` and/or `kairo.background` when the line has them,
    /// otherwise an empty object.
    pub plugin: serde_json::Value,
}

fn words_json(words: &[LyricsWord]) -> serde_json::Value {
    serde_json::Value::Array(
        words
            .iter()
            .map(|w| {
                serde_json::json!({
                    "timestamp": w.timestamp,
                    "duration": w.duration,
                    "text": w.text,
                })
            })
            .collect(),
    )
}

impl Line {
    pub fn from_engine(line: &::lyrics::LyricsLine) -> Self {
        let mut kairo = serde_json::Map::new();
        if let Some(words) = line.words.as_ref().filter(|w| !w.is_empty()) {
            kairo.insert("words".to_owned(), words_json(words));
        }
        if let Some(background) = line.background.as_deref() {
            let nested = Line::from_engine(background);
            kairo.insert(
                "background".to_owned(),
                serde_json::to_value(&nested).unwrap_or_else(|_| serde_json::json!({})),
            );
        }

        let plugin = if kairo.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "kairo": kairo })
        };

        Line {
            timestamp: line.timestamp,
            // A zero duration means unknown, which is what an unsynced parse yields.
            duration: (line.duration != 0).then_some(line.duration),
            line: line.text.clone(),
            plugin,
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

#[cfg(test)]
mod tests {
    use ::lyrics::LyricsLine;

    use super::*;

    #[test]
    fn a_plain_line_keeps_an_empty_plugin() {
        let line = Line::from_engine(&LyricsLine::line(1_000, 500, "hello".to_owned()));
        assert_eq!(line.plugin, serde_json::json!({}));
        assert_eq!(line.duration, Some(500));
    }

    #[test]
    fn a_gap_marker_is_an_empty_line_at_its_timestamp() {
        // A gap marker has empty text and unknown duration; it still carries its own timestamp.
        let line = Line::from_engine(&LyricsLine::line(4_000, 0, String::new()));
        assert_eq!(line.line, "");
        assert_eq!(line.timestamp, 4_000);
        assert_eq!(line.duration, None);
        assert_eq!(line.plugin, serde_json::json!({}));
    }

    #[test]
    fn word_timings_ride_under_the_plugin_key() {
        let engine = LyricsLine {
            words: Some(vec![
                LyricsWord {
                    timestamp: 1_000,
                    duration: 300,
                    text: "don't".to_owned(),
                },
                LyricsWord {
                    timestamp: 1_300,
                    duration: 200,
                    text: "stop".to_owned(),
                },
            ]),
            ..LyricsLine::line(1_000, 500, "don't stop".to_owned())
        };
        let line = Line::from_engine(&engine);
        let words = &line.plugin["kairo"]["words"];
        assert_eq!(words[0]["text"], "don't");
        assert_eq!(words[1]["timestamp"], 1_300);
        // A v4 client that ignores `plugin` still sees the joined text.
        assert_eq!(line.line, "don't stop");
    }

    #[test]
    fn a_backing_vocal_nests_a_line_of_the_same_shape() {
        let background = LyricsLine::line(2_000, 400, "(ooh)".to_owned());
        let engine = LyricsLine {
            background: Some(Box::new(background)),
            ..LyricsLine::line(2_000, 400, "hold on".to_owned())
        };
        let line = Line::from_engine(&engine);
        assert_eq!(line.plugin["kairo"]["background"]["line"], "(ooh)");
        assert_eq!(line.plugin["kairo"]["background"]["timestamp"], 2_000);
    }
}
