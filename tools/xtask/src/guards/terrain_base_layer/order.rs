//! The order of the style's sources as the file writes them.
//!
//! `serde_json::Value` keeps object members sorted by key, and jq's
//! `to_entries` keeps them in file order. A check that takes the first
//! source that qualifies reads the file order from here.

use std::fmt;

use serde::Deserialize;
use serde::de::{IgnoredAny, MapAccess, Visitor};

/// The member names of an object in file order.
struct Keys(Vec<String>);

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(KeysVisitor)
    }
}

struct KeysVisitor;

impl<'de> Visitor<'de> for KeysVisitor {
    type Value = Keys;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
        let mut keys = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            map.next_value::<IgnoredAny>()?;
            keys.push(key);
        }
        Ok(Keys(keys))
    }
}

#[derive(Deserialize)]
struct Style {
    #[serde(default)]
    sources: Option<Keys>,
}

/// The source names of a style file in file order; empty when the file
/// does not parse or has no sources object.
pub(super) fn source_order(bytes: &[u8]) -> Vec<String> {
    serde_json::from_slice::<Style>(bytes)
        .ok()
        .and_then(|style| style.sources)
        .map_or_else(Vec::new, |keys| keys.0)
}
