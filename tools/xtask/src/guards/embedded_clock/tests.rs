use super::{judge, reads};

#[test]
fn a_clock_read_is_found_and_a_comment_is_not() {
    let source = "let a = Instant::now();\n// Instant::now() in a comment\nlet b = at.elapsed();\nlet c = self.elapsed();\nuse tokio::time::sleep;\n";
    let found: Vec<usize> = reads(source).into_iter().map(|(line, _)| line).collect();
    assert_eq!(found, [1, 3, 5]);
}

#[test]
fn a_read_in_an_unlisted_file_is_refused() {
    let found = reads("let now = SystemTime::now();\n");
    assert_eq!(
        judge("crates/pilotage-host-core/src/core.rs", &found).len(),
        1
    );
}

#[test]
fn a_listed_file_passes_only_at_its_count() {
    let one = reads("let now = Instant::now();\n");
    assert!(judge("adapters/aviate/src/clock.rs", &one).is_empty());
    let two = reads("let a = Instant::now();\nlet b = Instant::now();\n");
    assert_eq!(judge("adapters/aviate/src/clock.rs", &two).len(), 1);
}
