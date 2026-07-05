//! Layered precedence merge for device profiles (ADR-0007).

/// The five precedence layers a device profile can come from, ordered from
/// weakest to strongest per ADR-0007:
///
/// ```text
/// built-in < organization < user < vehicle < session
/// ```
///
/// When two layers both configure the same axis or button `source_index`,
/// the entry from the higher-precedence (later-listed) layer wins in full —
/// merge happens per `source_index`, not per-field within an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProfileLayer {
    /// Ships with this crate's registry.
    BuiltIn,
    /// Organization-wide overrides.
    Organization,
    /// Per-user overrides.
    User,
    /// Vehicle-specific overrides.
    Vehicle,
    /// Current-session-only overrides, highest precedence.
    Session,
}

/// A device profile paired with the precedence layer it was loaded from.
#[derive(Debug, Clone, PartialEq)]
pub struct LayeredProfile<T> {
    /// The precedence layer this profile occupies.
    pub layer: ProfileLayer,
    /// The profile content itself.
    pub profile: T,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::ProfileLayer;

    #[test]
    fn layers_order_from_built_in_to_session() {
        assert!(ProfileLayer::BuiltIn < ProfileLayer::Organization);
        assert!(ProfileLayer::Organization < ProfileLayer::User);
        assert!(ProfileLayer::User < ProfileLayer::Vehicle);
        assert!(ProfileLayer::Vehicle < ProfileLayer::Session);
    }
}
