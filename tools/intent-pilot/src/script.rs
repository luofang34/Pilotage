//! Release of the scripted operator messages.

use crate::scenario::{Intent, OperatorMessage, Trigger};

/// The scenario's messages, released one at a time in order.
#[derive(Debug)]
pub(crate) struct Script {
    messages: Vec<OperatorMessage>,
    next: usize,
    /// True from a release until the classifier answers it. The next
    /// message waits, so its en-route time counts from the new intent.
    awaiting_reading: bool,
}

impl Script {
    /// A script at its first message.
    pub(crate) fn new(messages: Vec<OperatorMessage>) -> Self {
        Self {
            messages,
            next: 0,
            awaiting_reading: false,
        }
    }

    /// The message whose trigger holds now, if one does. `enroute_s` is
    /// the vehicle's continuous en-route time.
    pub(crate) fn release(&mut self, enroute_s: Option<f64>) -> Option<&OperatorMessage> {
        if self.awaiting_reading {
            return None;
        }
        let message = self.messages.get(self.next)?;
        let due = match message.trigger {
            Trigger::Start => true,
            Trigger::EnrouteFor { seconds } => enroute_s.is_some_and(|enroute| enroute >= seconds),
        };
        if !due {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        self.awaiting_reading = true;
        Some(message)
    }

    /// Marks the classifier's answer to the last release and returns the
    /// intent the author wrote for that message.
    pub(crate) fn answered(&mut self) -> Option<&Intent> {
        self.awaiting_reading = false;
        let last = self.next.checked_sub(1)?;
        self.messages.get(last).map(|message| &message.means)
    }
}

#[cfg(test)]
mod tests {
    use super::Script;
    use crate::scenario::{Arrival, Intent, OperatorMessage, Trigger};

    fn message(text: &str, trigger: Trigger) -> OperatorMessage {
        OperatorMessage {
            text: text.to_owned(),
            trigger,
            means: Intent {
                target: "HOME".to_owned(),
                on_arrival: Arrival::Land,
            },
        }
    }

    #[test]
    fn the_second_message_waits_for_the_answer_and_then_for_its_trigger() {
        let mut script = Script::new(vec![
            message("first", Trigger::Start),
            message("second", Trigger::EnrouteFor { seconds: 4.0 }),
        ]);
        assert_eq!(script.release(None).map(|m| m.text.as_str()), Some("first"));
        assert!(script.release(Some(9.0)).is_none(), "no answer yet");
        assert!(script.answered().is_some());
        assert!(
            script.release(Some(3.9)).is_none(),
            "not en route long enough"
        );
        assert_eq!(
            script.release(Some(4.0)).map(|m| m.text.as_str()),
            Some("second")
        );
        assert!(script.answered().is_some());
        assert!(script.release(Some(99.0)).is_none(), "the script is done");
    }
}
