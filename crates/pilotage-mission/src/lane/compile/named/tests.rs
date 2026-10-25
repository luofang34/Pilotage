use super::KINDS;

#[test]
fn each_kind_is_registered_once() {
    let mut names: Vec<&str> = KINDS.iter().map(|k| k.kind).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "a kind name is registered twice");
}
