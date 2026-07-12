//! The deterministic alert state machine.
//!
//! [`AlertManager::step`] is pure over its inputs and the manager's stored
//! state: it reads no clock, allocates nothing, and never panics. One step
//! applies the events, re-arms escalations whose deadline has passed,
//! arbitrates the single aural command, and produces a sorted
//! [`AlertOutput`]. Each managed alert is one slot in a fixed array keyed
//! by identity, so duplicates collapse and ordering is total.

use crate::class::{AlertClass, AlertState, AuralToken};
use crate::condition::{AlertCondition, AlertId};
use crate::event::{AlertContext, AlertEvent};
use crate::output::{ActiveAlert, AlertOutput, MAX_ACTIVE_ALERTS, ManagerHealth};
use crate::profile::AlertProfile;

/// One tracked alert. Present-but-unacknowledged is [`AlertState::Active`];
/// present-and-acknowledged is [`AlertState::Acknowledged`]; a slot that is
/// not present exists only for a latched, unacknowledged, cleared alert.
#[derive(Debug, Clone, Copy)]
struct Alert {
    id: AlertId,
    class: AlertClass,
    present: bool,
    acknowledged: bool,
    aural_armed: bool,
    last_aural_ms: u64,
    generation: u32,
}

impl Alert {
    fn new(id: AlertId, class: AlertClass, now_ms: u64, generation: u32) -> Self {
        Self {
            id,
            class,
            present: true,
            acknowledged: false,
            aural_armed: true,
            last_aural_ms: now_ms,
            generation,
        }
    }
}

/// The central alert manager. Bounded, deterministic, and clock-free.
#[derive(Debug, Clone)]
pub struct AlertManager {
    slots: [Option<Alert>; MAX_ACTIVE_ALERTS],
    generation: u32,
    overflow_dropped: u32,
}

impl Default for AlertManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AlertManager {
    /// A manager with no active alerts.
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_ACTIVE_ALERTS],
            generation: 0,
            overflow_dropped: 0,
        }
    }

    /// Advances the machine by one step and returns the resulting output.
    ///
    /// The same `(profile, events, ctx, now_ms)` applied to the same
    /// manager state produces byte-identical output and ordering; nothing
    /// here reads an interior clock.
    pub fn step(
        &mut self,
        profile: &AlertProfile,
        events: &[AlertEvent],
        ctx: AlertContext,
        now_ms: u64,
    ) -> AlertOutput {
        let mut overflowed = false;
        for &event in events {
            overflowed |= self.apply_event(event, now_ms);
        }
        self.apply_escalation(profile, now_ms);
        let aural = self.arbitrate(profile, ctx, now_ms);
        self.build_output(profile, ctx, aural, overflowed)
    }

    fn apply_event(&mut self, event: AlertEvent, now_ms: u64) -> bool {
        match event {
            AlertEvent::Assert(cond) => self.assert(cond, now_ms),
            AlertEvent::Clear(cond) => {
                self.clear(cond.id());
                false
            }
            AlertEvent::Acknowledge(id) => {
                self.acknowledge(id);
                false
            }
            AlertEvent::AcknowledgeAll => {
                self.acknowledge_all();
                false
            }
        }
    }

    fn assert(&mut self, cond: AlertCondition, now_ms: u64) -> bool {
        let id = cond.id();
        if let Some(i) = self.index_of(id) {
            if self.slots[i].is_some_and(|a| !a.present) {
                let generation = self.next_generation();
                if let Some(a) = self.slots[i].as_mut() {
                    a.present = true;
                    a.acknowledged = false;
                    a.aural_armed = true;
                    a.last_aural_ms = now_ms;
                    a.generation = generation;
                }
            }
            return false;
        }
        self.insert(id, cond.class(), now_ms)
    }

    /// Inserts a new alert. Returns `true` when capacity forced a drop: a
    /// higher-priority newcomer evicts the lowest-priority slot, otherwise
    /// the newcomer itself is dropped. Either way the wrapping drop counter
    /// advances and the drop is never silent.
    fn insert(&mut self, id: AlertId, class: AlertClass, now_ms: u64) -> bool {
        if let Some(i) = self.free_slot() {
            let generation = self.next_generation();
            self.slots[i] = Some(Alert::new(id, class, now_ms, generation));
            return false;
        }
        if let Some(i) = self.lowest_priority_index()
            && let Some(loser) = self.slots[i]
            && ranks_above(class, id, loser.class, loser.id)
        {
            let generation = self.next_generation();
            self.slots[i] = Some(Alert::new(id, class, now_ms, generation));
        }
        self.overflow_dropped = self.overflow_dropped.wrapping_add(1);
        true
    }

    fn clear(&mut self, id: AlertId) {
        let Some(i) = self.index_of(id) else { return };
        let Some(alert) = self.slots[i] else { return };
        if alert.class.latches() && !alert.acknowledged {
            if let Some(a) = self.slots[i].as_mut() {
                a.present = false;
                a.aural_armed = false;
            }
        } else {
            self.slots[i] = None;
        }
    }

    fn acknowledge(&mut self, id: AlertId) {
        if let Some(i) = self.index_of(id) {
            self.ack_index(i);
        }
    }

    fn acknowledge_all(&mut self) {
        let mut i = 0;
        while i < MAX_ACTIVE_ALERTS {
            if self.slots[i].is_some() {
                self.ack_index(i);
            }
            i += 1;
        }
    }

    fn ack_index(&mut self, i: usize) {
        let Some(alert) = self.slots[i] else { return };
        if alert.class.latches() && !alert.present {
            self.slots[i] = None;
        } else if let Some(a) = self.slots[i].as_mut() {
            a.acknowledged = true;
            a.aural_armed = false;
        }
    }

    /// Re-arms a one-shot aural whose escalation deadline has elapsed while
    /// the alert stays active and unacknowledged. Continuous and silent
    /// classes never re-arm this way.
    fn apply_escalation(&mut self, profile: &AlertProfile, now_ms: u64) {
        for slot in self.slots.iter_mut() {
            let Some(a) = slot else { continue };
            if !a.present || a.acknowledged || a.aural_armed {
                continue;
            }
            let token = a.class.aural_token();
            if token.is_continuous() || matches!(token, AuralToken::Silent) {
                continue;
            }
            if now_ms.saturating_sub(a.last_aural_ms) >= profile.escalation_ms(a.class) {
                a.aural_armed = true;
            }
        }
    }

    /// Picks the single highest-priority alert that wants to sound and
    /// returns its token, consuming its one-shot arming. Lower-priority
    /// pending one-shots stay armed and resume in a later step.
    fn arbitrate(&mut self, profile: &AlertProfile, ctx: AlertContext, now_ms: u64) -> AuralToken {
        let best = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, &slot)| slot.map(|a| (i, a)))
            .filter(|(_, a)| sounds(a, profile, ctx))
            .fold(
                None,
                |best: Option<(usize, AlertClass, AlertId)>, (i, a)| match best {
                    Some((_, bc, bid)) if !ranks_above(a.class, a.id, bc, bid) => best,
                    _ => Some((i, a.class, a.id)),
                },
            );
        let Some((i, class, _)) = best else {
            return AuralToken::Silent;
        };
        if let Some(a) = self.slots[i].as_mut() {
            a.aural_armed = false;
            a.last_aural_ms = now_ms;
        }
        class.aural_token()
    }

    fn build_output(
        &self,
        profile: &AlertProfile,
        ctx: AlertContext,
        aural: AuralToken,
        overflowed: bool,
    ) -> AlertOutput {
        let mut out = AlertOutput::empty(self.generation, self.overflow_dropped);
        out.set_aural(aural);
        out.set_overflow(overflowed);
        out.set_health(if ctx.alerting_path_healthy {
            ManagerHealth::Nominal
        } else {
            ManagerHealth::Faulted
        });
        for a in self.slots.iter().flatten() {
            out.push(view(a, profile, ctx));
        }
        out.sort_active();
        out
    }

    fn index_of(&self, id: AlertId) -> Option<usize> {
        self.slots
            .iter()
            .position(|&slot| slot.is_some_and(|a| a.id == id))
    }

    fn free_slot(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    fn lowest_priority_index(&self) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, &slot)| slot.map(|a| (i, a)))
            .fold(
                None,
                |worst: Option<(usize, AlertClass, AlertId)>, (i, a)| match worst {
                    Some((_, wc, wid)) if !ranks_above(wc, wid, a.class, a.id) => worst,
                    _ => Some((i, a.class, a.id)),
                },
            )
            .map(|(i, _, _)| i)
    }

    fn next_generation(&mut self) -> u32 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
}

/// Whether `(a_class, a_id)` outranks `(b_class, b_id)`: higher class wins;
/// within a class, the lower identity wins.
fn ranks_above(a_class: AlertClass, a_id: AlertId, b_class: AlertClass, b_id: AlertId) -> bool {
    a_class > b_class || (a_class == b_class && a_id < b_id)
}

/// Whether an alert would sound this step: present, unacknowledged, not
/// inhibited, not decluttered, and either continuous or armed.
fn sounds(a: &Alert, profile: &AlertProfile, ctx: AlertContext) -> bool {
    if !a.present || a.acknowledged {
        return false;
    }
    let token = a.class.aural_token();
    if matches!(token, AuralToken::Silent) {
        return false;
    }
    if a.class != AlertClass::Warning && profile.is_inhibited(a.id, ctx.phase) {
        return false;
    }
    if ctx.declutter && a.class.declutters_under_unusual() {
        return false;
    }
    token.is_continuous() || a.aural_armed
}

fn view(a: &Alert, profile: &AlertProfile, ctx: AlertContext) -> ActiveAlert {
    let inhibited = a.class != AlertClass::Warning && profile.is_inhibited(a.id, ctx.phase);
    let decluttered = ctx.declutter && a.class.declutters_under_unusual();
    let state = if a.present && !a.acknowledged {
        AlertState::Active
    } else if a.present {
        AlertState::Acknowledged
    } else {
        AlertState::LatchedCleared
    };
    ActiveAlert {
        id: a.id,
        class: a.class,
        state,
        aural: a.class.aural_token(),
        inhibited,
        decluttered,
        generation: a.generation,
    }
}

#[cfg(test)]
mod tests;
