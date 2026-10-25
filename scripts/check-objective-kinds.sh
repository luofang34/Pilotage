#!/usr/bin/env bash
# Only planning names a kind of objective other than the survey.
#
# Lifecycle, authorization, coordination, recovery and the clients treat
# every kind that is not a survey the same: it flies nowhere, and a team
# gives it whole to one member. A new kind then changes only its planning,
# its executor and the evaluation of its result. A match arm or a test of
# kind that names another kind outside the planning modules would make the
# next kind edit that code too, so this guard refuses it.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

# The planning dispatch and its kinds' modules, and the client that makes a
# report objective for its operator, name kinds by design.
allowed() {
    case "$1" in
        crates/pilotage-mission/src/lane/compile.rs) return 0 ;;
        crates/pilotage-mission/src/lane/compile/*) return 0 ;;
        crates/pilotage-client-session/src/mission/desk/answer.rs) return 0 ;;
    esac
    return 1
}

# Listed first, so that a listing that fails stops the guard.
files="$(git -C "$root_dir" ls-files --cached --others --exclude-standard -- '*.rs')"

status=0
while IFS= read -r file; do
    case "$file" in
        ""|*/tests/*|*/tests.rs|*/test.rs|*/*_tests.rs) continue ;;
    esac
    if allowed "$file" || [ ! -f "$root_dir/$file" ]; then
        continue
    fi
    # An import by another name, by glob or by group would name kinds
    # out of this guard's sight.
    while IFS= read -r hit; do
        echo "FORBIDDEN: $file:${hit%%:*}: imports the kinds of objective by another name" >&2
        status=1
    done < <(grep -nE 'mission_objective::(Kind[[:space:]]+as[[:space:]]|Kind::[*{]|\{[^}]*Kind[[:space:]]+as[[:space:]])' \
        "$root_dir/$file" || true)
    while IFS= read -r hit; do
        line="${hit%%:*}"
        kind="${hit#*Kind::}"
        echo "FORBIDDEN: $file:$line: Kind::$kind names a kind of objective outside planning" >&2
        status=1
    done < <(grep -noE '(^|[^A-Za-z0-9_])Kind::[A-Z][A-Za-z0-9]*' "$root_dir/$file" \
        | grep -vE 'Kind::Survey$' || true)
done <<< "$files"

if [ "$status" -ne 0 ]; then
    echo "Name only the survey, and treat any other kind the same, outside planning." >&2
    exit 1
fi
echo "objective kinds: OK"
