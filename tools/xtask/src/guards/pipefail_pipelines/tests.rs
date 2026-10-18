#![allow(clippy::expect_used, clippy::panic)]

use super::{check, findings_in};

fn lines(path: &str, text: &str) -> Vec<usize> {
    findings_in(path, text)
        .iter()
        .map(|finding| finding.line)
        .collect()
}

/// The pipeline that failed CI at random: the reader exits on its
/// first match while the writer still has output to write.
#[test]
fn early_exiting_readers_are_refused_under_pipefail() {
    let script = "#!/usr/bin/env bash\nset -euo pipefail\n\
        grep -E '^run:' ci.yml | grep -Fq \"$step\"\n\
        git ls-files | grep -qi Style\n\
        grep -A 6 x f | grep -qxF y\n\
        find . -name '*.json' | sort | head -1\n\
        cat f | sed -n '1p;q'\n\
        out=\"$(grep -A 8 x f | head -8)\"\n\
        grep x f | grep -m1 y\n\
        grep x f | grep --quiet y\n\
        grep x f | LC_ALL=C grep -l y\n";
    assert_eq!(lines("s.sh", script), vec![3, 4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn a_reader_continued_on_the_next_line_is_refused() {
    let script = "set -o pipefail\nif ! grep -A 4 x f \\\n    | grep -Eq 'a|b'; then :; fi\n\
        grep x f |\n    grep -q y\n";
    assert_eq!(lines("s.sh", script), vec![3, 5]);
}

/// Every spelling of the option arms the guard; without it, the
/// reader's own status decides the pipeline and nothing is refused.
#[test]
fn only_scripts_that_enable_pipefail_are_checked() {
    let pipeline = "grep x f | grep -q y\n";
    for header in [
        "set -euo pipefail\n",
        "set -e -o pipefail\n",
        "set -o pipefail\n",
    ] {
        assert_eq!(lines("s.sh", &format!("{header}{pipeline}")), vec![2]);
    }
    for header in ["set -eu\n", "# set -euo pipefail\n", "echo pipefail\n"] {
        assert!(lines("s.sh", &format!("{header}{pipeline}")).is_empty());
    }
}

/// The repaired forms read all of their input.
#[test]
fn readers_that_drain_their_input_pass() {
    let script = "set -euo pipefail\n\
        run_lines=$(grep -E '^run:' ci.yml || true)\n\
        grep -Fq \"$step\" <<<\"$run_lines\"\n\
        git ls-files | grep -i Style >/dev/null\n\
        grep -A 8 x f | sed -n 1,8p\n\
        grep -c x f | grep -E '^[0-9]+$' || true\n\
        grep -Eq 'a|b' f || echo 'no | grep -q here'\n\
        cmd >| out\n\
        # grep x f | grep -q y\n";
    assert!(lines("s.sh", script).is_empty(), "{script}");
}

/// A here-document body is data, even when it spells a pipeline.
#[test]
fn heredoc_bodies_are_not_code() {
    let script = "set -euo pipefail\ncat >\"$fixture\" <<'EOF'\ngrep x f | grep -q y\nEOF\n\
        cat <<-EOF\n\tgrep x f | head -1\n\tEOF\ngrep x f | head -1\n";
    assert_eq!(lines("s.sh", script), vec![8]);
}

#[test]
fn the_finding_names_the_script_line_and_reader() {
    let findings = findings_in(
        "scripts/check-x.sh",
        "set -o pipefail\na | grep -Fq -- '-- guards'\n",
    );
    let message = findings.first().expect("one finding").to_string();
    assert!(
        message.contains("scripts/check-x.sh:2: `| grep -Fq -- -- guards`"),
        "{message}"
    );
}

/// The repository itself holds no early-exiting reader under pipefail.
#[test]
fn repository_scripts_have_no_early_exiting_readers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let findings = check(&root).expect("scan tracked scripts");
    let report: Vec<String> = findings.iter().map(ToString::to_string).collect();
    assert!(findings.is_empty(), "{}", report.join("\n"));
}
