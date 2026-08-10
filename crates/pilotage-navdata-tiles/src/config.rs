//! Tile build configuration.

use crate::NavdataTileError;

/// Default highest zoom in a Navdata baseline bundle.
pub const DEFAULT_MAX_ZOOM: u8 = 8;

/// Display scale for each baseline layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavdataTileConfig {
    /// First zoom that contains published airspace bounds.
    pub airspace_min_zoom: u8,
    /// First zoom that contains airways.
    pub airway_min_zoom: u8,
    /// First zoom that contains aerodromes.
    pub aerodrome_min_zoom: u8,
    /// First zoom that contains navaids.
    pub navaid_min_zoom: u8,
    /// First zoom that contains fixes.
    pub fix_min_zoom: u8,
    /// Highest zoom produced for all layers.
    pub max_zoom: u8,
}

impl Default for NavdataTileConfig {
    fn default() -> Self {
        Self {
            airspace_min_zoom: 0,
            airway_min_zoom: 2,
            aerodrome_min_zoom: 3,
            navaid_min_zoom: 5,
            fix_min_zoom: 6,
            max_zoom: DEFAULT_MAX_ZOOM,
        }
    }
}

impl NavdataTileConfig {
    pub(crate) fn validate(self) -> Result<(), NavdataTileError> {
        if self.max_zoom > 14 {
            return Err(NavdataTileError::InvalidConfig {
                reason: format!("max_zoom {} is greater than 14", self.max_zoom),
            });
        }
        for (layer, minimum) in self.minimums() {
            if minimum > self.max_zoom {
                return Err(NavdataTileError::InvalidConfig {
                    reason: format!(
                        "{layer} minimum zoom {minimum} is greater than max_zoom {}",
                        self.max_zoom
                    ),
                });
            }
        }
        Ok(())
    }

    pub(crate) const fn minimums(self) -> [(&'static str, u8); 5] {
        [
            ("airspace", self.airspace_min_zoom),
            ("airway", self.airway_min_zoom),
            ("aerodrome", self.aerodrome_min_zoom),
            ("navaid", self.navaid_min_zoom),
            ("fix", self.fix_min_zoom),
        ]
    }
}
