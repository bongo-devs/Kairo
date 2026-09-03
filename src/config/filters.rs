//! `server.filters.*`, the per-filter enable toggles.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FiltersToggleConfig {
    pub volume: bool,
    pub equalizer: bool,
    pub karaoke: bool,
    pub timescale: bool,
    pub tremolo: bool,
    pub vibrato: bool,
    pub distortion: bool,
    pub rotation: bool,
    pub channel_mix: bool,
    pub low_pass: bool,
}

impl Default for FiltersToggleConfig {
    fn default() -> Self {
        Self {
            volume: true,
            equalizer: true,
            karaoke: true,
            timescale: true,
            tremolo: true,
            vibrato: true,
            distortion: true,
            rotation: true,
            channel_mix: true,
            low_pass: true,
        }
    }
}

impl FiltersToggleConfig {
    /// The wire names of the disabled filters.
    pub fn disabled(&self) -> Vec<String> {
        self.named()
            .filter(|(on, _)| !on)
            .map(|(_, n)| n.to_string())
            .collect()
    }

    /// The wire names of the enabled filters, as reported by `GET /v4/info`.
    pub fn enabled(&self) -> Vec<String> {
        self.named()
            .filter(|(on, _)| *on)
            .map(|(_, n)| n.to_string())
            .collect()
    }

    // `(enabled, wire name)` for every filter, in a stable order.
    fn named(&self) -> impl Iterator<Item = (bool, &'static str)> {
        [
            (self.volume, "volume"),
            (self.equalizer, "equalizer"),
            (self.karaoke, "karaoke"),
            (self.timescale, "timescale"),
            (self.tremolo, "tremolo"),
            (self.vibrato, "vibrato"),
            (self.distortion, "distortion"),
            (self.rotation, "rotation"),
            (self.channel_mix, "channelMix"),
            (self.low_pass, "lowPass"),
        ]
        .into_iter()
    }
}
