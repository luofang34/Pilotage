//! Standalone bench shell (ADR-0029): the third, deliberately unalike
//! shell. It composes the same registry the web shell consumes, runs
//! the admission harness, computes the cross-shell scene digest against
//! the pinned value, and rasterizes every panel × canonical state —
//! optionally writing PPM frames — with no host and no protocol. A
//! nonzero exit is a conformance failure, never a partial pass.

mod output;

use std::io::Write;

use output::print_line;
use std::path::PathBuf;

use pilotage_instrument_conformance::{AdmissionError, admit};
use pilotage_instrument_panels::{BUILTIN_PANELS, BUILTIN_SCENE_DIGEST};
use pilotage_instrument_raster::{FrameId, FramebufferDims, RenderStatus, render};
use pilotage_instrument_registry::{
    CANONICAL_STATES, EMPTY_CONFIG, PanelDrawError, Registry, RegistryError, scene_digest,
};
use pilotage_instrument_scene::{MAX_SCENE_BYTES, SceneWriter};
use pilotage_instrument_state::{FreshnessPolicy, resolve};

#[derive(Debug, thiserror::Error)]
enum BenchError {
    /// The shipped composition failed registry validation.
    #[error("registry composition refused: {0}")]
    Compose(#[from] RegistryError),
    /// The admission harness refused a panel.
    #[error("admission refused: {0}")]
    Admission(#[from] AdmissionError),
    /// This shell renders a different contract than the pin.
    #[error("scene digest {got} does not match the pinned {want}")]
    DigestMismatch {
        /// What this shell computed.
        got: String,
        /// The cross-shell pin.
        want: &'static str,
    },
    /// A panel refused to draw a corpus state.
    #[error("panel {panel} failed to draw {state}: {source}")]
    Draw {
        /// The refusing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The panel's reason.
        #[source]
        source: PanelDrawError,
    },
    /// The digest's scratch buffer cannot hold a scene.
    #[error("digest scratch buffer of {len} bytes is too small")]
    DigestScratch {
        /// The offending buffer length.
        len: usize,
    },
    /// The reference rasterizer refused a validated scene.
    #[error("raster failed for {panel} × {state}")]
    Raster {
        /// The panel whose scene failed.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
    },
    /// A PPM frame could not be written.
    #[error("writing {} failed", path.display())]
    Io {
        /// The destination that failed.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

fn main() -> Result<(), BenchError> {
    let out_dir = parse_out_dir();
    let registry = Registry::new(BUILTIN_PANELS)?;

    let report = admit(&registry)?;
    print_line(&format!(
        "admission: {} cases pass, {} counted warnings",
        report.cases,
        report.warnings.len()
    ));

    let mut scratch = vec![0u8; MAX_SCENE_BYTES];
    let digest = hex(
        scene_digest(&registry, &mut scratch).map_err(|error| match error {
            pilotage_instrument_registry::DigestError::Draw {
                panel,
                state,
                source,
            } => BenchError::Draw {
                panel,
                state,
                source,
            },
            pilotage_instrument_registry::DigestError::Scratch { len } => {
                BenchError::DigestScratch { len }
            }
        })?,
    );
    if digest != BUILTIN_SCENE_DIGEST {
        return Err(BenchError::DigestMismatch {
            got: digest,
            want: BUILTIN_SCENE_DIGEST,
        });
    }
    print_line(&format!("scene digest: {digest} (matches pin)"));

    for panel in registry.panels() {
        for state in CANONICAL_STATES {
            let frame = rasterize(panel, state.id, (state.build)(), &mut scratch)?;
            if let Some(dir) = &out_dir {
                write_ppm(dir, panel.id, state.id, panel.design_frame, &frame)?;
            }
        }
    }
    print_line(&format!(
        "rasterized {} panels x {} states",
        registry.panels().len(),
        CANONICAL_STATES.len()
    ));
    Ok(())
}

fn parse_out_dir() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn rasterize(
    panel: &'static pilotage_instrument_registry::PanelDescriptor,
    state_id: &'static str,
    state: pilotage_instrument_state::AircraftState,
    scratch: &mut [u8],
) -> Result<Vec<u8>, BenchError> {
    let data = resolve(&state, &FreshnessPolicy::default());
    let mut writer = SceneWriter::new(scratch).map_err(|_| BenchError::Raster {
        panel: panel.id,
        state: state_id,
    })?;
    (panel.draw)(&data, &EMPTY_CONFIG, None, &mut writer).map_err(|source| BenchError::Draw {
        panel: panel.id,
        state: state_id,
        source,
    })?;
    let used = writer.finish();
    let (w, h) = (
        panel.design_frame.width as u32,
        panel.design_frame.height as u32,
    );
    let mut framebuffer = vec![0u8; (w * h * 4) as usize];
    let report = render(
        scratch.get(..used).unwrap_or(&[]),
        &mut framebuffer,
        FramebufferDims::tight(w, h),
        FrameId::default(),
    )
    .map_err(|_| BenchError::Raster {
        panel: panel.id,
        state: state_id,
    })?;
    if report.status != RenderStatus::Painted {
        return Err(BenchError::Raster {
            panel: panel.id,
            state: state_id,
        });
    }
    Ok(framebuffer)
}

fn write_ppm(
    dir: &PathBuf,
    panel_id: &str,
    state_id: &str,
    frame: pilotage_instrument_registry::DesignFrame,
    rgba: &[u8],
) -> Result<(), BenchError> {
    let path = dir.join(format!("{panel_id}-{state_id}.ppm"));
    let io = |source| BenchError::Io {
        path: path.clone(),
        source,
    };
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut out = Vec::new();
    let (w, h) = (frame.width as usize, frame.height as usize);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for pixel in rgba.chunks_exact(4) {
        out.extend_from_slice(&pixel[..3]);
    }
    let mut file = std::fs::File::create(&path).map_err(io)?;
    file.write_all(&out).map_err(io)?;
    print_line(&format!("wrote {}", path.display()));
    Ok(())
}

fn hex(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
