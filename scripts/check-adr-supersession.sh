#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd "${script_dir}/.." && pwd)"

if [ "$#" -gt 1 ]; then
    echo "usage: $0 [ADR_ROOT]" >&2
    exit 2
fi

adr_root="${1:-${ADR_ROOT:-${repository_root}/docs/adr}}"
readme="${adr_root}/README.md"
failed=0

fail() {
    echo "$*" >&2
    failed=1
}

read_header() {
    local file="$1"
    local key="$2"

    awk -v key="$key" '
        BEGIN { prefix = "- " key ": " }
        /^##([[:space:]]|$)/ { body = 1 }
        !body && index($0, "- " key ":") == 1 {
            if (index($0, prefix) != 1) {
                malformed = 1
            } else {
                count++
                value = substr($0, length(prefix) + 1)
            }
        }
        END {
            if (malformed) exit 2
            if (count == 0) exit 3
            if (count != 1) exit 2
            print value
        }
    ' "$file"
}

valid_status() {
    local value="$1"

    case "$value" in
        Proposed|Accepted|Deprecated) return 0 ;;
    esac
    [[ "$value" =~ ^Superseded\ by\ ADR-[0-9]{4}$ ]]
}

resolve_adr_file() {
    local number="$1"
    local -a matches=()

    shopt -s nullglob
    matches=("${adr_root}/${number}-"*.md)
    shopt -u nullglob

    if [ "${#matches[@]}" -eq 0 ]; then
        echo "ADR-${number} has no file under ${adr_root}" >&2
        return 1
    fi
    if [ "${#matches[@]}" -ne 1 ]; then
        echo "ADR-${number} has ${#matches[@]} files under ${adr_root}" >&2
        return 1
    fi
    printf '%s\n' "${matches[0]}"
}

read_readme_entry() {
    local number="$1"

    awk -v wanted="$number" '
        function trim(value) {
            sub(/^[[:space:]]+/, "", value)
            sub(/[[:space:]]+$/, "", value)
            return value
        }
        $0 == "## Index" { in_index = 1; next }
        in_index && /^##([[:space:]]|$)/ { exit }
        in_index && /^\|/ {
            count_cells = split($0, cells, "|")
            if (count_cells < 5) next
            link = trim(cells[2])
            if (substr(link, 1, length(wanted) + 2) != "[" wanted "]") next
            matches++
            found_link = link
            found_status = trim(cells[4])
        }
        END {
            if (matches == 0) exit 3
            if (matches != 1) exit 2
            print found_link "\t" found_status
        }
    ' "$readme"
}

check_readme() {
    local number="$1"
    local expected_file="$2"
    local expected_status="$3"
    local entry link index_status

    if ! entry="$(read_readme_entry "$number")"; then
        fail "ADR-${number} must have exactly one row in ${readme}"
        return
    fi
    IFS=$'\t' read -r link index_status <<<"$entry"
    if [[ ! "$link" =~ ^\[([0-9]{4})\]\(([A-Za-z0-9._-]+\.md)\)$ ]]; then
        fail "ADR-${number} has an invalid README link: ${link}"
    elif [ "${BASH_REMATCH[1]}" != "$number" ] || [ "${BASH_REMATCH[2]}" != "$expected_file" ]; then
        fail "ADR-${number} README link must target ${expected_file}: ${link}"
    fi
    if [ "$index_status" != "$expected_status" ]; then
        fail "README status for ADR-${number} is '${index_status}'; file status is '${expected_status}'"
    fi
}

read_status() {
    local file="$1"
    local value

    if ! value="$(read_header "$file" Status)"; then
        echo "${file} must have exactly one '- Status:' field before its first section" >&2
        return 1
    fi
    if ! valid_status "$value"; then
        echo "${file} has an invalid status: ${value}" >&2
        return 1
    fi
    printf '%s\n' "$value"
}

read_forward_reference() {
    local file="$1"
    local value result

    if value="$(read_header "$file" "Supersedes on acceptance")"; then
        printf '%s\n' "$value"
        return 0
    else
        result=$?
    fi
    if [ "$result" -eq 3 ]; then
        return 3
    fi
    echo "${file} has a malformed or duplicate 'Supersedes on acceptance' field" >&2
    return 2
}

parse_forward_reference() {
    local owner_number="$1"
    local value="$2"

    if [[ ! "$value" =~ ^\[ADR-([0-9]{4})\]\(([0-9]{4}-[A-Za-z0-9._-]+\.md)\)$ ]]; then
        echo "ADR-${owner_number} has an invalid 'Supersedes on acceptance' value: ${value}" >&2
        return 1
    fi
    if [ "${BASH_REMATCH[1]}" != "${BASH_REMATCH[2]:0:4}" ]; then
        echo "ADR-${owner_number} supersession number and link disagree: ${value}" >&2
        return 1
    fi
    printf '%s\t%s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
}

validate_forward_edge() {
    local successor_file="$1"
    local successor_number="$2"
    local successor_status="$3"
    local reference parsed predecessor_number predecessor_name predecessor_file predecessor_status

    if ! reference="$(read_forward_reference "$successor_file")"; then
        return
    fi
    if ! parsed="$(parse_forward_reference "$successor_number" "$reference")"; then
        fail "ADR-${successor_number} has an invalid forward supersession reference"
        return
    fi
    IFS=$'\t' read -r predecessor_number predecessor_name <<<"$parsed"
    if [ "$predecessor_number" = "$successor_number" ]; then
        fail "ADR-${successor_number} cannot supersede itself"
        return
    fi
    if ! predecessor_file="$(resolve_adr_file "$predecessor_number")"; then
        failed=1
        return
    fi
    if [ "${predecessor_file##*/}" != "$predecessor_name" ]; then
        fail "ADR-${successor_number} must link to ${predecessor_file##*/}, not ${predecessor_name}"
        return
    fi
    if ! predecessor_status="$(read_status "$predecessor_file")"; then
        failed=1
        return
    fi

    case "$successor_status" in
        Proposed)
            if [ "$predecessor_status" != "Accepted" ]; then
                fail "proposed ADR-${successor_number} requires ADR-${predecessor_number} to be Accepted; found '${predecessor_status}'"
            fi
            ;;
        Accepted|Deprecated|Superseded\ by\ ADR-*)
            if [ "$predecessor_status" != "Superseded by ADR-${successor_number}" ]; then
                fail "${successor_status} ADR-${successor_number} requires ADR-${predecessor_number} status 'Superseded by ADR-${successor_number}'; found '${predecessor_status}'"
            fi
            ;;
    esac

    check_readme "$successor_number" "${successor_file##*/}" "$successor_status"
    check_readme "$predecessor_number" "$predecessor_name" "$predecessor_status"
}

validate_reverse_edge() {
    local predecessor_file="$1"
    local predecessor_number="$2"
    local predecessor_status="$3"
    local successor_number successor_file successor_status reference parsed linked_number linked_name

    if [[ ! "$predecessor_status" =~ ^Superseded\ by\ ADR-([0-9]{4})$ ]]; then
        return
    fi
    successor_number="${BASH_REMATCH[1]}"
    if [ "$successor_number" = "$predecessor_number" ]; then
        fail "ADR-${predecessor_number} cannot supersede itself"
        return
    fi
    if ! successor_file="$(resolve_adr_file "$successor_number")"; then
        failed=1
        return
    fi
    if ! successor_status="$(read_status "$successor_file")"; then
        failed=1
        return
    fi
    if [ "$successor_status" = "Proposed" ]; then
        fail "ADR-${predecessor_number} cannot be superseded by proposed ADR-${successor_number}"
    fi
    if ! reference="$(read_forward_reference "$successor_file")"; then
        fail "ADR-${successor_number} must declare that it supersedes ADR-${predecessor_number}"
        return
    fi
    if ! parsed="$(parse_forward_reference "$successor_number" "$reference")"; then
        fail "ADR-${successor_number} has an invalid forward supersession reference"
        return
    fi
    IFS=$'\t' read -r linked_number linked_name <<<"$parsed"
    if [ "$linked_number" != "$predecessor_number" ] || [ "$linked_name" != "${predecessor_file##*/}" ]; then
        fail "ADR-${successor_number} must declare that it supersedes ${predecessor_file##*/}"
    fi

    check_readme "$predecessor_number" "${predecessor_file##*/}" "$predecessor_status"
    check_readme "$successor_number" "${successor_file##*/}" "$successor_status"
}

if [ ! -d "$adr_root" ]; then
    echo "ADR root does not exist: ${adr_root}" >&2
    exit 1
fi
if [ ! -f "$readme" ]; then
    echo "ADR index does not exist: ${readme}" >&2
    exit 1
fi

shopt -s nullglob
adr_files=("${adr_root}"/[0-9][0-9][0-9][0-9]-*.md)
shopt -u nullglob
if [ "${#adr_files[@]}" -eq 0 ]; then
    echo "ADR root has no numbered ADR files: ${adr_root}" >&2
    exit 1
fi

for adr_file in "${adr_files[@]}"; do
    adr_name="${adr_file##*/}"
    adr_number="${adr_name:0:4}"
    if ! adr_status="$(read_status "$adr_file")"; then
        failed=1
        continue
    fi

    if read_forward_reference "$adr_file" >/dev/null 2>&1; then
        validate_forward_edge "$adr_file" "$adr_number" "$adr_status"
    else
        reference_status=$?
        if [ "$reference_status" -ne 3 ]; then
            read_forward_reference "$adr_file" >/dev/null || true
            failed=1
        fi
    fi
    validate_reverse_edge "$adr_file" "$adr_number" "$adr_status"
done

if [ "$failed" -ne 0 ]; then
    echo "check-adr-supersession: FAILED" >&2
    exit 1
fi

echo "check-adr-supersession: OK"
