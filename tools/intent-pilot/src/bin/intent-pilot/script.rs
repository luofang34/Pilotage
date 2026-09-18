//! Release of the scripted operator messages.

use pilotage_agent::{Directive, OperatorMessage, Trigger};

/// The scenario's messages, released one at a time in sequence.
#[derive(Debug)]
pub(crate) struct Script {
    messages: Vec<OperatorMessage>,
    next: usize,
    /// True from a release until the model answers it. The next message
    /// waits, so its flying time counts from the new directive.
    awaiting_reply: bool,
}

impl Script {
    /// A script at its first message.
    pub(crate) fn new(messages: Vec<OperatorMessage>) -> Self {
        Self {
            messages,
            next: 0,
            awaiting_reply: false,
        }
    }

    /// The message whose trigger holds now, if one does. `flying_s` is the
    /// time that the vehicle has flown the newest directive.
    pub(crate) fn release(&mut self, flying_s: Option<f64>) -> Option<&OperatorMessage> {
        if self.awaiting_reply {
            return None;
        }
        let message = self.messages.get(self.next)?;
        let due = match message.trigger {
            Trigger::Start {} => true,
            Trigger::FlyingFor { seconds } => flying_s.is_some_and(|flying| flying >= seconds),
        };
        if !due {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        self.awaiting_reply = true;
        Some(message)
    }

    /// Marks the model's answer to `text`. When `text` is the scripted
    /// message that was released last, it returns the directive that the
    /// author wrote for it. A live operator message has no such directive.
    pub(crate) fn answered(&mut self, text: &str) -> Option<&Directive> {
        let last = self.next.checked_sub(1)?;
        let message = self.messages.get(last)?;
        if !self.awaiting_reply || message.text != text {
            return None;
        }
        self.awaiting_reply = false;
        Some(&message.means)
    }
}

#[cfg(test)]
mod tests {
    use super::Script;
    use pilotage_agent::{Directive, OperatorMessage, Trigger};

    fn message(text: &str, trigger: Trigger) -> OperatorMessage {
        OperatorMessage {
            text: text.to_owned(),
            trigger,
            means: Directive::ReturnToBase {},
        }
    }

    #[test]
    fn the_second_message_waits_for_the_answer_and_then_for_its_trigger() {
        let mut script = Script::new(vec![
            message("first", Trigger::Start {}),
            message("second", Trigger::FlyingFor { seconds: 4.0 }),
        ]);
        assert_eq!(script.release(None).map(|m| m.text.as_str()), Some("first"));
        assert!(script.release(Some(9.0)).is_none(), "no answer yet");
        assert!(
            script.answered("typed by a person").is_none(),
            "not the scripted text"
        );
        assert!(script.answered("first").is_some());
        assert!(
            script.release(Some(3.9)).is_none(),
            "not flying long enough"
        );
        assert_eq!(
            script.release(Some(4.0)).map(|m| m.text.as_str()),
            Some("second")
        );
        assert!(script.answered("second").is_some());
        assert!(script.release(Some(99.0)).is_none(), "the script is done");
    }
}
