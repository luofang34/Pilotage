//! Formats the end-of-run measurement summary block (this binary's actual
//! product output, printed via `output::print_line`).

use std::time::Duration;

use crate::metrics::RunMetrics;
use crate::output::print_line;
use crate::video::VideoStats;

/// Prints the full end-of-run summary: control-to-telemetry latency
/// percentiles, Ping/Pong RTT, frame counters, telemetry received, video
/// downlink statistics, and the outcome of the deliberate stale-generation
/// fencing probe.
pub fn print_summary(
    metrics: &RunMetrics,
    video: &VideoStats,
    elapsed: Duration,
    fencing_confirmed: bool,
) {
    print_line("=== loopback-probe summary ===");
    print_control_latency(metrics);
    print_rtt(metrics);
    print_line(&format!("frames sent:        {}", metrics.frames_sent));
    print_line(&format!(
        "frames accepted:     {}",
        metrics.frames_accepted()
    ));
    print_line(&format!("frames rejected:     {}", metrics.frames_rejected));
    print_line(&format!(
        "telemetry received:  {}",
        metrics.telemetry_received
    ));
    print_line(&format!(
        "events dropped (channel full): {}",
        metrics.dropped_events
    ));
    print_final_pose(metrics);
    print_video(video, elapsed);
    print_line(&format!(
        "end-to-end fencing:  {}",
        if fencing_confirmed {
            "CONFIRMED (stale-generation frame was rejected)"
        } else {
            "NOT CONFIRMED (no FrameRejected observed for the stale probe frame)"
        }
    ));
}

/// Prints the vehicle's final observed pose, or a placeholder if no
/// telemetry arrived.
fn print_final_pose(metrics: &RunMetrics) {
    match metrics.last_pose {
        Some((x, y, heading)) => print_line(&format!(
            "final pose:          x={x:.3}m y={y:.3}m heading={heading:.3}rad"
        )),
        None => print_line("final pose:          no telemetry observed"),
    }
}

/// Prints the video downlink section: frames received/decoded/failed,
/// average fps, decoded dimensions, and inter-arrival latency percentiles.
fn print_video(video: &VideoStats, elapsed: Duration) {
    print_line(&format!(
        "video frames received: {} (decoded {}, decode failed {})",
        video.frames_received, video.frames_decoded, video.frames_decode_failed
    ));
    match video.avg_fps(elapsed) {
        Some(fps) => print_line(&format!("video avg fps:       {fps:.2}")),
        None => print_line("video avg fps:       no frames decoded"),
    }
    match video.last_dims {
        Some((w, h)) => print_line(&format!("video dimensions:    {w}x{h}")),
        None => print_line("video dimensions:    n/a"),
    }
    match video.inter_arrival.percentiles() {
        Some((p50, p95, max)) => print_line(&format!(
            "video inter-arrival: p50={} p95={} max={}",
            fmt_duration(p50),
            fmt_duration(p95),
            fmt_duration(max)
        )),
        None => print_line("video inter-arrival: no samples observed"),
    }
}

fn print_control_latency(metrics: &RunMetrics) {
    match metrics.control_to_telemetry.percentiles() {
        Some((p50, p95, max)) => {
            print_line(&format!(
                "control->telemetry latency: p50={} p95={} max={} (n={}, dropped={}, \
                 backlog_dropped={})",
                fmt_duration(p50),
                fmt_duration(p95),
                fmt_duration(max),
                metrics.control_to_telemetry.len(),
                metrics.control_to_telemetry.dropped(),
                metrics.control_to_telemetry_backlog_dropped
            ));
        }
        None => print_line("control->telemetry latency: no samples observed"),
    }
    print_line(
        "  (loopback, same-clock: measures software + local-transport latency only, not \
         cross-host wire time — see ADR-0009)",
    );
}

fn print_rtt(metrics: &RunMetrics) {
    match metrics.rtt.rtt() {
        Some(rtt) => print_line(&format!("ping/pong RTT (EWMA): {}", fmt_duration(rtt))),
        None => print_line("ping/pong RTT: no samples observed"),
    }
}

/// Formats a duration with millisecond precision, since this tool's targets
/// (ADR-0009) are all in the single-to-low-hundreds-of-milliseconds range.
fn fmt_duration(duration: Duration) -> String {
    format!("{:.3}ms", duration.as_secs_f64() * 1000.0)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::fmt_duration;
    use std::time::Duration;

    #[test]
    fn formats_milliseconds_with_precision() {
        assert_eq!(fmt_duration(Duration::from_micros(1500)), "1.500ms");
    }
}
