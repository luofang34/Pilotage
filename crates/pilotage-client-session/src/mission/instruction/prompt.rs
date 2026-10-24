//! The interpreter's request, generated from the chain and the action catalog: the
//! prompt names each lane with what it offers now and each action with its parameters,
//! and the reply's schema allows exactly the catalog's actions and parameters.

use serde_json::{Map, Value, json};

use super::Chain;
use super::catalog::{Action, ParamKind};

/// The most steps one instruction may become.
pub(super) const MAX_STEPS: usize = 4;

impl Chain {
    /// The reply's JSON schema: a restatement and one or more steps, each an action for
    /// a lane with the catalog's parameters.
    pub(super) fn schema(&self) -> Value {
        // Every lane the chain names may appear in a step, so an instruction about a
        // lane this client does not follow is refused rather than sent to its lead.
        let lanes = self.named_lanes();
        let mut names: Vec<&str> = lanes.clone();
        names.push("");
        let mut actions: Vec<&str> = self.actions.iter().map(|a| a.name).collect();
        actions.push("unable");
        let mut properties = Map::new();
        properties.insert("action".into(), json!({"type": "string", "enum": actions}));
        properties.insert("lane".into(), json!({"type": "string", "enum": lanes}));
        for param in self.actions.iter().flat_map(|a| a.params.iter()) {
            let kind = match param.kind {
                ParamKind::Member => json!({"type": "string", "enum": names}),
                ParamKind::Amount => json!({"type": "number"}),
                ParamKind::Area => json!({"type": "string"}),
            };
            properties.entry(param.name).or_insert(kind);
        }
        let required: Vec<&String> = properties.keys().collect();
        let step = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        json!({
            "type": "object",
            "properties": {
                "understood": {"type": "string"},
                "steps": {"type": "array", "items": step, "minItems": 1, "maxItems": MAX_STEPS},
            },
            "required": ["understood", "steps"],
            "additionalProperties": false,
        })
    }

    /// The instructions to the interpreter.
    pub(super) fn prompt(&self, selected: &[String]) -> String {
        let mut text = String::from(
            "You turn an operator's instruction into commands for the lanes of a chain of command of uncrewed aircraft.\nLanes, with what each offers now:\n",
        );
        for lane in &self.lanes {
            let offered: Vec<&str> = self
                .actions
                .iter()
                .filter(|a| a.offered.met(&lane.offers))
                .map(|a| a.name)
                .collect();
            let leads = if lane.members.is_empty() {
                String::new()
            } else {
                format!(" leads {};", lane.members.join(", "))
            };
            let offers = if offered.is_empty() {
                "nothing".to_owned()
            } else {
                offered.join(", ")
            };
            text.push_str(&format!("- {}{leads} offers {offers}\n", lane.lane));
        }
        text.push_str("Actions:\n");
        for action in &self.actions {
            text.push_str(&describe(action));
        }
        text.push_str("- unable: no action fits, for example a kind of mission no lane offers; say why in \"understood\".\n");
        text.push_str(&format!(
            "Give one step per action, in the order the words ask, at most {MAX_STEPS}. A step's \"lane\" receives it: the lowest lane that leads everything the step names. Fill every field; use \"\" and 0 for fields the action does not use. Restate the whole instruction in \"understood\".\n"
        ));
        if let Some(top) = self.top() {
            text.push_str(&format!(
                "The top lane is {top}: use it when the words name no lane. A place name is never a lane.\n"
            ));
        }
        if !selected.is_empty() {
            text.push_str(&format!(
                "The operator has selected: {}. Words such as \"these\" or \"them\" mean the selection.\n",
                selected.join(", ")
            ));
        }
        text
    }

    /// Every lane the chain names, followed or led, once each.
    fn named_lanes(&self) -> Vec<&str> {
        let mut lanes: Vec<&str> = self
            .lanes
            .iter()
            .flat_map(|l| {
                std::iter::once(l.lane.as_str()).chain(l.members.iter().map(String::as_str))
            })
            .collect();
        lanes.sort_unstable();
        lanes.dedup();
        lanes
    }

    /// The lane no followed lane leads.
    fn top(&self) -> Option<&str> {
        self.lanes
            .iter()
            .find(|l| {
                !self
                    .lanes
                    .iter()
                    .any(|other| other.members.contains(&l.lane))
            })
            .map(|l| l.lane.as_str())
    }
}

/// One action's line in the prompt.
fn describe(action: &Action) -> String {
    let params: Vec<String> = action
        .params
        .iter()
        .map(|p| format!("\"{}\" is {}", p.name, p.description))
        .collect();
    if params.is_empty() {
        format!("- {}: {}.\n", action.name, action.description)
    } else {
        format!(
            "- {}: {}; {}.\n",
            action.name,
            action.description,
            params.join("; ")
        )
    }
}
