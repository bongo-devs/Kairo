//! `crossfade.*`, the track transition defaults.
//!
//! These are node-wide, and a client can override them per player through the `crossfade` field of
//! a player update. With both crossfade and gapless off, a track simply ends with reason `finished`
//! and the client starts the next one.

use serde::Deserialize;

use player::{CrossfadeCurve as EngineCrossfadeCurve, CrossfadeOptions};

/// The `crossfade` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CrossfadeConfig {
    /// Whether to fade the outgoing and incoming tracks over each other on a transition.
    pub enable: bool,
    /// Overlap length in milliseconds, used when `enable` is set.
    pub duration_ms: u64,
    /// Overlap length in milliseconds for an explicit manual skip.
    pub manual_duration_ms: u64,
    /// Fade easing.
    pub curve: CrossfadeCurve,
    /// Whether to fall back to a zero-gap handoff when a fade is off or cannot apply, as with a
    /// track that is too short, not seekable or of unknown length. On by default.
    pub gapless: bool,
}

impl Default for CrossfadeConfig {
    fn default() -> Self {
        Self {
            enable: false,
            duration_ms: 6000,
            manual_duration_ms: 3500,
            curve: CrossfadeCurve::SCurve,
            gapless: true,
        }
    }
}

impl CrossfadeConfig {
    /// The transition options in effect, or `None` when neither a fade nor a gapless handoff is
    /// wanted. A fade uses `duration_ms` and `curve`, while a gapless handoff is the same
    /// transition with a zero overlap.
    pub fn to_engine(&self) -> Option<CrossfadeOptions> {
        if self.enable {
            if self.duration_ms == 0 && !self.gapless {
                return None;
            }
            Some(CrossfadeOptions {
                duration_ms: self.duration_ms,
                manual_duration_ms: self.manual_duration_ms,
                curve: self.curve.to_engine(),
                gapless: self.gapless,
            })
        } else if self.gapless {
            Some(CrossfadeOptions {
                duration_ms: 0,
                manual_duration_ms: self.manual_duration_ms,
                curve: self.curve.to_engine(),
                gapless: true,
            })
        } else {
            None
        }
    }
}

/// Fade easing as written in YAML.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CrossfadeCurve {
    /// Straight ramp.
    Linear,
    /// Fast start, slow end, `x²`.
    Exp,
    /// Slow start, fast end, `√x`.
    Log,
    /// Raised-cosine S-curve, the default.
    #[default]
    SCurve,
}

impl CrossfadeCurve {
    fn to_engine(self) -> EngineCrossfadeCurve {
        match self {
            CrossfadeCurve::Linear => EngineCrossfadeCurve::Linear,
            CrossfadeCurve::Exp => EngineCrossfadeCurve::Exponential,
            CrossfadeCurve::Log => EngineCrossfadeCurve::Logarithmic,
            CrossfadeCurve::SCurve => EngineCrossfadeCurve::SCurve,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_config_block() {
        let yaml = r#"
enable: true
durationMs: 4000
manualDurationMs: 3500
curve: linear
gapless: true
"#;
        let cfg: CrossfadeConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.enable);
        assert_eq!(cfg.duration_ms, 4000);
        assert_eq!(cfg.manual_duration_ms, 3500);
        let engine = cfg.to_engine().expect("an enabled fade has options");
        assert_eq!(engine.duration_ms, 4000);
        assert_eq!(engine.curve, EngineCrossfadeCurve::Linear);
        assert!(!engine.is_gapless());
    }

    #[test]
    fn defaults_are_gapless_not_crossfade() {
        let cfg: CrossfadeConfig = serde_yaml::from_str("{}").unwrap();
        assert!(!cfg.enable);
        assert!(cfg.gapless);
        assert_eq!(cfg.duration_ms, 6000);
        assert_eq!(cfg.manual_duration_ms, 3500);
        // No fade but gapless on still transitions, with a zero overlap.
        let engine = cfg.to_engine().expect("gapless alone has options");
        assert!(engine.is_gapless());
    }

    #[test]
    fn all_off_yields_no_transition() {
        let cfg: CrossfadeConfig = serde_yaml::from_str("enable: false\ngapless: false\n").unwrap();
        assert!(cfg.to_engine().is_none());
    }

    #[test]
    fn each_curve_name_parses() {
        for (name, expected) in [
            ("linear", EngineCrossfadeCurve::Linear),
            ("exp", EngineCrossfadeCurve::Exponential),
            ("log", EngineCrossfadeCurve::Logarithmic),
            ("sCurve", EngineCrossfadeCurve::SCurve),
        ] {
            let yaml = format!("enable: true\ncurve: {name}\n");
            let cfg: CrossfadeConfig = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(cfg.to_engine().unwrap().curve, expected);
        }
    }
}
