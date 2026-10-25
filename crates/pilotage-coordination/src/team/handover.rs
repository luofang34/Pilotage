//! Work that moves between two team members that both report (ADR-0052).
//! The work, and the space it needs, stay with the giving member until it
//! reports that it released them: its status names the assignment, the
//! generation, the revision the team approved, the run it replaced and the
//! tasks that run did not begin. Only then does the team commit the work to
//! the receiving member, one generation on, and send it there: the moved
//! tasks the release names. A moved task the giving member began before the
//! revision applied is not released, and it stays with that member. A release that does not come leaves the handover open; nothing is
//! assumed. A refusal withdraws it, and the giving member keeps the work.
//! Work released and then not taken by the receiving member is unassigned:
//! it does not go back to the giving member.
//!
//! The host writes the handovers to durable storage before it sends work to
//! a member. A coordinator that restarts reads them back: each handover that
//! was open or committed is unreconciled, shown to the operator, as it is not
//! known whether a member flies that work. An assignment's identity includes
//! the team's run, so a restarted coordinator never reuses an identity or a
//! generation, and it has no run to commit into.

use pilotage_mission::IntentContext;
use pilotage_protocol::wire;
use wire::MissionHandoverState as State;

use super::refly::ReflyState;
use super::{MemberIntent, TeamLane};

/// One handover.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Handover {
    pub assignment_id: String,
    pub generation: u64,
    pub from: String,
    pub to: String,
    /// The moved tasks, as the giving member names them, with their paths.
    pub moved: Vec<(String, Vec<wire::GeoPoint>)>,
    /// The giving member's run the release is asked of.
    pub from_run: String,
    /// The digest of the giving member's draft the team approved.
    pub approved: Option<Vec<u8>>,
    /// The giving member reported that it released the work. Without a
    /// release, as after a person's statement, the giver may still fly it.
    pub released: bool,
    pub state: State,
    pub basis: String,
}

impl Handover {
    /// The handover ended in a state nobody needs to act on: withdrawn,
    /// flown by its receiver, or resolved by a person.
    pub(super) fn settled(&self) -> bool {
        matches!(
            self.state,
            State::Withdrawn | State::Completed | State::Resolved
        )
    }

    /// The receiver will not fly the moved lines, for `why`. Work its giver
    /// released is unassigned. Work moved without a release may still be
    /// flown by its giver, so the handover is unreconciled.
    fn not_flown_by_receiver(&mut self, why: &str) {
        if self.released {
            self.state = State::Unassigned;
            self.basis = format!(
                "{} released the work and {why}; an operator gives it to a member",
                self.from
            );
        } else {
            self.state = State::Unreconciled;
            self.basis = format!(
                "{why}; {} did not release the work and may still fly it; check whether {} flies these lines, then give them to a member",
                self.from, self.from
            );
        }
    }

    fn wire(&self) -> wire::MissionHandover {
        wire::MissionHandover {
            assignment_id: self.assignment_id.clone(),
            generation: self.generation,
            from_member: self.from.clone(),
            to_member: self.to.clone(),
            state: self.state as i32,
            task_ids: self.moved.iter().map(|(id, _)| id.clone()).collect(),
            basis: self.basis.clone(),
        }
    }

    /// Whether `released` is this handover's release: its assignment and
    /// generation, the revision the team approved, and the run it was asked
    /// of.
    fn released_by(&self, released: &wire::WorkReleased) -> bool {
        released.assignment_id == self.assignment_id
            && released.generation == self.generation
            && self.approved.as_deref() == Some(released.digest.as_slice())
            && released.superseded_run_id == self.from_run
    }
}

impl TeamLane {
    /// Opens a handover of `moved` from `from` to `to`: the release `from`
    /// is asked for.
    pub(super) fn open_handover(
        &mut self,
        from: &str,
        to: &str,
        moved: &[Vec<wire::GeoPoint>],
    ) -> wire::WorkRelease {
        let assignment_id = format!(
            "{}/{}/handover-{}",
            self.config.name,
            self.run_id.clone().unwrap_or_default(),
            self.handovers.len().wrapping_add(1)
        );
        let moved = self.moved_tasks(from, moved);
        let from_run = self
            .statuses
            .get(from)
            .map(|s| s.run_id.clone())
            .unwrap_or_default();
        self.handovers.push(Handover {
            assignment_id: assignment_id.clone(),
            generation: 1,
            from: from.to_owned(),
            to: to.to_owned(),
            moved,
            from_run,
            approved: None,
            released: false,
            state: State::Releasing,
            basis: format!("waiting for {from} to release the work"),
        });
        wire::WorkRelease {
            assignment_id,
            generation: 1,
        }
    }

    /// Records a handover a person's statement that `from` is down commits
    /// at once: an override, not a release. Returns its assignment.
    pub(super) fn override_handover(
        &mut self,
        from: &str,
        to: &str,
        moved: &[Vec<wire::GeoPoint>],
    ) -> String {
        let stated_by = self
            .stated_by
            .get(from)
            .cloned()
            .unwrap_or_else(|| "a person".to_owned());
        let moved = self.moved_tasks(from, moved);
        let assignment_id = format!(
            "{}/{}/handover-{}",
            self.config.name,
            self.run_id.clone().unwrap_or_default(),
            self.handovers.len().wrapping_add(1)
        );
        self.handovers.push(Handover {
            assignment_id: assignment_id.clone(),
            generation: 2,
            from: from.to_owned(),
            to: to.to_owned(),
            moved,
            from_run: String::new(),
            approved: None,
            released: false,
            state: State::Committed,
            basis: format!(
                "{stated_by} stated that {from} is down; this does not show that {from} stopped or that its airspace is clear"
            ),
        });
        assignment_id
    }

    /// The tasks of `from` on the paths `moved`, as `from` names them.
    fn moved_tasks(
        &self,
        from: &str,
        moved: &[Vec<wire::GeoPoint>],
    ) -> Vec<(String, Vec<wire::GeoPoint>)> {
        let prefix = format!("{from}/");
        self.book
            .unfinished_lines(from)
            .into_iter()
            .filter(|(_, path)| moved.contains(path))
            .filter_map(|(id, path)| Some((id.strip_prefix(&prefix)?.to_owned(), path)))
            .collect()
    }

    /// The open handover `member` gives.
    fn releasing(&mut self, member: &str) -> Option<&mut Handover> {
        self.handovers
            .iter_mut()
            .find(|h| h.from == member && h.state == State::Releasing)
    }

    /// Whether `draft`, `member`'s share of the revision, still flies a line
    /// its open handover moves: then the team does not approve it.
    pub(super) fn keeps_moved_work(&self, member: &str, draft: &wire::MissionProposal) -> bool {
        let Some(handover) = self
            .handovers
            .iter()
            .find(|h| h.from == member && h.state == State::Releasing)
        else {
            return false;
        };
        let (Some(across), Some(sweep)) = (self.across(), self.sweep) else {
            return true;
        };
        let moved: Vec<f64> = handover
            .moved
            .iter()
            .filter_map(|(_, p)| across.of_path(p))
            .collect();
        draft
            .tasks
            .iter()
            .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
            .filter_map(|t| across.of_path(&t.path))
            .any(|at| moved.iter().any(|m| (at - m).abs() < sweep.spacing_m / 2.0))
    }

    /// Notes the digest of `member`'s draft the team approves.
    pub(super) fn approving_release(&mut self, member: &str, digest: &[u8]) {
        if let Some(handover) = self.releasing(member) {
            handover.approved = Some(digest.to_vec());
        }
    }

    /// `member` refused its share: its open handover is withdrawn and it
    /// keeps the work.
    pub(super) fn withdraw_handover(&mut self, member: &str, reason: &str) {
        if let Some(handover) = self.releasing(member) {
            handover.state = State::Withdrawn;
            handover.basis = format!("{member} refused: {reason}; it keeps the work");
            if let Some(revision) = self.revision.as_mut() {
                revision.stage = wire::MissionSuccessorState::Withdrawn;
            }
        }
    }

    /// Commits the open handover of `member` when its status reports the
    /// release, and sends the moved lines to the receiving member.
    pub(super) fn commit_released(
        &mut self,
        member: &str,
        status: &wire::MissionStatus,
        context: &IntentContext,
    ) -> Vec<MemberIntent> {
        let Some((handover, released)) = self.handovers.iter_mut().find_map(|h| {
            let released = status.released.iter().find(|r| h.released_by(r))?;
            (h.from == member && h.state == State::Releasing).then_some((h, released))
        }) else {
            return Vec::new();
        };
        let (kept, moved): (Vec<_>, Vec<_>) = handover
            .moved
            .drain(..)
            .partition(|(id, _)| !released.task_ids.contains(id));
        handover.moved = moved;
        handover.released = true;
        if !matches!(
            self.phase,
            super::Phase::Running | super::Phase::Held | super::Phase::Settling
        ) {
            // The release happened; the abort ended the work it was for.
            // Lines the giver began first are not released and stay its own.
            if handover.moved.is_empty() {
                handover.state = State::Withdrawn;
                handover.basis = format!("{member} released none of the moved lines");
            } else {
                handover.state = State::Unassigned;
                handover.basis =
                    format!("{member} released the work after the team aborted; nobody flies it");
            }
            return Vec::new();
        }
        if handover.moved.is_empty() {
            handover.state = State::Withdrawn;
            handover.basis = format!("{member} began every moved line before it released any");
            return Vec::new();
        }
        handover.state = State::Committed;
        handover.generation = handover.generation.wrapping_add(1);
        handover.basis = if kept.is_empty() {
            format!("{member} released the work")
        } else {
            format!(
                "{member} released the work, except the {} it began first",
                kept.len()
            )
        };
        let to = handover.to.clone();
        let assignment = handover.assignment_id.clone();
        let paths: Vec<Vec<wire::GeoPoint>> =
            handover.moved.iter().map(|(_, p)| p.clone()).collect();
        self.detail = format!("{member} released {} lines; {to} flies them", paths.len());
        self.queue_moved(&to, member, paths, assignment, context)
    }

    /// The moved work of `assignment` did not start: it is unassigned when
    /// its giver released it.
    pub(super) fn unassign_moved(&mut self, assignment: &str, reason: &str) {
        for handover in &mut self.handovers {
            if handover.assignment_id == assignment && handover.state == State::Committed {
                handover
                    .not_flown_by_receiver(&format!("{} did not take it ({reason})", handover.to));
            }
        }
    }

    /// A person stated `member` down: moved work it had not started is
    /// unassigned.
    pub(super) fn unassign_queued_for(&mut self, member: &str) {
        let queued: Vec<String> = self
            .reflies
            .iter_mut()
            .filter(|r| r.member == member && r.state == ReflyState::Queued)
            .filter_map(|r| {
                let assignment = r.assignment.clone()?;
                r.state = ReflyState::Failed;
                Some(assignment)
            })
            .collect();
        for assignment in queued {
            self.unassign_moved(&assignment, "it is stated down");
        }
    }

    /// Refuses new work while a handover of this team's members is
    /// unreconciled: whether a member flies its work is not known.
    pub(super) fn refuse_while_unreconciled(&self) -> Result<(), String> {
        match self
            .handovers
            .iter()
            .find(|h| h.state == State::Unreconciled)
        {
            Some(h) => Err(format!(
                "handover {} is unreconciled: check whether {} or {} flies its lines, then resolve it",
                h.assignment_id, h.from, h.to
            )),
            None => Ok(()),
        }
    }

    /// A person states the unreconciled or unassigned handover `resolve`
    /// names resolved.
    pub(super) fn resolve_handover(
        &mut self,
        resolve: &wire::MissionResolveHandover,
    ) -> Result<(Vec<MemberIntent>, bool), String> {
        let handover = self
            .handovers
            .iter_mut()
            .find(|h| h.assignment_id == resolve.assignment_id)
            .ok_or_else(|| format!("no handover {}", resolve.assignment_id))?;
        if !matches!(handover.state, State::Unreconciled | State::Unassigned) {
            return Err(format!(
                "handover {} is {:?}; only an unreconciled or unassigned one is resolved by a person",
                handover.assignment_id, handover.state
            ));
        }
        let by = resolve
            .resolved_by
            .as_ref()
            .map_or_else(|| "a person".to_owned(), |o| o.subject.clone());
        handover.state = State::Resolved;
        handover.basis = format!(
            "{by} resolved it after checking {} and {}",
            handover.from, handover.to
        );
        self.detail = format!("handover {} is resolved", handover.assignment_id);
        Ok((Vec::new(), false))
    }

    /// The flight of the moved lines of `assignment` ended: flown to its
    /// end, or failed part of the way with nobody holding the rest.
    pub(super) fn moved_flight_ended(&mut self, assignment: &str, completed: bool) {
        for handover in &mut self.handovers {
            if handover.assignment_id == assignment && handover.state == State::Committed {
                if completed {
                    handover.state = State::Completed;
                    handover.basis = format!("{} flew the moved lines", handover.to);
                } else {
                    handover.not_flown_by_receiver(&format!(
                        "{}'s flight of the moved lines ended before they were flown",
                        handover.to
                    ));
                }
            }
        }
    }

    /// A team whose run ended starts new work: a handover still open or
    /// committed has no run to follow any more, so whether a member flies
    /// its work is not known.
    pub(super) fn orphan_live_handovers(&mut self) {
        for handover in &mut self.handovers {
            if matches!(handover.state, State::Releasing | State::Committed) {
                handover.basis = format!(
                    "the team's run ended while this handover was {}; check whether {} or {} flies its lines, then resolve it",
                    if handover.state == State::Releasing {
                        "open"
                    } else {
                        "committed"
                    },
                    handover.from,
                    handover.to
                );
                handover.state = State::Unreconciled;
            }
        }
    }

    /// The handovers, as the host writes them before it sends any intent.
    #[must_use]
    pub fn handovers(&self) -> Vec<wire::MissionHandover> {
        self.handovers.iter().map(Handover::wire).collect()
    }

    /// Takes the handovers a coordinator wrote before it restarted. One that
    /// was open or committed is unreconciled: the restarted team cannot know
    /// whether a member flies its work, so it shows it and moves nothing.
    pub fn restore_handovers(&mut self, records: &[wire::MissionHandover]) {
        for record in records {
            if self
                .handovers
                .iter()
                .any(|h| h.assignment_id == record.assignment_id)
            {
                continue;
            }
            let open = matches!(
                State::try_from(record.state),
                Ok(State::Releasing | State::Committed)
            );
            let mut restored = Handover {
                assignment_id: record.assignment_id.clone(),
                generation: record.generation,
                from: record.from_member.clone(),
                to: record.to_member.clone(),
                moved: record
                    .task_ids
                    .iter()
                    .map(|id| (id.clone(), Vec::new()))
                    .collect(),
                from_run: String::new(),
                approved: None,
                released: false,
                state: State::try_from(record.state).unwrap_or(State::Unspecified),
                basis: record.basis.clone(),
            };
            if open {
                restored.state = State::Unreconciled;
                restored.basis = format!(
                    "the coordinator restarted while this handover was {}; check whether {} or {} flies these lines, then give them to a member",
                    if record.state == State::Releasing as i32 {
                        "open"
                    } else {
                        "committed"
                    },
                    record.from_member,
                    record.to_member
                );
            }
            self.handovers.push(restored);
        }
        self.changed = true;
    }
}
