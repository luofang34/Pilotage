//! Fixed limits for trial data.

/// The supported trial manifest schema version.
pub const TRIAL_MANIFEST_SCHEMA_VERSION: u16 = 1;
/// The supported scenario schema version.
pub const SCENARIO_SCHEMA_VERSION: u16 = 1;
/// The supported backend capabilities schema version.
pub const BACKEND_CAPABILITIES_SCHEMA_VERSION: u16 = 1;
/// The supported trial sample schema version.
pub const TRIAL_SAMPLE_SCHEMA_VERSION: u16 = 1;

/// The maximum trial manifest JSON size.
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
/// The maximum trial sample JSON size.
pub const MAX_SAMPLE_BYTES: usize = 64 * 1024;
/// The maximum UTF-8 byte count for one text value.
pub const MAX_TEXT_BYTES: usize = 256;
/// The maximum number of phases in one scenario.
pub const MAX_PHASES: usize = 128;
/// The maximum number of conditions in one phase condition list.
pub const MAX_PHASE_CONDITIONS: usize = 32;
/// The maximum number of capabilities in one list.
pub const MAX_CAPABILITIES: usize = 32;
/// The maximum number of clock mappings in one run identity.
pub const MAX_CLOCK_MAPPINGS: usize = 16;
/// The maximum number of components in one multisine waveform.
pub const MAX_WAVE_COMPONENTS: usize = 32;
/// The maximum number of axes in one raw input sample.
pub const MAX_RAW_AXES: usize = 32;
/// The maximum number of buttons in one raw input sample.
pub const MAX_RAW_BUTTONS: usize = 128;
/// The maximum number of actuator values in one sample.
pub const MAX_ACTUATOR_VALUES: usize = 64;
/// The maximum number of named condition values in one sample.
pub const MAX_CONDITION_VALUES: usize = 64;
