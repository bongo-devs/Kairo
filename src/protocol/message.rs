//! Outbound WebSocket messages: client-bound only, so [`Serialize`] alone. Internally tagged on
//! `op` (`ready`, `playerUpdate`, `stats`, `event`), and an `event` is tagged again on `type`.

use serde::Serialize;

use crate::protocol::load_result::Exception;
use crate::protocol::lyrics::{Line, Lyrics};
use crate::protocol::player::PlayerState;
use crate::protocol::stats::{FrameStats, Stats};
use crate::protocol::track::Track;
use crate::sponsorblock::{Chapter, Segment};

/// A message sent to a connected client over the WebSocket.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "op")]
pub enum Message {
    /// First message after the socket opens.
    #[serde(rename = "ready", rename_all = "camelCase")]
    Ready {
        resumed: bool,
        /// The session id to use for REST calls.
        session_id: String,
    },
    /// Periodic player state.
    #[serde(rename = "playerUpdate", rename_all = "camelCase")]
    PlayerUpdate {
        guild_id: String,
        state: PlayerState,
    },
    /// Periodic node statistics.
    ///
    /// `frameStats` is hoisted out of the inner [`Stats`] because this op always encodes it, as
    /// `null` when the node has no players, while the `GET /v4/stats` body leaves it out entirely.
    /// Build it through [`Message::stats`], which keeps the two in step.
    #[serde(rename = "stats", rename_all = "camelCase")]
    Stats {
        /// `null` when the node has no players.
        frame_stats: Option<FrameStats>,
        #[serde(flatten)]
        stats: Stats,
    },
    /// Boxed, since events are far larger than the other variants.
    #[serde(rename = "event")]
    Event(Box<EmittedEvent>),
}

impl Message {
    pub fn event(event: EmittedEvent) -> Self {
        Message::Event(Box::new(event))
    }

    /// Hoists `frameStats` out of `stats` so it is encoded exactly once.
    pub fn stats(mut stats: Stats) -> Self {
        let frame_stats = stats.frame_stats.take();
        Message::Stats { frame_stats, stats }
    }
}

/// A player event, tagged on `type`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum EmittedEvent {
    #[serde(rename = "TrackStartEvent", rename_all = "camelCase")]
    TrackStart {
        guild_id: String,
        track: Track,
    },
    /// A pre-buffered successor became the active track. The server is already playing it, so a
    /// client advances its queue state and re-anchors position without sending a play request.
    #[serde(rename = "TrackPromotedEvent", rename_all = "camelCase")]
    TrackPromoted {
        guild_id: String,
        track: Track,
    },
    #[serde(rename = "TrackEndEvent", rename_all = "camelCase")]
    TrackEnd {
        guild_id: String,
        track: Track,
        reason: TrackEndReason,
    },
    /// A track threw an exception, which may or may not be fatal.
    #[serde(rename = "TrackExceptionEvent", rename_all = "camelCase")]
    TrackException {
        guild_id: String,
        track: Track,
        exception: Exception,
    },
    /// A track got stuck, producing no frames within the threshold.
    #[serde(rename = "TrackStuckEvent", rename_all = "camelCase")]
    TrackStuck {
        guild_id: String,
        track: Track,
        threshold_ms: i64,
    },
    /// The Discord voice WebSocket closed.
    #[serde(rename = "WebSocketClosedEvent", rename_all = "camelCase")]
    WebSocketClosed {
        guild_id: String,
        /// The Discord voice close code.
        code: i32,
        reason: String,
        /// Whether Discord initiated the close.
        by_remote: bool,
    },
    #[serde(rename = "SegmentSkipped", rename_all = "camelCase")]
    SegmentSkipped {
        guild_id: String,
        segment: Segment,
    },
    /// SponsorBlock segments were loaded for a track.
    #[serde(rename = "SegmentsLoaded", rename_all = "camelCase")]
    SegmentsLoaded {
        guild_id: String,
        segments: Vec<Segment>,
    },
    #[serde(rename = "ChapterStarted", rename_all = "camelCase")]
    ChapterStarted {
        guild_id: String,
        chapter: Chapter,
    },
    /// Video chapters were loaded for a track.
    #[serde(rename = "ChaptersLoaded", rename_all = "camelCase")]
    ChaptersLoaded {
        guild_id: String,
        chapters: Vec<Chapter>,
    },
    /// Lyrics were found for the current track.
    #[serde(rename = "LyricsFoundEvent", rename_all = "camelCase")]
    LyricsFound {
        guild_id: String,
        lyrics: Lyrics,
    },
    /// No lyrics were found for the current track.
    #[serde(rename = "LyricsNotFoundEvent", rename_all = "camelCase")]
    LyricsNotFound {
        guild_id: String,
    },
    /// A synced lyric line was reached during playback.
    #[serde(rename = "LyricsLineEvent", rename_all = "camelCase")]
    LyricsLine {
        guild_id: String,
        /// The index of the line within the lyrics.
        line_index: i32,
        line: Line,
        /// Whether the line was passed over, by a seek or by arriving late, rather than reached in
        /// real time.
        skipped: bool,
    },
}

/// Why a track stopped.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackEndReason {
    /// The track reached its end, or an end marker stopped it.
    Finished,
    /// The track failed to start, throwing before producing audio.
    LoadFailed,
    Stopped,
    Replaced,
    /// The player was cleaned up after the idle threshold.
    Cleanup,
    /// The track was crossfaded into its successor, which is already playing, so a client advances
    /// its queue state without sending a new play request.
    Crossfade,
    /// The track was handed off gaplessly to a pre-buffered successor, which is already playing.
    Gapless,
}

impl TrackEndReason {
    pub fn from_player(reason: player::track::state::AudioTrackEndReason) -> Self {
        use player::track::state::AudioTrackEndReason as R;
        match reason {
            R::Finished => TrackEndReason::Finished,
            R::LoadFailed => TrackEndReason::LoadFailed,
            R::Stopped => TrackEndReason::Stopped,
            R::Replaced => TrackEndReason::Replaced,
            R::Cleanup => TrackEndReason::Cleanup,
            R::Crossfade => TrackEndReason::Crossfade,
            R::Gapless => TrackEndReason::Gapless,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::stats::{Cpu, Memory};

    fn sample_stats(frame_stats: Option<FrameStats>) -> Stats {
        Stats {
            frame_stats,
            players: 1,
            playing_players: 1,
            uptime: 1000,
            memory: Memory {
                free: 1,
                used: 2,
                allocated: 3,
                reservable: 4,
            },
            cpu: Cpu {
                cores: 8,
                system_load: 0.1,
                lavalink_load: 0.2,
            },
        }
    }

    // The `stats` op has to carry `frameStats` exactly once, `null` included, and the flatten plus
    // hoist in `Message::stats` is easy to break without noticing.
    #[test]
    fn ws_stats_always_encodes_frame_stats() {
        let json = serde_json::to_string(&Message::stats(sample_stats(None))).unwrap();
        assert!(json.contains(r#""op":"stats""#), "{json}");
        assert!(json.contains(r#""frameStats":null"#), "{json}");

        let frames = FrameStats {
            sent: 3000,
            nulled: 0,
            deficit: 0,
        };
        let json = serde_json::to_string(&Message::stats(sample_stats(Some(frames)))).unwrap();
        assert_eq!(json.matches("frameStats").count(), 1, "{json}");
        assert!(json.contains(r#""sent":3000"#), "{json}");

        // The REST body leaves it out instead.
        let json = serde_json::to_string(&sample_stats(None)).unwrap();
        assert!(!json.contains("frameStats"), "{json}");
    }
}
