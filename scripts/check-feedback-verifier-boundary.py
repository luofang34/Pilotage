#!/usr/bin/env python3
"""Reject production paths that bypass the independent feedback verifier."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from pathlib import Path


SENSITIVE_NAMES = {
    "all_passed",
    "calculate_promotion",
    "classify",
    "is_promoted",
    "promotion_policy_digest",
    "run_terminal_policy_digest",
    "validate_for",
    "validate_for_runtime",
}
FORBIDDEN_SOURCE_NAMES = {
    "aviate_xil",
    "flight_tune_aviate",
    "flight_tune_xplane",
    "pilotage_xplane",
    "test_rig",
}
ALLOWED_MACROS = {"assert", "assert_eq", "format", "matches", "vec"}
TOKEN_PATTERN = re.compile(r"r#[A-Za-z_][A-Za-z0-9_]*|[A-Za-z_][A-Za-z0-9_]*|::|[^\s]")
IDENTIFIER_PATTERN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


@dataclass(frozen=True)
class Token:
    """One Rust token that is sufficient for this boundary check."""

    value: str
    line: int
    raw: bool


def production_files(root: Path) -> list[Path]:
    """Return all production Rust sources in stable order."""
    source_root = root / "crates/pilotage-tuning-feedback/src"
    if not source_root.is_dir():
        return []
    files = []
    for path in source_root.rglob("*.rs"):
        relative_parts = path.relative_to(source_root).parts
        if path.name in {"tests.rs", "test_support.rs"}:
            continue
        if "tests" in relative_parts or "test_support" in relative_parts:
            continue
        files.append(path)
    return sorted(files)


def sanitize(source: str) -> str:
    """Remove comments and literals while preserving source line numbers."""
    output = list(source)
    index = 0
    block_depth = 0
    while index < len(source):
        if block_depth:
            if source.startswith("/*", index):
                blank(output, index, 2)
                block_depth += 1
                index += 2
            elif source.startswith("*/", index):
                blank(output, index, 2)
                block_depth -= 1
                index += 2
            else:
                blank(output, index, 1)
                index += 1
            continue
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end < 0 else end
            blank(output, index, end - index)
            index = end
            continue
        if source.startswith("/*", index):
            blank(output, index, 2)
            block_depth = 1
            index += 2
            continue
        literal_end = string_literal_end(source, index)
        if literal_end is not None:
            blank(output, index, literal_end - index)
            index = literal_end
            continue
        index += 1
    return "".join(output)


def blank(output: list[str], start: int, length: int) -> None:
    """Blank a source range and keep each newline."""
    for offset in range(start, start + length):
        if output[offset] != "\n":
            output[offset] = " "


def string_literal_end(source: str, start: int) -> int | None:
    """Return the end of one Rust string or character literal."""
    prefix = start
    if source.startswith("br", start) or source.startswith("rb", start):
        prefix += 2
    elif source.startswith("r", start) or source.startswith("b", start):
        prefix += 1
    if prefix < len(source) and source[prefix] == "#":
        hashes = 0
        while prefix + hashes < len(source) and source[prefix + hashes] == "#":
            hashes += 1
        quote = prefix + hashes
        if quote < len(source) and source[quote] == '"':
            marker = '"' + ("#" * hashes)
            end = source.find(marker, quote + 1)
            return len(source) if end < 0 else end + len(marker)
    quote = prefix if prefix < len(source) else start
    if quote >= len(source) or source[quote] not in {'"', "'"}:
        return None
    if source[quote] == "'" and quote + 2 < len(source) and source[quote + 2] != "'":
        return None
    index = quote + 1
    while index < len(source):
        if source[index] == "\\":
            index += 2
            continue
        if source[index] == source[quote]:
            return index + 1
        index += 1
    return len(source)


def tokens(source: str) -> list[Token]:
    """Tokenize the names and punctuation used by this check."""
    result = []
    for line_number, line in enumerate(sanitize(source).splitlines(), start=1):
        for match in TOKEN_PATTERN.finditer(line):
            value = match.group(0)
            raw = value.startswith("r#")
            result.append(Token(value[2:] if raw else value, line_number, raw))
    return result


def is_allowed_validate(path: str, parsed: list[Token], index: int) -> bool:
    """Accept only the two reviewed validation calls."""
    values = [token.value for token in parsed]
    receipt = [
        "::",
        "flight_tune",
        "::",
        "RunTerminalReceipt",
        "::",
        "validate",
        "(",
        "receipt",
        ")",
    ]
    writer = ["writer", ".", "validate", "(", "&", "directory", ")"]
    if parsed[index].raw:
        return False
    if path == "crates/pilotage-tuning-feedback/src/qualification/evaluation.rs":
        return values[max(0, index - 5) : index + 4] == receipt
    if path == "crates/pilotage-tuning-feedback/src/storage.rs":
        return values[max(0, index - 2) : index + 5] == writer
    return False


def core_imports(source: str) -> dict[str, str]:
    """Return each local name imported from the core crate."""
    clean = sanitize(source)
    imported: dict[str, str] = {}
    group = re.compile(r"\buse\s+(?:::)?flight_tune\s*::\s*\{(.*?)\}\s*;", re.DOTALL)
    direct = re.compile(
        r"\buse\s+(?:::)?flight_tune\s*::\s*([A-Z][A-Za-z0-9_]*)"
        r"(?:\s+as\s+(r#)?([A-Za-z_][A-Za-z0-9_]*))?\s*;"
    )
    item = re.compile(
        r"^\s*([A-Z][A-Za-z0-9_]*)(?:\s+as\s+(?:r#)?([A-Za-z_][A-Za-z0-9_]*))?\s*$"
    )
    for match in group.finditer(clean):
        for entry in match.group(1).split(","):
            parsed = item.match(entry)
            if parsed:
                original = parsed.group(1)
                imported[parsed.group(2) or original] = original
    for match in direct.finditer(clean):
        original = match.group(1)
        imported[match.group(3) or original] = original
    return imported


def is_supported_core_use(parsed: list[Token]) -> bool:
    """Accept one direct or simple grouped core import."""
    values = [token.value for token in parsed]
    cursor = 0
    if values[:1] == ["::"]:
        cursor += 1
    if values[cursor : cursor + 2] != ["flight_tune", "::"]:
        return False
    cursor += 2
    if cursor >= len(values):
        return False
    if values[cursor] != "{":
        if not values[cursor][:1].isupper() or parsed[cursor].raw:
            return False
        cursor += 1
        if cursor < len(values) and values[cursor] == "as":
            cursor += 2
        return cursor == len(values)
    cursor += 1
    while cursor < len(values) and values[cursor] != "}":
        if not values[cursor][:1].isupper() or parsed[cursor].raw:
            return False
        cursor += 1
        if cursor < len(values) and values[cursor] == "as":
            cursor += 2
        if cursor < len(values) and values[cursor] == ",":
            cursor += 1
        elif cursor >= len(values) or values[cursor] != "}":
            return False
    return cursor + 1 == len(values) and values[cursor] == "}"


def forbidden_core_use_lines(parsed: list[Token]) -> list[int]:
    """Return unsupported core import lines."""
    values = [token.value for token in parsed]
    result = []
    for index, token in enumerate(parsed):
        if token.value != "use":
            continue
        try:
            end = values.index(";", index + 1)
        except ValueError:
            result.append(token.line)
            continue
        statement = parsed[index + 1 : end]
        if any(part.value == "flight_tune" for part in statement) and not is_supported_core_use(
            statement
        ):
            result.append(token.line)
    return result


def forbidden_core_type_alias_lines(
    parsed: list[Token], imported: dict[str, str]
) -> list[int]:
    """Return type alias lines that can hide one core type."""
    values = [token.value for token in parsed]
    result = []
    for index, token in enumerate(parsed):
        if token.value != "type":
            continue
        try:
            end = values.index(";", index + 1)
            equals = values.index("=", index + 1, end)
        except ValueError:
            continue
        right = values[equals + 1 : end]
        if "flight_tune" in right or any(value in imported for value in right):
            result.append(token.line)
    return result


def inside_attribute(values: list[str], index: int) -> bool:
    """Return true when one token is inside a Rust attribute."""
    bracket_depth = 0
    for cursor in range(index - 1, -1, -1):
        if values[cursor] == "]":
            bracket_depth += 1
        elif values[cursor] == "[":
            if bracket_depth:
                bracket_depth -= 1
            elif cursor > 0 and values[cursor - 1] == "#":
                return True
            else:
                return False
    return False


def associated_member(values: list[str], type_index: int) -> str:
    """Return one direct or UFCS associated member."""
    cursor = type_index + 1
    if cursor < len(values) and values[cursor] == ">":
        cursor += 1
    elif cursor < len(values) and values[cursor] == "as":
        while cursor < len(values) and values[cursor] != ">":
            cursor += 1
        cursor += 1
    if cursor + 1 < len(values) and values[cursor] == "::":
        return values[cursor + 1]
    return ""


def forbidden_core_association(
    parsed: list[Token], values: list[str], index: int, imported: dict[str, str]
) -> bool:
    """Reject a callable core item that is not an approved primitive."""
    receiver = parsed[index]
    allowed = {
        ("Digest", "from_bytes"),
        ("Digest", "is_zero"),
        ("RunTerminalReceipt", "receipt_digest"),
        ("RunTerminalReceipt", "validate"),
    }
    if receiver.value == "flight_tune" and values[index + 1 : index + 2] == ["::"]:
        original = values[index + 2]
        member = associated_member(values, index + 2)
        return member[:1].islower() and (original, member) not in allowed
    if receiver.value not in imported:
        return False
    member = associated_member(values, index)
    original = imported[receiver.value]
    if (original, member) in allowed:
        return False
    return member[:1].islower()


def violations(path: str, source: str) -> list[int]:
    """Return each line that crosses the verifier boundary."""
    parsed = tokens(source)
    values = [token.value for token in parsed]
    imported = core_imports(source)
    result = forbidden_core_use_lines(parsed)
    result.extend(forbidden_core_type_alias_lines(parsed, imported))
    for index, token in enumerate(parsed):
        value = token.value
        next_value = values[index + 1] if index + 1 < len(values) else ""
        previous = values[index - 1] if index else ""
        after_bang = values[index + 2] if index + 2 < len(values) else ""
        if (
            IDENTIFIER_PATTERN.fullmatch(value)
            and next_value == "!"
            and after_bang in {"(", "[", "{"}
            and value not in ALLOWED_MACROS
        ):
            result.append(token.line)
        if value == "macro_rules":
            result.append(token.line)
        if value == "validate" and not is_allowed_validate(path, parsed, index):
            result.append(token.line)
        if (
            value in SENSITIVE_NAMES
            or value in FORBIDDEN_SOURCE_NAMES
            or value.startswith("recompute_")
        ):
            result.append(token.line)
        if value == "digest" and (previous == "::" or (previous == "." and next_value == "(")):
            result.append(token.line)
        if value == "path" and inside_attribute(values, index):
            result.append(token.line)
        if token.raw and value in {"digest", "tests", "test_support", "validate"}:
            result.append(token.line)
        if forbidden_core_association(parsed, values, index, imported):
            result.append(token.line)
        if value == "flight_tune" and (previous in {"as", "mod"} or next_value == "as"):
            result.append(token.line)
    return sorted(set(result))


def main() -> int:
    """Check one repository root."""
    if len(sys.argv) != 2:
        print("usage: check-feedback-verifier-boundary.py ROOT", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve()
    failed = False
    for path in production_files(root):
        relative = path.relative_to(root).as_posix()
        try:
            source = path.read_text(encoding="utf-8")
        except OSError as error:
            print(f"FORBIDDEN: {relative} cannot be scanned: {error}", file=sys.stderr)
            failed = True
            continue
        for line in violations(relative, source):
            print(
                f"FORBIDDEN: {relative}:{line} bypasses the independent feedback verifier boundary",
                file=sys.stderr,
            )
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
