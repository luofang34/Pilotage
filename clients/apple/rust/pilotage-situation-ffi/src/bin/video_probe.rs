//! Loopback video-throughput probe: connects the SAME driver the iPad
//! runs to a local session and reports what actually arrives, so a
//! host-side "client writer still busy" can be attributed to the
//! client's transport rather than to any radio.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::print_stdout)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pilotage_situation_ffi::{LinkConfig, LinkEvent, LinkObserver, LinkSession};

#[derive(Default)]
struct Counter {
    frames: AtomicU64,
    bytes: AtomicU64,
    events: AtomicU64,
    states: AtomicU64,
}

impl LinkObserver for Counter {
    fn on_event(&self, event: LinkEvent) {
        self.events.fetch_add(1, Ordering::Relaxed);
        if let LinkEvent::Admitted { .. } = event {
            println!("admitted");
        }
    }
    fn on_state_frame(&self, _frame: Vec<u8>, _at: u64) {
        self.states.fetch_add(1, Ordering::Relaxed);
    }
    fn on_video_frame(&self, source_id: u8, codec: String, payload: Vec<u8>) {
        let n = self.frames.fetch_add(1, Ordering::Relaxed) + 1;
        self.bytes.fetch_add(payload.len() as u64, Ordering::Relaxed);
        if n <= 3 || n % 100 == 0 {
            println!("frame {n}: source={source_id} codec={codec} bytes={}", payload.len());
        }
    }
}

fn main() {
    let observer = Arc::new(Counter::default());
    let session = LinkSession::connect(
        LinkConfig {
            url: "https://127.0.0.1:4433/pilotage".to_owned(),
            certificate_sha256_hex: String::new(),
            client_name: "video-probe".to_owned(),
        },
        observer.clone(),
    )
    .expect("connect");
    std::thread::sleep(std::time::Duration::from_secs(15));
    println!(
        "15s totals: frames={} bytes={} states={} events={}",
        observer.frames.load(Ordering::Relaxed),
        observer.bytes.load(Ordering::Relaxed),
        observer.states.load(Ordering::Relaxed),
        observer.events.load(Ordering::Relaxed),
    );
    session.shutdown();
}
