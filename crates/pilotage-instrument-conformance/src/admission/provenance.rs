//! The honest-status family, provenance form: a text run carries a
//! claim about the state group its value derives from, and the
//! withholding matrix tests the claim.
//!
//! The rule is positional nowhere: an honest scale label passes through
//! any screen region freely, and a fabricated numeral is refused
//! wherever it is drawn. Soundness needs totality — a numeric run
//! without a claim is refused outright, so omitting the tag is not an
//! escape — and claims are bounded to the panel's own required groups,
//! so a fabricator cannot park its number on a group the matrix never
//! withholds. The one residual escape is a fabricator whose fake value
//! is gated on the group it falsely claims — at which point it is,
//! behaviourally, deriving from that group.

use pilotage_instrument_registry::PanelDescriptor;
use pilotage_instrument_scene::ATTR_CONFIG;
use pilotage_instrument_state::GroupId;

use super::{AdmissionError, TextRun};

/// Checks every run of one drawn case: claims are total over numerals,
/// bounded to the panel's required groups, and a run claiming the
/// withheld group may not be visible. The test is against withholding,
/// not against the resolved group status — a per-field signal may
/// legitimately outlive its group's summary status (the baro setting
/// stays shown when absent trust fails the Air group, because a
/// setting is not an estimate), and the resolve layer's per-field
/// gates remain the authority on what may be shown.
pub(super) fn check_provenance(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    withheld: Option<GroupId>,
    runs: &[TextRun],
) -> Result<(), AdmissionError> {
    for run in runs {
        let Some(tag) = run.attribution else {
            // Totality: unclaimed numerals are refused even when
            // scrolled outside a clip — an unclaimed numeral is a
            // hole, not merely invisible ink.
            if run.numeric() {
                return Err(AdmissionError::UntaggedNumeral {
                    panel: panel.id,
                    state: state_id,
                    text: run.text.clone(),
                });
            }
            continue;
        };
        // A run scrolled fully outside its clip paints nothing, and
        // the claim rule is about what is shown.
        if !run.visible {
            continue;
        }
        if tag == ATTR_CONFIG {
            // The harness draws the fixed empty configuration, so a
            // visible run claiming configuration provenance derives
            // from nothing. Consequence, stated plainly: a panel whose
            // numeral legitimately derives from configuration cannot
            // pass admission as drawn today.
            return Err(AdmissionError::ConfigClaim {
                panel: panel.id,
                state: state_id,
                text: run.text.clone(),
            });
        }
        let group = GroupId::from_u8(tag).filter(|g| panel.required_groups.contains(*g));
        let Some(group) = group else {
            // A claim outside the panel's required groups would never
            // be withheld, so the matrix could never test it.
            return Err(AdmissionError::ForeignClaim {
                panel: panel.id,
                state: state_id,
                tag,
                text: run.text.clone(),
            });
        };
        if withheld == Some(group) {
            return Err(AdmissionError::FabricatedNumeral {
                panel: panel.id,
                state: state_id,
                group,
                text: run.text.clone(),
            });
        }
    }
    Ok(())
}
