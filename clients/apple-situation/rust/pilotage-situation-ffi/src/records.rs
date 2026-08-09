//! Flat records that cross the Apple FFI boundary.

/// Schema versions of the linked domain producers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ProducerSchemaVersions {
    /// Surveillance track schema version.
    pub surveillance: u16,
    /// Airmass weather snapshot schema version.
    pub airmass: u16,
}

#[cfg(test)]
mod tests {
    use super::ProducerSchemaVersions;

    #[test]
    fn linked_versions_are_nonzero() {
        let versions = crate::producer_schema_versions();
        assert_ne!(
            versions,
            ProducerSchemaVersions {
                surveillance: 0,
                airmass: 0,
            }
        );
        assert_ne!(versions.surveillance, 0);
        assert_ne!(versions.airmass, 0);
    }
}
