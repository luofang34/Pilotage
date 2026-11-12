//! The flight of one holding pattern in its plane frame: which part of the
//! pattern the vehicle flies, and the path guidance follows there.
//!
//! The frame has its origin at the fix, `x` along the inbound course and
//! `y` toward the holding side, so the pattern always turns right. A
//! course is radians from `x` toward `y`.

mod turn;

use core::f64::consts::{FRAC_PI_4, FRAC_PI_6, PI};

use super::{HoldingEntry, HoldingLeg, HoldingTurn, entry_for};
use turn::{Turn, offset};

/// The angle between the teardrop course and the outbound course, radians.
const TEARDROP_OFFSET_RAD: f64 = FRAC_PI_6;

/// The angle at which the parallel entry turns back to intercept the
/// inbound track, radians.
const PARALLEL_INTERCEPT_RAD: f64 = FRAC_PI_4;

/// The time without a step after which the vehicle enters the pattern
/// again, seconds. A step is missed while no solution is fit to fly or
/// while another principal flies the vehicle: the vehicle can be anywhere
/// when the steps start again.
pub(crate) const REENTRY_GAP_S: f64 = 1.0;

/// The bounds of the outbound time as a share of the leg time: a measured
/// inbound time cannot drive the outbound leg to nothing or without end.
const OUTBOUND_SHARE: (f64, f64) = (0.25, 3.0);

/// The path guidance follows on one step, in the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Steer {
    /// Follow the track from `from` toward `to`.
    Track {
        /// The start of the track.
        from: [f64; 2],
        /// A point ahead on the track.
        to: [f64; 2],
    },
    /// Fly toward `point`.
    Toward {
        /// The guided point.
        point: [f64; 2],
    },
}

/// The part of the pattern flown, with its state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    ToFix {
        from: [f64; 2],
    },
    DirectTurn(Turn),
    /// The first leg of a parallel or teardrop entry.
    EntryLeg {
        entry: HoldingEntry,
        from: [f64; 2],
        course: f64,
        started_s: f64,
    },
    /// The turn of a parallel or teardrop entry back to the inbound track.
    EntryTurn {
        entry: HoldingEntry,
        turn: Turn,
    },
    Inbound {
        from: [f64; 2],
        started_s: f64,
        /// Where along the inbound course the leg started.
        rollout_x: f64,
    },
    FixTurn(Turn),
    Outbound {
        /// When and where along the inbound course the timer started:
        /// abeam the fix, or at once after the turn at the fix, which ends
        /// abeam it.
        started: Option<(f64, f64)>,
    },
    OutboundTurn(Turn),
}

/// The flight of one holding pattern.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PatternFlight {
    radius_m: f64,
    leg_time_s: f64,
    /// The distance to a point far ahead on a straight leg, metres.
    far_m: f64,
    /// The radius within which the vehicle counts as over the fix, metres.
    capture_radius_m: f64,
    outbound_time_s: f64,
    phase: Phase,
    entry: Option<HoldingEntry>,
    /// The entries started, so a host can announce each once.
    entries: u32,
    circuits: u32,
    /// Whether the inbound leg flown follows a whole outbound leg, so its
    /// time measures the outbound time.
    timed_inbound: bool,
    /// The groundspeed of the last timed outbound leg, m/s.
    outbound_speed_mps: Option<f64>,
    last_step_s: Option<f64>,
}

impl PatternFlight {
    /// The flight of a pattern with turns of `radius_m` and an inbound leg
    /// of `leg_time_s`, at `speed_mps` over the ground. The outbound leg
    /// first lasts `outbound_time_s`. Within `capture_radius_m` of the fix
    /// the vehicle counts as over it.
    pub(crate) fn new(
        radius_m: f64,
        leg_time_s: f64,
        speed_mps: f64,
        capture_radius_m: f64,
        outbound_time_s: f64,
    ) -> Self {
        Self {
            radius_m,
            leg_time_s,
            far_m: 3.0 * speed_mps * leg_time_s + 4.0 * radius_m,
            capture_radius_m,
            outbound_time_s: clamp_outbound(outbound_time_s, leg_time_s),
            phase: Phase::ToFix { from: [0.0; 2] },
            entry: None,
            entries: 0,
            circuits: 0,
            timed_inbound: false,
            outbound_speed_mps: None,
            last_step_s: None,
        }
    }

    /// Starts the flight at `position` on `course` at `now_s`. A vehicle
    /// within the capture radius of the fix enters at once; so does one
    /// that `passed` the fix. Any other flies to the fix first.
    pub(crate) fn begin(&mut self, position: [f64; 2], course: f64, now_s: f64, passed: bool) {
        self.last_step_s = Some(now_s);
        if passed || norm(position) <= self.capture_radius_m {
            self.enter(position, course, now_s);
        } else {
            self.phase = Phase::ToFix { from: position };
        }
    }

    /// One step at `position` on the course over the ground `course`, at
    /// `now_s`: the path to follow. After a gap in the steps, or with the
    /// vehicle far outside the pattern, the vehicle enters again: someone
    /// else flew it meanwhile, and the part of the pattern it was in no
    /// longer says where it is.
    pub(crate) fn step(&mut self, position: [f64; 2], course: f64, now_s: f64) -> Steer {
        let gap = self
            .last_step_s
            .is_none_or(|last| now_s - last > REENTRY_GAP_S);
        let away = !matches!(self.phase, Phase::ToFix { .. }) && self.far_outside(position);
        if gap || away {
            self.begin(position, course, now_s, false);
        }
        self.last_step_s = Some(now_s);
        self.advance(position, course, now_s);
        self.steer(position)
    }

    /// Whether `position` is so far outside the pattern that no part of it
    /// or of an entry reaches there.
    fn far_outside(&self, position: [f64; 2]) -> bool {
        let [x, y] = position;
        let margin_m = 2.0 * self.radius_m;
        x > margin_m + self.radius_m
            || x < -self.far_m
            || y < -margin_m
            || y > 2.0 * self.radius_m + margin_m
    }

    /// The part of the pattern flown now.
    pub(crate) fn leg(&self) -> HoldingLeg {
        match self.phase {
            Phase::ToFix { .. } => HoldingLeg::ToFix,
            Phase::DirectTurn(_) => HoldingLeg::Entry(HoldingEntry::Direct),
            Phase::EntryLeg { entry, .. } | Phase::EntryTurn { entry, .. } => {
                HoldingLeg::Entry(entry)
            }
            Phase::Inbound { .. } => HoldingLeg::Inbound,
            Phase::FixTurn(_) => HoldingLeg::FixTurn,
            Phase::Outbound { .. } => HoldingLeg::Outbound,
            Phase::OutboundTurn(_) => HoldingLeg::OutboundTurn,
        }
    }

    /// The entry of the last arrival at the fix.
    pub(crate) const fn entry(&self) -> Option<HoldingEntry> {
        self.entry
    }

    /// The entries started.
    pub(crate) const fn entries(&self) -> u32 {
        self.entries
    }

    /// The circuits completed since the entry.
    pub(crate) const fn circuits(&self) -> u32 {
        self.circuits
    }

    /// The time the outbound leg is flown for, seconds.
    pub(crate) const fn outbound_time_s(&self) -> f64 {
        self.outbound_time_s
    }

    /// Starts the entry the course at the fix selects.
    fn enter(&mut self, position: [f64; 2], course: f64, now_s: f64) {
        let entry = entry_for(0.0, HoldingTurn::Right, course);
        self.entry = Some(entry);
        self.entries = self.entries.wrapping_add(1);
        self.circuits = 0;
        self.timed_inbound = false;
        self.phase = match entry {
            HoldingEntry::Direct => {
                Phase::DirectTurn(Turn::new(position, course, self.radius_m, true, PI))
            }
            HoldingEntry::Teardrop => Phase::EntryLeg {
                entry,
                from: position,
                course: PI - TEARDROP_OFFSET_RAD,
                started_s: now_s,
            },
            HoldingEntry::Parallel => Phase::EntryLeg {
                entry,
                from: position,
                course: PI,
                started_s: now_s,
            },
        };
    }

    /// Moves to the next part of the pattern when the present one ends.
    fn advance(&mut self, position: [f64; 2], course: f64, now_s: f64) {
        match self.phase {
            Phase::ToFix { from } => {
                let passed = position[0] * from[0] + position[1] * from[1] <= 0.0;
                if passed || norm(position) <= self.capture_radius_m {
                    self.enter(position, course, now_s);
                }
            }
            Phase::DirectTurn(mut turn) => {
                self.phase = if turn.advance(position) {
                    Phase::Outbound {
                        started: (position[0] <= 0.0).then_some((now_s, position[0])),
                    }
                } else {
                    Phase::DirectTurn(turn)
                };
            }
            Phase::EntryLeg {
                entry,
                course: leg_course,
                started_s,
                ..
            } => {
                if now_s - started_s >= self.leg_time_s {
                    let (clockwise, exit) = match entry {
                        HoldingEntry::Parallel => (false, -PARALLEL_INTERCEPT_RAD),
                        HoldingEntry::Direct | HoldingEntry::Teardrop => (true, 0.0),
                    };
                    let turn = Turn::new(position, leg_course, self.radius_m, clockwise, exit);
                    self.phase = Phase::EntryTurn { entry, turn };
                }
            }
            Phase::EntryTurn { entry, mut turn } => {
                if turn.advance(position) {
                    self.start_inbound(position, now_s, false);
                } else {
                    self.phase = Phase::EntryTurn { entry, turn };
                }
            }
            Phase::OutboundTurn(mut turn) => {
                if turn.advance(position) {
                    self.start_inbound(position, now_s, true);
                } else {
                    self.phase = Phase::OutboundTurn(turn);
                }
            }
            Phase::Inbound {
                started_s,
                rollout_x,
                ..
            } => {
                if position[0] >= 0.0 {
                    let inbound_s = now_s - started_s;
                    let speed_mps = (position[0] - rollout_x) / inbound_s;
                    self.cross_fix(position, inbound_s, speed_mps);
                }
            }
            Phase::FixTurn(mut turn) => {
                self.phase = if turn.advance(position) {
                    Phase::Outbound {
                        started: Some((now_s, position[0])),
                    }
                } else {
                    Phase::FixTurn(turn)
                };
            }
            Phase::Outbound { started } => self.fly_outbound(position, now_s, started),
        }
    }

    /// One step on the outbound leg, whose timer `started` at a time and a
    /// place: the timer starts abeam the fix, and the outbound turn starts
    /// when it runs out.
    fn fly_outbound(&mut self, position: [f64; 2], now_s: f64, started: Option<(f64, f64)>) {
        let started = started.or((position[0] <= 0.0).then_some((now_s, position[0])));
        self.phase = match started {
            Some((started_s, start_x)) if now_s - started_s >= self.outbound_time_s => {
                self.outbound_speed_mps = Some((start_x - position[0]) / (now_s - started_s));
                Phase::OutboundTurn(Turn::new(position, PI, self.radius_m, true, 0.0))
            }
            _ => Phase::Outbound { started },
        };
    }

    /// Rolls out on the inbound course at `position`: the inbound leg to
    /// the fix starts. Its time measures the outbound time when it follows
    /// a `timed` outbound leg.
    fn start_inbound(&mut self, position: [f64; 2], now_s: f64, timed: bool) {
        self.timed_inbound = timed;
        self.phase = Phase::Inbound {
            from: [position[0].min(0.0) - self.radius_m, 0.0],
            started_s: now_s,
            rollout_x: position[0],
        };
    }

    /// The fix is crossed inbound after `inbound_s` on the inbound leg at
    /// `inbound_speed_mps` over the ground: a circuit ends, the outbound
    /// time takes the error of a timed inbound leg, and the turn at the fix
    /// starts.
    fn cross_fix(&mut self, position: [f64; 2], inbound_s: f64, inbound_speed_mps: f64) {
        self.circuits = self.circuits.wrapping_add(1);
        if self.timed_inbound {
            // A second of outbound time gives about `outbound / inbound`
            // groundspeed seconds of inbound time.
            let ratio = match self.outbound_speed_mps {
                Some(outbound) if outbound > 0.0 && inbound_speed_mps > 0.0 => {
                    inbound_speed_mps / outbound
                }
                _ => 1.0,
            };
            let corrected = self.outbound_time_s + (self.leg_time_s - inbound_s) * ratio;
            self.outbound_time_s = clamp_outbound(corrected, self.leg_time_s);
        }
        self.phase = Phase::FixTurn(Turn::new(position, 0.0, self.radius_m, true, PI));
    }

    /// The path to follow from `position` in the present part.
    fn steer(&self, position: [f64; 2]) -> Steer {
        let fix = [0.0, 0.0];
        match self.phase {
            Phase::ToFix { from } => Steer::Track { from, to: fix },
            Phase::DirectTurn(turn)
            | Phase::EntryTurn { turn, .. }
            | Phase::FixTurn(turn)
            | Phase::OutboundTurn(turn) => Steer::Toward {
                point: turn.lead(position),
            },
            Phase::EntryLeg { from, course, .. } => Steer::Track {
                from,
                to: offset(from, course, self.far_m),
            },
            Phase::Inbound { from, .. } => Steer::Track { from, to: fix },
            Phase::Outbound { .. } => {
                let from = [0.0, 2.0 * self.radius_m];
                Steer::Track {
                    from,
                    to: offset(from, PI, self.far_m),
                }
            }
        }
    }
}

fn norm(point: [f64; 2]) -> f64 {
    point[0].hypot(point[1])
}

fn clamp_outbound(outbound_s: f64, leg_time_s: f64) -> f64 {
    let (low, high) = OUTBOUND_SHARE;
    if outbound_s.is_finite() {
        outbound_s.clamp(low * leg_time_s, high * leg_time_s)
    } else {
        leg_time_s
    }
}
