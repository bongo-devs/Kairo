//! Wire types only. The DSP lives in the [`player`] crate, and [`Filters::to_engine`] hands the
//! payload over through a JSON hop since the field names line up. A present sub-filter serialises
//! all of its fields, defaults included; one never set stays `None` and is left out.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    /// Filter volume multiplier, separate from the player's own volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equalizer: Option<Vec<Band>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub karaoke: Option<Karaoke>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timescale: Option<Timescale>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tremolo: Option<Tremolo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vibrato: Option<Vibrato>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distortion: Option<Distortion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_mix: Option<ChannelMix>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_pass: Option<LowPass>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub plugin_filters: Map<String, Value>,
}

impl Filters {
    /// The wire names of the filters set here, plugin filter keys included.
    pub fn set_filter_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.volume.is_some() {
            names.push("volume".to_string());
        }
        if self.equalizer.is_some() {
            names.push("equalizer".to_string());
        }
        if self.karaoke.is_some() {
            names.push("karaoke".to_string());
        }
        if self.timescale.is_some() {
            names.push("timescale".to_string());
        }
        if self.tremolo.is_some() {
            names.push("tremolo".to_string());
        }
        if self.vibrato.is_some() {
            names.push("vibrato".to_string());
        }
        if self.distortion.is_some() {
            names.push("distortion".to_string());
        }
        if self.rotation.is_some() {
            names.push("rotation".to_string());
        }
        if self.channel_mix.is_some() {
            names.push("channelMix".to_string());
        }
        if self.low_pass.is_some() {
            names.push("lowPass".to_string());
        }
        names.extend(self.plugin_filters.keys().cloned());
        names
    }

    /// Convert into the engine's filter config; plugin filters are ignored by the engine. Every
    /// engine field has a serde default, so schema drift silently resets the chain, hence the log.
    pub fn to_engine(&self) -> player::filter::config::FilterConfig {
        match serde_json::to_value(self).and_then(serde_json::from_value) {
            Ok(config) => config,
            Err(err) => {
                tracing::error!(
                    %err,
                    "filter payload does not match the engine schema; filters not applied"
                );
                player::filter::config::FilterConfig::default()
            }
        }
    }
}

/// A single equalizer band: `{ "band": 0..=14, "gain": -0.25..=1.0 }`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Band {
    pub band: u32,
    #[serde(default = "one_f32")]
    pub gain: f32,
}

/// Karaoke, or vocal removal, filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Karaoke {
    #[serde(default = "one_f32")]
    pub level: f32,
    #[serde(default = "one_f32")]
    pub mono_level: f32,
    /// Filter band in Hz.
    #[serde(default = "f32_220")]
    pub filter_band: f32,
    #[serde(default = "f32_100")]
    pub filter_width: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Timescale {
    #[serde(default = "one_f64")]
    pub speed: f64,
    #[serde(default = "one_f64")]
    pub pitch: f64,
    #[serde(default = "one_f64")]
    pub rate: f64,
}

/// Tremolo, or amplitude modulation, filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Tremolo {
    /// Frequency in Hz.
    #[serde(default = "f32_2")]
    pub frequency: f32,
    /// Depth `0.0..=1.0`.
    #[serde(default = "f32_half")]
    pub depth: f32,
}

/// Vibrato, or pitch modulation, filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Vibrato {
    /// Frequency in Hz.
    #[serde(default = "f32_2")]
    pub frequency: f32,
    /// Depth `0.0..=1.0`.
    #[serde(default = "f32_half")]
    pub depth: f32,
}

/// Rotation, or 8D audio, filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rotation {
    /// Rotation speed in Hz.
    #[serde(default)]
    pub rotation_hz: f64,
}

/// Trig waveshaper distortion filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distortion {
    #[serde(default)]
    pub sin_offset: f32,
    #[serde(default = "one_f32")]
    pub sin_scale: f32,
    #[serde(default)]
    pub cos_offset: f32,
    #[serde(default = "one_f32")]
    pub cos_scale: f32,
    #[serde(default)]
    pub tan_offset: f32,
    #[serde(default = "one_f32")]
    pub tan_scale: f32,
    #[serde(default)]
    pub offset: f32,
    #[serde(default = "one_f32")]
    pub scale: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMix {
    #[serde(default = "one_f32")]
    pub left_to_left: f32,
    #[serde(default)]
    pub left_to_right: f32,
    #[serde(default)]
    pub right_to_left: f32,
    #[serde(default = "one_f32")]
    pub right_to_right: f32,
}

/// One-pole low-pass filter.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LowPass {
    #[serde(default = "f32_20")]
    pub smoothing: f32,
}

// serde's `default = ` takes a function, so there is one per value.
fn one_f32() -> f32 {
    1.0
}
fn one_f64() -> f64 {
    1.0
}
fn f32_half() -> f32 {
    0.5
}
fn f32_2() -> f32 {
    2.0
}
fn f32_20() -> f32 {
    20.0
}
fn f32_100() -> f32 {
    100.0
}
fn f32_220() -> f32 {
    220.0
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every value below is non-default, so a field name that drifts from the engine's falls back
    // to the engine default and trips an assert.
    #[test]
    fn filters_round_trip_to_engine() {
        let filters = Filters {
            volume: Some(0.5),
            equalizer: Some(vec![Band {
                band: 3,
                gain: 0.25,
            }]),
            karaoke: Some(Karaoke {
                level: 0.9,
                mono_level: 0.8,
                filter_band: 210.0,
                filter_width: 90.0,
            }),
            timescale: Some(Timescale {
                speed: 1.1,
                pitch: 1.2,
                rate: 1.3,
            }),
            tremolo: Some(Tremolo {
                frequency: 3.0,
                depth: 0.4,
            }),
            vibrato: Some(Vibrato {
                frequency: 4.0,
                depth: 0.3,
            }),
            distortion: Some(Distortion {
                sin_offset: 0.1,
                sin_scale: 1.1,
                cos_offset: 0.2,
                cos_scale: 1.2,
                tan_offset: 0.3,
                tan_scale: 1.3,
                offset: 0.4,
                scale: 1.4,
            }),
            rotation: Some(Rotation { rotation_hz: 0.6 }),
            channel_mix: Some(ChannelMix {
                left_to_left: 0.7,
                left_to_right: 0.3,
                right_to_left: 0.2,
                right_to_right: 0.8,
            }),
            low_pass: Some(LowPass { smoothing: 15.0 }),
            plugin_filters: Map::new(),
        };

        let engine = filters.to_engine();
        assert_eq!(engine.volume, Some(0.5));
        let bands = engine.equalizer.expect("equalizer");
        assert_eq!((bands[0].band, bands[0].gain), (3, 0.25));
        let karaoke = engine.karaoke.expect("karaoke");
        assert_eq!(
            (
                karaoke.level,
                karaoke.mono_level,
                karaoke.filter_band,
                karaoke.filter_width
            ),
            (0.9, 0.8, 210.0, 90.0)
        );
        let timescale = engine.timescale.expect("timescale");
        assert_eq!(
            (timescale.speed, timescale.pitch, timescale.rate),
            (1.1, 1.2, 1.3)
        );
        let tremolo = engine.tremolo.expect("tremolo");
        assert_eq!((tremolo.frequency, tremolo.depth), (3.0, 0.4));
        let vibrato = engine.vibrato.expect("vibrato");
        assert_eq!((vibrato.frequency, vibrato.depth), (4.0, 0.3));
        let d = engine.distortion.expect("distortion");
        assert_eq!(
            (d.sin_offset, d.sin_scale, d.cos_offset, d.cos_scale),
            (0.1, 1.1, 0.2, 1.2)
        );
        assert_eq!(
            (d.tan_offset, d.tan_scale, d.offset, d.scale),
            (0.3, 1.3, 0.4, 1.4)
        );
        assert_eq!(engine.rotation.expect("rotation").rotation_hz, 0.6);
        let mix = engine.channel_mix.expect("channelMix");
        assert_eq!(
            (
                mix.left_to_left,
                mix.left_to_right,
                mix.right_to_left,
                mix.right_to_right
            ),
            (0.7, 0.3, 0.2, 0.8)
        );
        assert_eq!(engine.low_pass.expect("lowPass").smoothing, 15.0);
    }
}
