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
