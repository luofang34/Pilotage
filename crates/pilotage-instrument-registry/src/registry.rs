//! Registry composition and its init-time validation.

use pilotage_instrument_scene::LAYER_COUNT;

use crate::descriptor::PanelDescriptor;

/// A validated panel composition. Construction is the gate: a shell
/// that composes nonsense fails at init, not at draw time.
#[derive(Debug, Clone, Copy)]
pub struct Registry {
    panels: &'static [PanelDescriptor],
}

/// Why a composition was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// A shell with no panels has nothing to display.
    #[error("a registry must contain at least one panel")]
    Empty,
    /// A panel id violates the lowercase/digits/dashes charset.
    #[error("panel {index} has a malformed id")]
    BadId {
        /// Position in the composed slice.
        index: usize,
    },
    /// Two panels share an id.
    #[error("panel {index} repeats an earlier panel's id")]
    DuplicateId {
        /// Position of the second occurrence.
        index: usize,
    },
    /// An empty title cannot label health or layout surfaces.
    #[error("panel {index} has an empty title")]
    EmptyTitle {
        /// Position in the composed slice.
        index: usize,
    },
    /// A panel that requires no layers would pass every completeness
    /// check vacuously.
    #[error("panel {index} declares no required layers")]
    NoRequiredLayers {
        /// Position in the composed slice.
        index: usize,
    },
    /// Required-layer bits beyond the defined scene layers.
    #[error("panel {index} requires undefined layer bits {bits:#04x}")]
    UndefinedLayerBits {
        /// Position in the composed slice.
        index: usize,
        /// The offending mask.
        bits: u8,
    },
    /// A non-finite or non-positive design frame.
    #[error("panel {index} has a degenerate design frame")]
    BadDesignFrame {
        /// Position in the composed slice.
        index: usize,
    },
    /// Schema keys must be strictly ascending (unique by construction).
    #[error("panel {index} schema key {key} repeats or descends")]
    SchemaKeysNotAscending {
        /// Position in the composed slice.
        index: usize,
        /// The out-of-order key.
        key: u16,
    },
    /// A group region for a group the panel does not consume.
    #[error("panel {index} declares a region for group {group} it does not require")]
    RegionGroupNotRequired {
        /// Position in the composed slice.
        index: usize,
        /// The wire tag of the unrequired group.
        group: u8,
    },
    /// A group region outside the design frame (or degenerate).
    #[error("panel {index} declares a region for group {group} outside its design frame")]
    RegionOutsideFrame {
        /// Position in the composed slice.
        index: usize,
        /// The wire tag of the group.
        group: u8,
    },
    /// Two extreme states of one panel share an id.
    #[error("panel {index} repeats the extreme-state id at position {position}")]
    DuplicateExtremeId {
        /// Position in the composed slice.
        index: usize,
        /// Position of the second occurrence within the panel.
        position: usize,
    },
    /// An extreme-state id violates the lowercase/digits/dashes charset.
    #[error("panel {index} extreme state {position} has a malformed id")]
    BadExtremeId {
        /// Position in the composed slice.
        index: usize,
        /// Position of the offending extreme state within the panel.
        position: usize,
    },
}

/// Bits a required-layer mask may set: one per defined scene layer.
/// The u16 intermediate keeps the shift well-defined right up to the
/// mask's own u8 capacity; growing past eight layers must widen the
/// descriptor mask deliberately, not overflow silently.
const DEFINED_LAYER_BITS: u8 = {
    assert!(LAYER_COUNT <= 8, "layer mask is a u8");
    ((1u16 << LAYER_COUNT) - 1) as u8
};

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Registry {
    /// Validates and composes `panels`.
    pub fn new(panels: &'static [PanelDescriptor]) -> Result<Registry, RegistryError> {
        if panels.is_empty() {
            return Err(RegistryError::Empty);
        }
        for (index, panel) in panels.iter().enumerate() {
            validate_panel(index, panel)?;
            if panels[..index].iter().any(|earlier| earlier.id == panel.id) {
                return Err(RegistryError::DuplicateId { index });
            }
        }
        Ok(Registry { panels })
    }

    /// The composed descriptors, in shell order.
    pub const fn panels(&self) -> &'static [PanelDescriptor] {
        self.panels
    }

    /// The descriptor with `id`, if composed.
    pub fn by_id(&self, id: &str) -> Option<&'static PanelDescriptor> {
        self.panels.iter().find(|panel| panel.id == id)
    }
}

fn validate_panel(index: usize, panel: &PanelDescriptor) -> Result<(), RegistryError> {
    if !id_ok(panel.id) {
        return Err(RegistryError::BadId { index });
    }
    if panel.title.is_empty() {
        return Err(RegistryError::EmptyTitle { index });
    }
    if panel.required_layers == 0 {
        return Err(RegistryError::NoRequiredLayers { index });
    }
    if panel.required_layers & !DEFINED_LAYER_BITS != 0 {
        return Err(RegistryError::UndefinedLayerBits {
            index,
            bits: panel.required_layers,
        });
    }
    let frame = panel.design_frame;
    if !(frame.width.is_finite()
        && frame.height.is_finite()
        && frame.width > 0.0
        && frame.height > 0.0)
    {
        return Err(RegistryError::BadDesignFrame { index });
    }
    let mut previous: Option<u16> = None;
    for key in panel.config_schema {
        if previous.is_some_and(|previous| key.0 <= previous) {
            return Err(RegistryError::SchemaKeysNotAscending { index, key: key.0 });
        }
        previous = Some(key.0);
    }
    for (group, region) in panel.group_regions {
        if !panel.required_groups.contains(*group) {
            return Err(RegistryError::RegionGroupNotRequired {
                index,
                group: *group as u8,
            });
        }
        let inside = region.x >= 0.0
            && region.y >= 0.0
            && region.width > 0.0
            && region.height > 0.0
            && region.x + region.width <= frame.width
            && region.y + region.height <= frame.height;
        if !inside {
            return Err(RegistryError::RegionOutsideFrame {
                index,
                group: *group as u8,
            });
        }
    }
    for (position, extreme) in panel.extreme_states.iter().enumerate() {
        if !id_ok(extreme.id) {
            return Err(RegistryError::BadExtremeId { index, position });
        }
        if panel.extreme_states[..position]
            .iter()
            .any(|earlier| earlier.id == extreme.id)
        {
            return Err(RegistryError::DuplicateExtremeId { index, position });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
