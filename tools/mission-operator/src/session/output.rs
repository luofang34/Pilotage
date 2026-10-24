//! What the operator prints: a short line a person reads on a terminal, or
//! one JSON object per line for a program, with the same facts either way.
//! A status prints only when its summary changed: the lane's state, the
//! line counts, the tasks in progress, a revision or the commands offered.
//! The full view, with every task's path and evidence, is printed only on
//! request.

use std::io::Write as _;
use std::time::Instant;

use pilotage_client_session::mission::{MissionSummary, MissionView, ProposalView, ReceiptView};
use serde_json::{Value, json};

/// Where the operator's lines go, and in which form.
pub(crate) struct Output {
    human: bool,
    started: Instant,
    last: Option<MissionSummary>,
}

impl Output {
    pub(crate) fn new(human: bool) -> Self {
        Self {
            human,
            started: Instant::now(),
            last: None,
        }
    }

    fn line(&self, text: &str) {
        let seconds = self.started.elapsed().as_secs();
        let stamp = format!("+{:02}:{:02}", seconds / 60, seconds % 60);
        writeln!(std::io::stdout().lock(), "{stamp} {text}").ok();
    }

    fn json(value: &Value) {
        writeln!(std::io::stdout().lock(), "{value}").ok();
    }

    /// Whether the lines are for a person.
    pub(crate) fn human(&self) -> bool {
        self.human
    }

    pub(crate) fn admitted(&self, name: &str) {
        if self.human {
            self.line(&format!("connected as {name}"));
        } else {
            Self::json(&json!({"event": "admitted", "as": name}));
        }
    }

    pub(crate) fn sent(&self, request: &str) {
        if !self.human {
            Self::json(&json!({"event": "sent", "request": request}));
        }
    }

    /// A command that never reached the lane.
    pub(crate) fn refused(&self, by: &str, reason: &str) {
        if self.human {
            self.line(&format!("✗ not sent ({by}): {reason}"));
        } else {
            Self::json(&json!({"event": "refused", "by": by, "reason": reason}));
        }
    }

    pub(crate) fn receipt(&self, receipt: &ReceiptView, proposal: Option<&ProposalView>) {
        if !self.human {
            let mut line = json!({"event": "receipt"});
            merge(&mut line, receipt);
            if let Some(proposal) = proposal {
                line["proposal"] = json!(proposal);
            }
            Self::json(&line);
            return;
        }
        let intent = format!("{:?}", receipt.intent).to_lowercase();
        if receipt.accepted {
            self.line(&format!("✓ {intent} accepted by {}", receipt.lane));
        } else {
            self.line(&format!(
                "✗ {intent} refused by {}: {}",
                receipt.lane, receipt.reason
            ));
        }
        if let Some(p) = proposal {
            let digest: String = p.digest.chars().take(8).collect();
            self.line(&format!(
                "  plan {digest}: {} lines, {:.1} km, {:.0} min, {} members",
                p.lines,
                p.path_length_m / 1000.0,
                p.duration_s / 60.0,
                p.members
            ));
        }
    }

    /// The lane's summary, when it changed.
    pub(crate) fn status(&mut self, view: &MissionView) {
        let summary = view.summary();
        if self.last.as_ref() == Some(&summary) {
            return;
        }
        if self.human {
            self.line(&words(&summary));
        } else {
            let mut line = json!({"event": "status"});
            merge(&mut line, &summary);
            Self::json(&line);
        }
        self.last = Some(summary);
    }

    /// The full view, on request.
    pub(crate) fn full(&self, view: &MissionView) {
        let mut line = json!({"event": "view"});
        merge(&mut line, view);
        Self::json(&line);
    }

    /// What a scripted agent is doing, and why.
    /// The command an instruction became, waiting to be confirmed.
    pub(crate) fn draft(&self, instruction: &pilotage_client_session::mission::Instruction) {
        if self.human {
            self.line(&format!("{} · y sends it", instruction.describe()));
        } else {
            Self::json(&json!({
                "event": "draft",
                "understood": instruction.understood,
                "describes": instruction.describe(),
                "steps": instruction
                    .steps
                    .iter()
                    .map(|s| json!({"lane": s.lane, "action": s.action, "command": s.command()}))
                    .collect::<Vec<_>>(),
            }));
        }
    }

    pub(crate) fn note(&self, text: &str) {
        if self.human {
            self.line(&format!("agent: {text}"));
        } else {
            Self::json(&json!({"event": "agent", "note": text}));
        }
    }

    pub(crate) fn telemetry(&self, lat: f64, lon: f64, height_m: f64, speed_mps: f64) {
        if self.human {
            self.line(&format!(
                "at {lat:.6}, {lon:.6}, {height_m:.0} m, {speed_mps:.1} m/s"
            ));
        } else {
            Self::json(
                &json!({"event": "telemetry", "lat": lat, "lon": lon, "height_m": height_m, "speed_mps": speed_mps}),
            );
        }
    }
}

/// A summary in one line: the lane, the line counts, each member and any
/// revision.
fn words(s: &MissionSummary) -> String {
    let mut text = format!("{} {} · {}/{} lines done", s.lane, s.state, s.done, s.lines);
    for (count, word) in [
        (s.incomplete, "incomplete"),
        (s.skipped, "skipped"),
        (s.reflown as usize, "flown again"),
    ] {
        if count > 0 {
            text.push_str(&format!(" · {count} {word}"));
        }
    }
    for member in &s.members {
        text.push_str(&format!(" · {} {}", member.lane, member.state));
        if !member.active.is_empty() {
            text.push_str(&format!(" ({})", member.active));
        }
    }
    if let Some(revision) = &s.successor {
        text.push_str(&format!(
            " · revision {}: {}",
            revision.state, revision.summary
        ));
        if !revision.applies_after.is_empty() {
            text.push_str(&format!(" (after {})", revision.applies_after));
        }
    }
    if s.members.is_empty() && !s.detail.is_empty() {
        text.push_str(&format!(" · {}", s.detail));
    }
    text
}

/// Adds the fields of `value` to the object `line`.
fn merge(line: &mut Value, value: &impl serde::Serialize) {
    if let (Some(line), Ok(Value::Object(fields))) =
        (line.as_object_mut(), serde_json::to_value(value))
    {
        line.extend(fields);
    }
}
