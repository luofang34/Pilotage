//! Native and browser transports use the same terminal admission close code.

use pilotage_protocol::ADMISSION_REJECTED_CLOSE_CODE;

#[test]
fn browser_fixture_matches_the_host_admission_close_code() -> Result<(), serde_json::Error> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../clients/web/session-admission-fixture.json"
    ))?;
    assert_eq!(
        fixture
            .get("rejectedCloseCode")
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(ADMISSION_REJECTED_CLOSE_CODE))
    );
    Ok(())
}
