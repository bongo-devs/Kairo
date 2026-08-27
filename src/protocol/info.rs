//! The node `info` response.

use serde::Serialize;

/// Server information.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    /// The server version.
    pub version: Version,
    /// Unix millisecond timestamp of when this build was produced.
    pub build_time: i64,
    /// Git build information.
    pub git: Git,
    /// The runtime version, keyed `jvm` for wire compatibility.
    pub jvm: String,
    /// The audio engine version, keyed `lavaplayer` for wire compatibility.
    pub lavaplayer: String,
    /// Enabled source manager names.
    pub source_managers: Vec<String>,
    /// Enabled filter names.
    pub filters: Vec<String>,
    /// Installed plugins.
    pub plugins: Vec<Plugin>,
}

/// A semantic version.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// The full semver string.
    pub semver: String,
    /// Major version.
    pub major: i32,
    /// Minor version.
    pub minor: i32,
    /// Patch version.
    pub patch: i32,
    /// The pre-release identifier, if any.
    pub pre_release: Option<String>,
}

impl Version {
    /// Parse a `major.minor.patch[-prerelease]` string, taking anything non-numeric as zero.
    pub fn from_semver(semver: &str) -> Self {
        let (core, pre_release) = match semver.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_string())),
            None => (semver, None),
        };
        let mut parts = core.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Self {
            semver: semver.to_string(),
            major,
            minor,
            patch,
            pre_release,
        }
    }
}

/// Git build information.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Git {
    /// The git branch.
    pub branch: String,
    /// The git commit hash.
    pub commit: String,
    /// Unix millisecond timestamp of the commit.
    pub commit_time: i64,
}

/// An installed plugin.
#[derive(Debug, Clone, Serialize)]
pub struct Plugin {
    /// The plugin name.
    pub name: String,
    /// The plugin version.
    pub version: String,
}
