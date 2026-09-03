use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub version: Version,
    /// Unix millisecond timestamp of when this build was produced.
    pub build_time: i64,
    pub git: Git,
    /// The runtime version, keyed `jvm` for wire compatibility.
    pub jvm: String,
    /// The audio engine version, keyed `lavaplayer` for wire compatibility.
    pub lavaplayer: String,
    pub source_managers: Vec<String>,
    pub filters: Vec<String>,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub semver: String,
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Git {
    pub branch: String,
    pub commit: String,
    /// Unix millisecond timestamp of the commit.
    pub commit_time: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plugin {
    pub name: String,
    pub version: String,
}
