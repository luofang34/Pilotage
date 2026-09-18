//! The directive vocabulary.
//!
//! The kinds follow the phrases of air traffic control, because an operator
//! knows them. A directive is data. The executor gives it a meaning.

use serde::{Deserialize, Serialize};

/// What the vehicle does at a fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrival {
    /// Descend, touch down and disarm.
    Land,
    /// Stay at the fix at the commanded height.
    Hold,
}

/// The side of a turn to a heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnDirection {
    /// Turn to the left, the long way if necessary.
    Left,
    /// Turn to the right, the long way if necessary.
    Right,
    /// Turn the short way.
    Shortest,
}

/// Where a hold is flown.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "at", content = "fix", rename_all = "snake_case")]
pub enum HoldPoint {
    /// The position at the moment the directive arrives.
    PresentPosition,
    /// A named fix.
    Fix(String),
}

/// One instruction that the agent can fly.
///
/// A kind with no slot is written `Land {}` and not `Land`. With serde, a unit
/// variant of a tagged enum accepts unknown fields, and a model could then add
/// a slot of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Directive {
    /// Arm, climb to the commanded height and hold there.
    Takeoff {},
    /// Fly to a fix.
    DirectTo {
        /// The fix name.
        fix: String,
        /// What to do at the fix.
        on_arrival: Arrival,
    },
    /// Turn to a heading and fly it.
    Heading {
        /// Degrees true, 0 to 359.
        degrees: u16,
        /// The side of the turn.
        turn: TurnDirection,
    },
    /// Climb or descend to a height and keep it.
    Altitude {
        /// Metres above the launch point.
        height_m: f64,
    },
    /// Change the cruise speed.
    Speed {
        /// Metres per second over the ground.
        speed_mps: f64,
    },
    /// Hold at a point.
    Hold {
        /// Where to hold.
        point: HoldPoint,
    },
    /// Fly a named procedure: its fixes in sequence, then its end behaviour.
    JoinProcedure {
        /// The procedure name.
        procedure: String,
    },
    /// Descend at the present position, touch down and disarm.
    Land {},
    /// Stop a descent, climb to the commanded height and hold.
    GoAround {},
    /// Fly to the launch point and land.
    ReturnToBase {},
    /// The message is not an instruction that the agent can fly.
    Unable {
        /// Why, in the words of the model or its adapter.
        reason: String,
    },
}

/// A directive kind without its slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveKind {
    /// See [`Directive::Takeoff`].
    Takeoff,
    /// See [`Directive::DirectTo`].
    DirectTo,
    /// See [`Directive::Heading`].
    Heading,
    /// See [`Directive::Altitude`].
    Altitude,
    /// See [`Directive::Speed`].
    Speed,
    /// See [`Directive::Hold`].
    Hold,
    /// See [`Directive::JoinProcedure`].
    JoinProcedure,
    /// See [`Directive::Land`].
    Land,
    /// See [`Directive::GoAround`].
    GoAround,
    /// See [`Directive::ReturnToBase`].
    ReturnToBase,
    /// See [`Directive::Unable`].
    Unable,
}

impl DirectiveKind {
    /// Every kind, in the order of the vocabulary table.
    pub const ALL: [Self; 11] = [
        Self::Takeoff,
        Self::DirectTo,
        Self::Heading,
        Self::Altitude,
        Self::Speed,
        Self::Hold,
        Self::JoinProcedure,
        Self::Land,
        Self::GoAround,
        Self::ReturnToBase,
        Self::Unable,
    ];
}

impl Directive {
    /// The kind of this directive.
    #[must_use]
    pub const fn kind(&self) -> DirectiveKind {
        match self {
            Self::Takeoff {} => DirectiveKind::Takeoff,
            Self::DirectTo { .. } => DirectiveKind::DirectTo,
            Self::Heading { .. } => DirectiveKind::Heading,
            Self::Altitude { .. } => DirectiveKind::Altitude,
            Self::Speed { .. } => DirectiveKind::Speed,
            Self::Hold { .. } => DirectiveKind::Hold,
            Self::JoinProcedure { .. } => DirectiveKind::JoinProcedure,
            Self::Land {} => DirectiveKind::Land,
            Self::GoAround {} => DirectiveKind::GoAround,
            Self::ReturnToBase {} => DirectiveKind::ReturnToBase,
            Self::Unable { .. } => DirectiveKind::Unable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Arrival, Directive, HoldPoint, TurnDirection};

    fn round_trip(directive: &Directive, json: &str) {
        let encoded = serde_json::to_string(directive).unwrap_or_default();
        assert_eq!(encoded, json);
        let decoded: Result<Directive, _> = serde_json::from_str(json);
        assert_eq!(decoded.ok().as_ref(), Some(directive));
    }

    #[test]
    fn a_directive_has_one_stable_json_form() {
        round_trip(&Directive::Takeoff {}, r#"{"kind":"takeoff"}"#);
        round_trip(
            &Directive::DirectTo {
                fix: "BRAVO".into(),
                on_arrival: Arrival::Land,
            },
            r#"{"kind":"direct_to","fix":"BRAVO","on_arrival":"land"}"#,
        );
        round_trip(
            &Directive::Heading {
                degrees: 270,
                turn: TurnDirection::Left,
            },
            r#"{"kind":"heading","degrees":270,"turn":"left"}"#,
        );
        round_trip(
            &Directive::Hold {
                point: HoldPoint::PresentPosition,
            },
            r#"{"kind":"hold","point":{"at":"present_position"}}"#,
        );
        round_trip(
            &Directive::Hold {
                point: HoldPoint::Fix("ALPHA".into()),
            },
            r#"{"kind":"hold","point":{"at":"fix","fix":"ALPHA"}}"#,
        );
    }

    #[test]
    fn an_unknown_slot_is_refused() {
        let decoded: Result<Directive, _> =
            serde_json::from_str(r#"{"kind":"land","immediately":true}"#);
        assert!(decoded.is_err(), "a model cannot add a slot of its own");
    }
}
