//! The first-connect check and configuration of the flight controller.
//!
//! What can change at run time is configured here, idempotently: the aux
//! mode ranges for ARM and ANGLE are compared slot by slot and written
//! (then saved) only where they differ. What takes effect only at boot is
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
use crate::rc::required_mode_ranges;

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
}

/// What the first connect learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The box order the status mode bits use.
    pub(crate) boxes: BoxIds,
    /// What blocks flight, if anything.
    pub(crate) issue: Option<ConfigIssue>,
}

/// Reads the configuration, writes the aux ranges where they differ, and
/// reports what blocks flight.
pub(crate) async fn configure<S: AsyncRead + AsyncWrite + Unpin>(
    port: &mut MspPort<S>,
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
    Ok(Setup { boxes, issue })
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
