//! Translation of engine module events into shell events.
//!
//! Split from the driver loop so each file stays about one thing: the
//! loop moves bytes; this file decides what the shell hears.

use pilotage_client_session::ModuleEvent;
use pilotage_control_web::AuthorityEvent;
use pilotage_instrument_feed::{FeedParams, InstrumentFeed};
use pilotage_protocol::wire;

use super::driver::Link;
use super::records::{LinkCatalog, LinkEvent};

impl Link {
    /// Translates one module event for the shell, feeding telemetry into
    /// the shared instrument feed on the way.
    /// One lease outcome: the runtime's authority mirror learns it
    /// first — a mirror that misses a grant gates the plan into the
    /// exact silence the host revokes — then the shell hears it, with
    /// the denial named for the operator.
    fn emit_lease(&mut self, response: &wire::LeaseResponse) {
        let scope = response
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let generation = response.generation.as_ref().map_or(0, |g| g.value);
        self.mirror_authority(
            &scope,
            if response.granted {
                AuthorityEvent::LeaseGranted { generation }
            } else {
                // Any not-granted lands the slot back at idle, so a
                // later grant (or the gimbal plan's own retry) stands.
                AuthorityEvent::Revoked { generation }
            },
        );
        if response.granted {
            self.control.begin_control_run();
        }
        self.delivery.event(LinkEvent::LeaseChanged {
            held: response.granted,
            scope,
            detail: if response.granted {
                String::new()
            } else {
                match response.reason {
                    1 => "another operator holds control".to_owned(),
                    2 => "the host does not publish this scope".to_owned(),
                    3 => "this principal is not authorized".to_owned(),
                    other => format!("denied ({other})"),
                }
            },
        });
    }

    /// A stale-generation or no-holder rejection is the host saying the
    /// fence moved; the mirror follows it exactly as the browser's does.
    fn emit_rejection(&mut self, rejected: &wire::FrameRejected) {
        self.stats.rejected = self.stats.rejected.wrapping_add(1);
        if rejected.reason == 1 || rejected.reason == 2 {
            let scope = rejected
                .scope
                .as_ref()
                .map(|s| s.value.clone())
                .unwrap_or_default();
            let generation = rejected.current_generation.as_ref().map_or(0, |g| g.value);
            self.mirror_authority(&scope, AuthorityEvent::Revoked { generation });
        }
        self.delivery.event(LinkEvent::ControlRejected {
            sequence: rejected.sequence.as_ref().map_or(0, |s| s.value),
            reason: rejected.reason,
        });
    }

    /// The action verdict feeds the runtime's mirror (arm state gates
    /// its plans) and then the shell.
    fn emit_action_result(&mut self, result: wire::ControlActionResult) {
        self.stats.action_results = self.stats.action_results.wrapping_add(1);
        let scope = result
            .scope
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        self.mirror_authority(
            &scope,
            AuthorityEvent::ActionResult {
                action: u32::try_from(result.action).unwrap_or(0),
                accepted: result.accepted,
            },
        );
        self.delivery.event(LinkEvent::ActionResult {
            action: result.action,
            accepted: result.accepted,
            detail: result.detail,
        });
    }

    pub(super) fn emit(&mut self, event: ModuleEvent) {
        match event {
            ModuleEvent::Admitted(admission) => {
                // A fresh admission is a fresh transport session for the
                // runtime's mirror too.
                self.control.begin_session();
                // The feed follows the first offered vehicle until a lease
                // narrows the interest; a multi-vehicle chooser is shell
                // work over this same catalog.
                if let Some(vehicle) = admission.vehicles.first() {
                    self.feed = Some(InstrumentFeed::new(&FeedParams {
                        vehicle_id: vehicle.vehicle_id,
                        sim_accept_unseen: true,
                    }));
                }
                self.delivery.event(LinkEvent::Admitted {
                    catalog: LinkCatalog::from_admission(&admission),
                });
            }
            ModuleEvent::Telemetry(sample) => {
                self.stats.telemetry = self.stats.telemetry.wrapping_add(1);
                let now_ms = self.now_ms();
                if let Some(feed) = self.feed.as_mut() {
                    #[allow(clippy::cast_precision_loss)]
                    feed.ingest(&sample, now_ms as f64);
                }
            }
            ModuleEvent::Lease(response) => self.emit_lease(&response),
            ModuleEvent::LeaseReleased(released) => {
                let scope = released
                    .scope
                    .as_ref()
                    .map(|s| s.value.clone())
                    .unwrap_or_default();
                self.delivery.event(LinkEvent::LeaseChanged {
                    held: false,
                    scope,
                    detail: "released".to_owned(),
                });
            }
            ModuleEvent::ControlRejected(rejected) => self.emit_rejection(&rejected),
            ModuleEvent::ConnectionDown { retry_at_ms } => {
                self.delivery.event(LinkEvent::Down { retry_at_ms });
            }
            ModuleEvent::ActionResult(result) => self.emit_action_result(result),
            ModuleEvent::VideoFrame(body) => {
                // Structural decode only; a body that does not parse is
                // dropped and the next one stands alone.
                if let Ok(frame) = pilotage_protocol::video_frame::decode_v2(&body) {
                    let codec = String::from_utf8_lossy(&frame.codec).into_owned();
                    self.delivery
                        .video(frame.header.source_id, codec, frame.payload.to_vec());
                }
            }
            ModuleEvent::Authority(event) => {
                if let Some(wire::authority_event::Event::ScopeTransferRequested(requested)) =
                    event.event
                    && self.engine.holds_control()
                {
                    self.delivery.event(LinkEvent::TakeoverAsked {
                        from_principal: requested.from_principal.map_or(0, |p| p.value),
                        scope: requested.scope.map(|s| s.value).unwrap_or_default(),
                    });
                }
            }
            ModuleEvent::VideoStreamCorrupt { claimed_bytes } => {
                self.delivery.event(LinkEvent::Notice {
                    text: format!(
                        "a video stream went bad (claimed {claimed_bytes} bytes) and was \
                         stopped; its picture resumes when the host cycles it"
                    ),
                });
            }
            ModuleEvent::Pong(_) => {}
        }
    }
}
