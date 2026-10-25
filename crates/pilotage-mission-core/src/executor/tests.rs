use std::collections::BTreeMap;

use super::*;

const PERSON: ExecutorRegistry = ExecutorRegistry::new(&[WAIT_FOR_PERSON]);

fn confirm(parameters: &[(&str, &str)], version: u32) -> BoundAction {
    BoundAction {
        executor: "person".to_owned(),
        operation: "confirm".to_owned(),
        version,
        parameters: parameters
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[test]
fn a_registered_operation_with_typed_parameters_is_admitted() {
    let action = confirm(&[("question", "is the area clear"), ("timeout_s", "60")], 1);
    assert_eq!(PERSON.admit(&action).map(|d| d.version), Ok(1));
}

#[test]
fn an_unregistered_operation_or_another_version_is_refused() {
    let action = confirm(&[("question", "is the area clear")], 1);
    assert!(matches!(
        ExecutorRegistry::NONE.admit(&action),
        Err(BoundRefused::Unregistered { .. })
    ));
    assert!(matches!(
        PERSON.admit(&confirm(&[("question", "q")], 2)),
        Err(BoundRefused::Version {
            asked: 2,
            registered: 1,
            ..
        })
    ));
}

#[test]
fn missing_unknown_or_ill_typed_parameters_are_refused() {
    assert!(matches!(
        PERSON.admit(&confirm(&[], 1)),
        Err(BoundRefused::Missing {
            name: "question",
            ..
        })
    ));
    assert!(matches!(
        PERSON.admit(&confirm(&[("question", "q"), ("colour", "red")], 1)),
        Err(BoundRefused::Unknown { .. })
    ));
    assert!(matches!(
        PERSON.admit(&confirm(&[("question", "q"), ("timeout_s", "soon")], 1)),
        Err(BoundRefused::Type {
            name: "timeout_s",
            ..
        })
    ));
    assert!(matches!(
        PERSON.admit(&confirm(&[("question", "q"), ("timeout_s", "0")], 1)),
        Err(BoundRefused::Type {
            name: "timeout_s",
            ..
        })
    ));
}

#[test]
fn a_rule_holds_a_value_to_its_limit() {
    use super::rule::at_most;
    assert_eq!(
        at_most("21.5 C", "30"),
        Ok((true, "21.5 is within the limit of 30".to_owned()))
    );
    assert_eq!(at_most("21.5C", " 30 ").map(|(w, _)| w), Ok(true));
    assert_eq!(at_most("21.5°C", "30").map(|(w, _)| w), Ok(true));
    assert_eq!(at_most("30", "30").map(|(w, _)| w), Ok(true));
    assert_eq!(at_most("31.2 C", "30").map(|(w, _)| w), Ok(false));
    assert_eq!(at_most("-4 C", "-5").map(|(w, _)| w), Ok(false));
    assert!(at_most("warm", "30").is_err());
    assert!(at_most("21.5", "thirty").is_err());
    assert!(at_most("NaN", "30").is_err());
    assert!(at_most("1e400", "30").is_err());
    // The verdict names the number, so it stays short whatever the value.
    let long = format!("21.5 {}", "C".repeat(1000));
    let (_, verdict) = at_most(&long, "30").expect("a verdict");
    assert!(verdict.len() < 64, "{verdict}");
}
