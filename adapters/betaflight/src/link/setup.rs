//! The first-connect check and configuration of the flight controller.
//!
//! What can change at run time is configured here, idempotently: the aux
//! mode ranges for ARM and ANGLE are compared slot by slot and written
//! (then saved) only where they differ. Writes persist on the flight
//! controller, so they happen only under [`BetaflightProfile::Simulation`],
//! with the [`SimulationWrites`] proof no other profile yields; this
//! module is the only place an MSP configuration write can come from. On
//! a physical vehicle a differing range is a [`ConfigIssue::ModeRange`]
//! that blocks arming and names the slot and the range Pilotage needs. What takes effect only at boot is
//! checked, never written: the receiver provider is chosen in `rxInit`,
//! and a reboot of Betaflight SITL ends its process, so an adapter that
//! wrote it would kill the vehicle it configures. A wrong receiver is a
//! typed [`ConfigIssue`] that blocks arming and names the fix.

use pilotage_msp::message::feature;
use pilotage_msp::{
    ApiVersion, BoxIds, FailsafeConfig, ModeRange, command, decode_features, encode_set_mode_range,
};
use tokio::io::{AsyncRead, AsyncWrite};

use super::LinkError;
use super::port::MspPort;
use crate::config::BetaflightProfile;
use crate::rc::required_mode_ranges;

/// The proof that configuration writes are permitted: constructible only
/// from [`BetaflightProfile::Simulation`], inside this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SimulationWrites(());

impl SimulationWrites {
    fn for_profile(profile: BetaflightProfile) -> Option<Self> {
        match profile {
            BetaflightProfile::Simulation => Some(Self(())),
            BetaflightProfile::Physical => None,
        }
    }
}

/// A configuration fact that keeps the adapter from flying the vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConfigIssue {
    /// The MSP receiver is off, or the UDP receiver (which Betaflight
    /// prefers) is on. Both take effect only at boot.
    #[error(
        "the receiver is not MSP (feature mask {features:#010x}); enable RX_MSP, disable RX_UDP, and reboot (sim/betaflight/sitl.cli does both)"
    )]
    ReceiverNotMsp {
        /// The enabled feature mask.
        features: u32,
    },
    /// The firmware does not list a box the channel layout drives.
    #[error("the firmware has no {name} mode")]
    MissingMode {
        /// The mode's name.
        name: &'static str,
    },
    /// A physical flight controller's aux mode range differs from the
    /// channel layout the adapter drives; it is never written.
    #[error(
        "Betaflight aux mode range {slot} is {found:?}; Pilotage needs {wanted:?} (set it on the vehicle: the adapter never writes a physical vehicle's configuration)"
    )]
    ModeRange {
        /// The mode-range slot.
        slot: usize,
        /// The range the channel layout needs.
        wanted: ModeRange,
        /// The range the flight controller reported, if the slot exists.
        found: Option<ModeRange>,
    },
}

/// What the first connect learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The box order the status mode bits use.
    pub(crate) boxes: BoxIds,
    /// What blocks flight, if anything.
    pub(crate) issue: Option<ConfigIssue>,
}

/// Reads the configuration, writes the aux ranges where they differ
/// (simulation only), and reports what blocks flight.
pub(crate) async fn configure<S: AsyncRead + AsyncWrite + Unpin>(
    port: &mut MspPort<S>,
    profile: BetaflightProfile,
) -> Result<Setup, LinkError> {
    let api = ApiVersion::decode(&port.request(command::MSP_API_VERSION, vec![]).await?)?;
    let boxes = BoxIds::decode(&port.request(command::MSP_BOXIDS, vec![]).await?);
    let features = decode_features(&port.request(command::MSP_FEATURE_CONFIG, vec![]).await?)?;
    let failsafe =
        FailsafeConfig::decode(&port.request(command::MSP_FAILSAFE_CONFIG, vec![]).await?)?;
    let ranges = ModeRange::decode_all(&port.request(command::MSP_MODE_RANGES, vec![]).await?);
    tracing::info!(
        api = format_args!("{}.{}", api.major, api.minor),
        features = format_args!("{features:#010x}"),
        failsafe = ?failsafe.procedure,
        failsafe_delay_s = f64::from(failsafe.delay_ds) / 10.0,
        "Betaflight connected"
    );
    let issue = receiver_issue(features).or_else(|| missing_mode(&boxes));
    let Some(writes) = SimulationWrites::for_profile(profile) else {
        return Ok(Setup {
            boxes,
            issue: issue.or_else(|| mode_range_issue(&ranges)),
        });
    };
    write_mode_ranges(port, &ranges, writes).await?;
    Ok(Setup { boxes, issue })
}

/// The first aux range that differs from the channel layout.
fn mode_range_issue(ranges: &[ModeRange]) -> Option<ConfigIssue> {
    required_mode_ranges()
        .into_iter()
        .enumerate()
        .find(|(slot, wanted)| ranges.get(*slot) != Some(wanted))
        .map(|(slot, wanted)| ConfigIssue::ModeRange {
            slot,
            wanted,
            found: ranges.get(slot).copied(),
        })
}

/// Writes every differing aux range and saves them.
async fn write_mode_ranges<S: AsyncRead + AsyncWrite + Unpin>(
    port: &mut MspPort<S>,
    ranges: &[ModeRange],
    SimulationWrites(()): SimulationWrites,
) -> Result<(), LinkError> {
    let mut changed = false;
    for (slot, wanted) in required_mode_ranges().into_iter().enumerate() {
        if ranges.get(slot) == Some(&wanted) {
            continue;
        }
        let index = u8::try_from(slot).unwrap_or(u8::MAX);
        let payload = encode_set_mode_range(index, wanted)?;
        port.request(command::MSP_SET_MODE_RANGE, payload).await?;
        tracing::info!(slot, ?wanted, "wrote a Betaflight aux mode range");
        changed = true;
    }
    if changed {
        port.request(command::MSP_EEPROM_WRITE, vec![]).await?;
        tracing::info!("saved the Betaflight aux mode ranges");
    }
    Ok(())
}

/// The receiver check: MSP on, UDP off.
pub(crate) fn receiver_issue(features: u32) -> Option<ConfigIssue> {
    let msp = features & feature::RX_MSP != 0;
    let udp = features & feature::RX_UDP != 0;
    (!msp || udp).then_some(ConfigIssue::ReceiverNotMsp { features })
}

fn missing_mode(boxes: &BoxIds) -> Option<ConfigIssue> {
    use pilotage_msp::message::permanent_id;
    [(permanent_id::ARM, "ARM"), (permanent_id::ANGLE, "ANGLE")]
        .into_iter()
        .find(|(id, _)| !boxes.0.contains(id))
        .map(|(_, name)| ConfigIssue::MissingMode { name })
}
