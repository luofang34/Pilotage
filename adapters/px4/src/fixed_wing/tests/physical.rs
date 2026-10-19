//! The fixed-wing uplink on a physical vehicle: the required parameters
//! are read, never written, and a differing value refuses arming by name.

use pilotage_mavlink::codec::{ParamValue, encode_param_value};

use super::{ArmRefusal, Harness, PINNED, ParamRefusal, SYSTEM, Sent};
use crate::config::Px4Profile;
use std::time::Duration;

#[test]
fn a_physical_vehicle_has_its_parameters_read_and_never_written() {
    let mut harness = Harness::with_profile(Px4Profile::Physical);
    harness.uplink.maintain(Some(0.0));
    let sent = harness.sent();
    let read: Vec<&str> = sent
        .iter()
        .filter_map(|frame| match frame {
            Sent::ParamRead(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    let pinned: Vec<&str> = PINNED.iter().map(|(name, _)| *name).collect();
    assert_eq!(read, pinned);
    assert!(
        !sent.iter().any(|frame| matches!(frame, Sent::ParamSet(_))),
        "a physical vehicle got a parameter write: {sent:?}"
    );

    // The vehicle keeps its RC transmitter (COM_RC_IN_MODE 0); the rest
    // match. Arming is refused naming the parameter and the needed value.
    let mut echoes: Vec<[u8; 37]> = PINNED
        .iter()
        .skip(1)
        .map(|(name, value)| encode_param_value(0, name, *value, SYSTEM, 1))
        .collect();
    echoes.push(encode_param_value(
        0,
        "COM_RC_IN_MODE",
        ParamValue::Int32(0),
        SYSTEM,
        1,
    ));
    let frames: Vec<&[u8]> = echoes.iter().map(|echo| echo.as_slice()).collect();
    harness.deliver(&frames, Some(0.0));
    let refusal = harness.uplink.begin_arm(0.0);
    assert_eq!(
        refusal,
        Err(ArmRefusal::Parameter(ParamRefusal::Mismatch {
            name: "COM_RC_IN_MODE",
            expected: ParamValue::Int32(1),
            found: ParamValue::Int32(0),
        }))
    );
    let Err(ArmRefusal::Parameter(refusal)) = refusal else {
        panic!("refused");
    };
    let detail = refusal.to_string();
    assert!(
        detail.contains("COM_RC_IN_MODE is 0") && detail.contains("needs 1"),
        "{detail}"
    );

    // Retries read the differing parameter again; still no write.
    harness.tick(Duration::from_secs(1), Some(0.0));
    let resent = harness.sent();
    assert_eq!(
        resent
            .iter()
            .filter(|frame| matches!(frame, Sent::ParamRead(_) | Sent::ParamSet(_)))
            .collect::<Vec<_>>(),
        vec![&Sent::ParamRead("COM_RC_IN_MODE".to_owned())]
    );

    // Once the operator sets it on the vehicle, arming proceeds.
    let echo = encode_param_value(0, "COM_RC_IN_MODE", ParamValue::Int32(1), SYSTEM, 1);
    harness.deliver(&[&echo], Some(0.0));
    assert_eq!(harness.uplink.begin_arm(0.0), Ok(()));
}
