//! Session resuming configuration.

use serde::{Deserialize, Serialize};

use crate::protocol::omissible::Omissible;

/// A session's resuming configuration, the response of `PATCH /v4/sessions/{sessionId}`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Session {
    /// Whether resuming is enabled.
    pub resuming: bool,
    /// Seconds the session may be resumed for after the socket drops.
    pub timeout: i64,
}

/// The body of `PATCH /v4/sessions/{sessionId}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SessionUpdate {
    /// Whether to enable resuming.
    pub resuming: Omissible<bool>,
    /// Seconds the session may be resumed for.
    pub timeout: Omissible<i64>,
}
