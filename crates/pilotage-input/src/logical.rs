//! Well-known logical axis and button names for device profiles (ADR-0007).
//!
//! Device profiles reference logical inputs by name rather than raw
//! `LogicalAxisId`/`LogicalButtonId` values, so profiles stay readable and
//! portable across the crate's evolution. This module is the single
//! authority translating a name to its numeric identifier; unknown names are
//! a typed load-time error, never a silent default.

use pilotage_protocol::{LogicalAxisId, LogicalButtonId};

use crate::profile::ProfileError;

/// First identifier of the positional `slotN` band (see [`axis_id_for_name`]).
pub const SLOT_AXIS_BASE: u16 = 64;
/// Number of positional `slotN` identifiers in the band.
pub const SLOT_AXIS_COUNT: u16 = 8;

/// Resolves a well-known logical axis name to its `LogicalAxisId`.
///
/// The table is: `roll` = 0, `pitch` = 1, `throttle` = 2, `yaw` = 3, then
/// `aux1`..`aux60` = 4..=63, then the positional band `slot0`..`slot7` =
/// 64..=71. Names outside this table are a typed error so profile authors
/// get a load-time failure instead of a silently-dropped axis.
///
/// `slotN` names identify PHYSICAL POSITIONS of the canonical browser pad
/// layout (0 = left-stick X, 1 = left-stick Y, 2 = right-stick X,
/// 3 = right-stick Y, 4..7 spare): a device profile targeting slots defers
/// semantic meaning to the consuming control scheme (which may permute
/// positions per operator mode), whereas `roll`/`pitch`/`throttle`/`yaw`
/// assert the device itself fixes the meaning (an RC transmitter's gimbals).
/// Slot identifiers never appear on the wire as semantic axes.
///
/// # Errors
///
/// Returns [`ProfileError::UnknownAxisName`] if `name` is not in the table.
pub fn axis_id_for_name(name: &str) -> Result<LogicalAxisId, ProfileError> {
    let raw = match name {
        "roll" => 0,
        "pitch" => 1,
        "throttle" => 2,
        "yaw" => 3,
        other => {
            if let Some(slot) = other.strip_prefix("slot") {
                let index: u16 = slot.parse().map_err(|_| ProfileError::UnknownAxisName {
                    name: name.to_string(),
                })?;
                if index >= SLOT_AXIS_COUNT {
                    return Err(ProfileError::UnknownAxisName {
                        name: name.to_string(),
                    });
                }
                SLOT_AXIS_BASE + index
            } else {
                aux_index(other, "aux")?
            }
        }
    };
    Ok(LogicalAxisId::new(raw))
}

/// Resolves a well-known logical button name to its `LogicalButtonId`.
///
/// Buttons are named `button0`, `button1`, ... mapping directly to
/// `LogicalButtonId::new(n)`. Names outside this pattern are a typed error.
///
/// # Errors
///
/// Returns [`ProfileError::UnknownButtonName`] if `name` is not in the
/// table.
pub fn button_id_for_name(name: &str) -> Result<LogicalButtonId, ProfileError> {
    let raw = aux_index(name, "button").map_err(|_| ProfileError::UnknownButtonName {
        name: name.to_string(),
    })?;
    Ok(LogicalButtonId::new(raw))
}

/// Parses a `{prefix}{n}` name into `n`, returning `UnknownAxisName` on any
/// mismatch. Shared by axis `auxN` and button `buttonN` parsing since both
/// follow the same prefix-plus-index shape.
fn aux_index(name: &str, prefix: &str) -> Result<u16, ProfileError> {
    let suffix = name
        .strip_prefix(prefix)
        .ok_or_else(|| ProfileError::UnknownAxisName {
            name: name.to_string(),
        })?;
    let index: u16 = suffix.parse().map_err(|_| ProfileError::UnknownAxisName {
        name: name.to_string(),
    })?;
    let raw = if prefix == "aux" {
        4u16.checked_add(
            index
                .checked_sub(1)
                .ok_or_else(|| ProfileError::UnknownAxisName {
                    name: name.to_string(),
                })?,
        )
    } else {
        Some(index)
    };
    raw.ok_or_else(|| ProfileError::UnknownAxisName {
        name: name.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{axis_id_for_name, button_id_for_name};

    #[test]
    fn resolves_named_axes() {
        assert_eq!(axis_id_for_name("roll").expect("roll").as_u16(), 0);
        assert_eq!(axis_id_for_name("pitch").expect("pitch").as_u16(), 1);
        assert_eq!(axis_id_for_name("throttle").expect("throttle").as_u16(), 2);
        assert_eq!(axis_id_for_name("yaw").expect("yaw").as_u16(), 3);
    }

    #[test]
    fn resolves_aux_axes_starting_at_four() {
        assert_eq!(axis_id_for_name("aux1").expect("aux1").as_u16(), 4);
        assert_eq!(axis_id_for_name("aux2").expect("aux2").as_u16(), 5);
    }

    #[test]
    fn resolves_positional_slots_in_the_reserved_band() {
        assert_eq!(axis_id_for_name("slot0").expect("slot0").as_u16(), 64);
        assert_eq!(axis_id_for_name("slot7").expect("slot7").as_u16(), 71);
    }

    #[test]
    fn rejects_slots_outside_the_band() {
        assert!(axis_id_for_name("slot8").is_err());
        assert!(axis_id_for_name("slot-1").is_err());
        assert!(axis_id_for_name("slotx").is_err());
    }

    #[test]
    fn rejects_unknown_axis_name() {
        let err = axis_id_for_name("bogus").expect_err("should fail");
        assert!(matches!(err, super::ProfileError::UnknownAxisName { .. }));
    }

    #[test]
    fn resolves_button_names_from_zero() {
        assert_eq!(button_id_for_name("button0").expect("button0").as_u16(), 0);
        assert_eq!(button_id_for_name("button5").expect("button5").as_u16(), 5);
    }

    #[test]
    fn rejects_unknown_button_name() {
        let err = button_id_for_name("nope").expect_err("should fail");
        assert!(matches!(err, super::ProfileError::UnknownButtonName { .. }));
    }
}
