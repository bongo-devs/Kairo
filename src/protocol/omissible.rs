//! `Omissible<T>` tells absent (`Omitted`, leave unchanged) from `null` (`Present(None)`, clear)
//! from a value (`Present`, set). Every field needs `#[serde(default)]`.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Omissible<T> {
    Present(T),
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
