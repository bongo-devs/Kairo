//! The three-state wrapper for fields that may be left out of a payload.
//!
//! A field of type `Omissible<T>` separates three wire states:
//!
//! - absent from the JSON: [`Omissible::Omitted`], meaning leave unchanged
//! - present as `null`: `Omissible::Present(None)` when `T = Option<U>`, meaning clear
//! - present with a value: `Omissible::Present(value)`, meaning set
//!
//! Serde drives the absent versus present split, so every field of this type needs
//! `#[serde(default)]`: a missing key falls back to [`Default`], a present one runs the
//! [`Deserialize`] impl below, which always yields `Present`.
//!
//! Deserialize only, since the types holding one are inbound payloads and there is nothing to
//! leave out on the way back.

use serde::{Deserialize, Deserializer};

/// A value that may be omitted from a JSON payload entirely.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Omissible<T> {
    /// The field was present in the payload.
    Present(T),
    /// The field was absent from the payload.
    #[default]
    Omitted,
}

impl<T> Omissible<T> {
    pub fn is_present(&self) -> bool {
        matches!(self, Omissible::Present(_))
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Omissible<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Serde only calls this when the key is present, so any value here, an explicit `null`
        // included, is `Present`.
        T::deserialize(deserializer).map(Omissible::Present)
    }
}
