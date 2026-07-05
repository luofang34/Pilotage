//! Merges device profiles across precedence layers into one effective
//! profile (ADR-0007).

use std::collections::BTreeMap;

use crate::profile::{AxisConfig, ButtonConfig, DeviceProfile};

use super::layer::{LayeredProfile, ProfileLayer};

/// Merges profiles from multiple layers into one effective profile.
///
/// Layers are applied in ascending [`ProfileLayer`] order regardless of the
/// order they appear in `layers`: for a given `source_index`, the entry from
/// the highest-precedence layer that configures it replaces entries from
/// every lower layer entirely (whole-entry replacement, not per-field
/// merging). `schema_version`, `device`, and `description` are taken from
/// the highest-precedence layer present; `revision` is also taken from the
/// highest-precedence layer, since a merged profile is itself one
/// evaluable revision.
///
/// Returns `None` if `layers` is empty.
#[must_use]
pub fn merge_layers(mut layers: Vec<LayeredProfile<DeviceProfile>>) -> Option<DeviceProfile> {
    layers.sort_by_key(|entry| entry.layer);

    let mut axes: BTreeMap<usize, AxisConfig> = BTreeMap::new();
    let mut buttons: BTreeMap<u8, ButtonConfig> = BTreeMap::new();
    let mut base: Option<DeviceProfile> = None;

    for entry in layers {
        for axis in entry.profile.axes.iter().cloned() {
            axes.insert(axis.source_index, axis);
        }
        for button in entry.profile.buttons.iter().cloned() {
            buttons.insert(button.source_index, button);
        }
        base = Some(entry.profile);
    }

    let base = base?;
    Some(DeviceProfile {
        schema_version: base.schema_version,
        revision: base.revision,
        device: base.device,
        description: base.description,
        axes: axes.into_values().collect(),
        buttons: buttons.into_values().collect(),
    })
}

/// Convenience constructor for a [`LayeredProfile`] entry.
#[must_use]
pub const fn layered(layer: ProfileLayer, profile: DeviceProfile) -> LayeredProfile<DeviceProfile> {
    LayeredProfile { layer, profile }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{layered, merge_layers};
    use crate::profile::{AxisCalibration, AxisConfig, ButtonConfig, DeviceInfo, DeviceProfile};
    use crate::registry::layer::ProfileLayer;

    fn base_profile(
        revision: u32,
        axes: Vec<AxisConfig>,
        buttons: Vec<ButtonConfig>,
    ) -> DeviceProfile {
        DeviceProfile {
            schema_version: 1,
            revision,
            device: DeviceInfo {
                vendor_id: 1,
                product_id: 2,
                product: None,
            },
            description: None,
            axes,
            buttons,
        }
    }

    fn axis(source_index: usize, logical: &str) -> AxisConfig {
        AxisConfig {
            source_index,
            logical: logical.to_string(),
            invert: false,
            deadzone: 0.0,
            expo: 0.0,
            calibration: AxisCalibration {
                min: -1.0,
                center: 0.0,
                max: 1.0,
            },
        }
    }

    #[test]
    fn empty_layers_produce_no_profile() {
        assert!(merge_layers(vec![]).is_none());
    }

    #[test]
    fn single_layer_passes_through_unchanged() {
        let profile = base_profile(1, vec![axis(0, "roll")], vec![]);
        let merged = merge_layers(vec![layered(ProfileLayer::BuiltIn, profile.clone())])
            .expect("merge produces a profile");
        assert_eq!(merged, profile);
    }

    #[test]
    fn later_layer_replaces_same_source_index() {
        let built_in = base_profile(1, vec![axis(0, "roll")], vec![]);
        let mut user_axis = axis(0, "roll");
        user_axis.invert = true;
        let user = base_profile(2, vec![user_axis.clone()], vec![]);
        let merged = merge_layers(vec![
            layered(ProfileLayer::BuiltIn, built_in),
            layered(ProfileLayer::User, user),
        ])
        .expect("merge produces a profile");
        assert_eq!(merged.axes, vec![user_axis]);
    }

    #[test]
    fn merge_is_order_independent_input_order() {
        let built_in = base_profile(1, vec![axis(0, "roll")], vec![]);
        let mut session_axis = axis(0, "roll");
        session_axis.deadzone = 0.2;
        let session = base_profile(3, vec![session_axis.clone()], vec![]);

        // Pass session first, built-in second: result must be identical.
        let merged = merge_layers(vec![
            layered(ProfileLayer::Session, session),
            layered(ProfileLayer::BuiltIn, built_in),
        ])
        .expect("merge produces a profile");
        assert_eq!(merged.axes, vec![session_axis]);
    }

    #[test]
    fn distinct_source_indexes_are_all_retained() {
        let built_in = base_profile(1, vec![axis(0, "roll"), axis(1, "pitch")], vec![]);
        let user = base_profile(2, vec![axis(2, "throttle")], vec![]);
        let merged = merge_layers(vec![
            layered(ProfileLayer::BuiltIn, built_in),
            layered(ProfileLayer::User, user),
        ])
        .expect("merge produces a profile");
        assert_eq!(merged.axes.len(), 3);
    }

    #[test]
    fn highest_layer_wins_device_metadata() {
        let built_in = base_profile(1, vec![], vec![]);
        let vehicle = base_profile(9, vec![], vec![]);
        let merged = merge_layers(vec![
            layered(ProfileLayer::BuiltIn, built_in),
            layered(ProfileLayer::Vehicle, vehicle),
        ])
        .expect("merge produces a profile");
        assert_eq!(merged.revision, 9);
    }
}
