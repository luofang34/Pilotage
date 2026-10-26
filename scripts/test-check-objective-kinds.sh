#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/crates/pilotage-mission/src/lane/compile" \
    "$fixture/crates/pilotage-coordination/src/tests"
cp "$root_dir/scripts/check-objective-kinds.sh" "$fixture/scripts/"
git -C "$fixture" init -q

# Planning names every kind; other code names only the survey.
printf '%s\n' \
    'match kind { Kind::Survey(s) => survey(s), Kind::Named(n) => named(n) }' \
    > "$fixture/crates/pilotage-mission/src/lane/compile.rs"
printf '%s\n' \
    'match kind { Kind::Survey(s) => Some(s), _ => None }' \
    > "$fixture/crates/pilotage-coordination/src/team.rs"
printf '%s\n' \
    'let objective = Kind::Named(report);' \
    > "$fixture/crates/pilotage-coordination/src/tests/whole.rs"
bash "$fixture/scripts/check-objective-kinds.sh" "$fixture" >/dev/null

# A match arm outside planning that names another kind is refused.
printf '%s\n' \
    'match kind { Kind::Survey(s) => Some(s), Kind::Named(_) => None }' \
    > "$fixture/crates/pilotage-coordination/src/team.rs"
output="$fixture/failure.txt"
if bash "$fixture/scripts/check-objective-kinds.sh" "$fixture" >"$output" 2>&1; then
    echo "the objective kinds guard accepted a kind named outside planning" >&2
    exit 1
fi
if ! grep -Fq 'crates/pilotage-coordination/src/team.rs:1:' "$output"; then
    echo "the objective kinds guard did not name the file and line" >&2
    exit 1
fi

# A name that only ends in Kind, such as TaskKind, is another enum.
printf '%s\n' \
    'let kind = TaskKind::Operation;' \
    > "$fixture/crates/pilotage-coordination/src/team.rs"
bash "$fixture/scripts/check-objective-kinds.sh" "$fixture" >/dev/null

# Planning's own kinds' modules may name any kind.
printf '%s\n' \
    'pub fn compile(check: Kind::Named) {}' \
    > "$fixture/crates/pilotage-mission/src/lane/compile/check.rs"
bash "$fixture/scripts/check-objective-kinds.sh" "$fixture" >/dev/null

# An import by another name or by glob is refused.
for import in \
    'use wire::mission_objective::Kind as ObjectiveKind;' \
    'use wire::mission_objective::Kind::*;' \
    'use wire::mission_objective::Kind::{Survey, Named};'; do
    printf '%s\n' "$import" > "$fixture/crates/pilotage-coordination/src/team.rs"
    if bash "$fixture/scripts/check-objective-kinds.sh" "$fixture" >"$output" 2>&1; then
        echo "the objective kinds guard accepted: $import" >&2
        exit 1
    fi
done

echo "test-check-objective-kinds: OK"
