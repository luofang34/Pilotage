//! Stable render-status reason codes shared with the JS backend (DISP-01).

/// Stable status field carried by the packed WASM render result.
///
/// Codes are append-only ABI: values are never reused or renumbered. The
/// JS mirror in `clients/web/instrument-health.js` maps each code to the
/// `D-<code>` diagnostic shown on the failure page; codes 100 and above
/// are reserved for failures only the JS backend can observe (module
/// load, ABI mismatch, paint, liveness).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderStatus {
    /// The scene rendered, self-validated, and its generation advanced.
    Ok = 0,
    /// `init` has not succeeded; there is no context to render into.
    NotInitialized = 1,
    /// The runtime boundary could not provide exclusive access. The explicit
    /// wasm-bindgen resource does not emit this during ordinary calls; the
    /// code remains reserved for ABI stability and alternate hosts.
    ContextUnavailable = 2,
    /// The state block is shorter than the ABI requires.
    StateTruncated = 3,
    /// The state block's version is not one this build decodes.
    StateBadVersion = 4,
    /// The panel id is not one this build draws.
    InvalidPanel = 5,
    /// The scene buffer cannot hold the panel's command stream.
    SceneBufferFull = 6,
    /// A draw call exceeded a per-command encoding limit.
    SceneCommandLimit = 7,
    /// The encoded scene failed structural self-validation.
    SceneStructure = 8,
}
