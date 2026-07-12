#![allow(clippy::expect_used, clippy::panic)]

//! CI gate on the deterministic work counters: the demo panel fixtures must
//! fit inside [`RenderWork::BUDGET`], and the counters must be a pure
//! function of scene bytes and dimensions.

use pilotage_instrument_panels::{PANEL_H, PANEL_W, PfdConfig, draw_hsi, draw_pfd};
use pilotage_instrument_state::{FreshnessPolicy, resolve};
use std::vec::Vec;

use super::frame_hashes::{demo_state, encode};
use crate::{FrameId, FramebufferDims, RenderWork, render};

fn work_for(scene: &[u8]) -> RenderWork {
    let (w, h) = (PANEL_W as u32, PANEL_H as u32);
    let mut fb = std::vec![0u8; (w * h * 4) as usize];
    let report = render(
        scene,
        &mut fb,
        FramebufferDims::tight(w, h),
        FrameId::default(),
    )
    .expect("panel scene renders");
    report.work
}

fn panel_scenes() -> Vec<(&'static str, Vec<u8>)> {
    let data = resolve(&demo_state(), &FreshnessPolicy::default());
    std::vec![
        (
            "PFD",
            encode(|w| draw_pfd(&data, &PfdConfig::default(), None, w).expect("pfd")),
        ),
        ("HSI", encode(|w| draw_hsi(&data, None, w).expect("hsi"))),
    ]
}

#[test]
fn panel_fixtures_fit_within_work_budget() {
    for (name, scene) in panel_scenes() {
        let work = work_for(&scene);
        assert!(
            work.within(&RenderWork::BUDGET),
            "{name} exceeds the engineering work budget: \
             {work:?} vs {:?} — either the scene grew pathologically or a \
             primitive's bounded region loop regressed; investigate before \
             raising the budget",
            RenderWork::BUDGET,
        );
    }
}

#[test]
fn work_counters_are_deterministic_and_nonzero() {
    for (name, scene) in panel_scenes() {
        let first = work_for(&scene);
        let second = work_for(&scene);
        assert_eq!(first, second, "{name} work counters are reproducible");
        assert!(
            first.coverage_samples > 0 && first.composites > 0,
            "{name} paints content, so both counters must advance"
        );
        assert!(
            first.composites <= first.coverage_samples,
            "{name}: every composite follows a coverage sample"
        );
    }
}
