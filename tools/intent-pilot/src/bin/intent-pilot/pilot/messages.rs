//! Operator messages on their way to the model, and the model's answers on
//! their way to the executor.

use std::time::Instant;

use pilotage_agent::Directive;

use super::{Answer, Ask, Pilot, input};
use crate::error::PilotError;
use crate::record::{Entry, Source};

impl Pilot {
    pub(super) async fn ask(&mut self, text: String, source: Source) {
        self.clock.get_or_insert_with(Instant::now);
        tracing::info!(%text, ?source, "operator message");
        let request = self.flight.request(&text, Vec::new());
        // A closed queue means the model task ended. The run record then has
        // no answer for this message, and the vehicle keeps its directive.
        self.asks.send((Ask { text, source }, request)).await.ok();
    }

    pub(super) async fn on_operator(&mut self, line: input::Line) -> Result<(), PilotError> {
        match line {
            input::Line::Message(text) => self.ask(text, Source::Operator).await,
            input::Line::Quit => {
                tracing::info!("the operator ended the run; the vehicle returns and lands");
                self.quitting = true;
                let now_s = self.elapsed_s();
                let home = Directive::ReturnToBase {};
                if self.flight.fly(&home, now_s).is_err() {
                    tracing::error!("the executor refused the return to base");
                }
            }
        }
        Ok(())
    }

    pub(super) async fn on_answer(&mut self, answer: Answer) -> Result<(), PilotError> {
        let at_s = self.elapsed_s();
        let text = answer.ask.text.as_str();
        let means = self.script.answered(text).cloned();
        let (reply, round_trip_ms) = match answer.result {
            Ok(answer) => answer,
            Err(error) => {
                // The vehicle keeps its last directive. A model fault must
                // not become a flight command.
                tracing::error!(%error, "model fault; the directive does not change");
                let detail = error.to_string();
                return self
                    .record
                    .append(&Entry::ModelFault { at_s, text, detail })
                    .await;
            }
        };
        match self.flight.take_reply(text, &reply, at_s) {
            Ok(directive) => {
                tracing::info!(?directive, model_ms = reply.model_ms, "directive");
                let read_correctly = means.as_ref().map(|means| same(means, &directive));
                self.record
                    .append(&Entry::Message {
                        at_s,
                        text,
                        source: answer.ask.source,
                        reply: &reply,
                        round_trip_ms,
                        means: means.as_ref(),
                        read_correctly,
                    })
                    .await
            }
            Err(refusal) => {
                tracing::warn!(%refusal, "the reply is not flown");
                self.record
                    .append(&Entry::Refused {
                        at_s,
                        text,
                        reply: &reply,
                        reason: refusal.to_string(),
                        means: means.as_ref(),
                    })
                    .await
            }
        }
    }
}

/// True when two directives are the same instruction. The reason of `unable`
/// is free text and takes no part.
fn same(means: &Directive, got: &Directive) -> bool {
    match (means, got) {
        (Directive::Unable { .. }, Directive::Unable { .. }) => true,
        _ => means == got,
    }
}
