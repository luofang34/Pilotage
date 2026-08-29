use pilotage_mission_core::{MissionAction, MissionDocument, TrialAction};
use pilotage_trial::Digest;

use crate::{ControlChannel, ControlFamily, TuneError};

use super::{ResponseTargetTable, invalid_table};

/// The stimulus a scoped scenario states it commands.
struct DocumentScope {
    family: ControlFamily,
    channel: ControlChannel,
    envelope_digest: Digest,
}

/// Requires the stated scope of one scenario to be the one its document
/// commands.
///
/// A table row can otherwise carry any family and any envelope identity and
/// still be internally consistent, because every other check compares the
/// table against itself. This one compares it against the executed mission,
/// which is the artifact the run actually flew.
///
/// A scenario with no rows is a training scenario, which the table does not
/// scope, so it passes.
///
/// # Errors
///
/// Returns [`TuneError`] when the document commands no stimulus, commands
/// more than one, or commands a family, channel, or envelope the table does
/// not name.
pub(crate) fn verify_document(
    table: &ResponseTargetTable,
    mission_revision_id: &str,
    document: &MissionDocument,
) -> Result<(), TuneError> {
    let Some(row) = table
        .targets
        .iter()
        .find(|target| target.mission_revision_id == mission_revision_id)
    else {
        return Ok(());
    };
    let scope = document_scope(mission_revision_id, document)?;
    if scope.family != row.control_family || scope.channel != row.control_channel {
        return Err(invalid_table(format!(
            "the response target scope of {mission_revision_id} names another control family"
        )));
    }
    if scope.envelope_digest != row.envelope_digest {
        return Err(invalid_table(format!(
            "the response target scope of {mission_revision_id} names another stimulus envelope"
        )));
    }
    Ok(())
}

/// The one stimulus a scoped mission commands.
///
/// A scoped scenario measures one physical response, so it commands exactly
/// one stimulus. Two would leave the row's family ambiguous, and none would
/// leave it unsupported by anything the vehicle was asked to do.
fn document_scope(
    mission_revision_id: &str,
    document: &MissionDocument,
) -> Result<DocumentScope, TuneError> {
    let mut found: Option<DocumentScope> = None;
    for phase in &document.phases {
        let MissionAction::Trial(TrialAction::Stimulate {
            family,
            channel,
            envelope,
            ..
        }) = &phase.action
        else {
            continue;
        };
        if found.is_some() {
            return Err(invalid_table(format!(
                "the scoped mission {mission_revision_id} commands two stimuli"
            )));
        }
        let envelope_digest = envelope.canonical_digest().map_err(|source| {
            invalid_table(format!(
                "the stimulus envelope of {mission_revision_id} cannot encode: {source}"
            ))
        })?;
        found = Some(DocumentScope {
            family: *family,
            channel: *channel,
            envelope_digest: Digest::from_bytes(*envelope_digest.as_bytes()),
        });
    }
    found.ok_or_else(|| {
        invalid_table(format!(
            "the scoped mission {mission_revision_id} commands no stimulus"
        ))
    })
}
