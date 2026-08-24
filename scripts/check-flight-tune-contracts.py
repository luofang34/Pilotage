#!/usr/bin/env python3
"""Check simulator-neutral tuning manifests and Rust contract identifiers."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

FORBIDDEN_PACKAGES = {"flight-tune-xplane", "pilotage-xplane-trial"}
FORBIDDEN_IDENTIFIERS = (
    "xplane",
    "acf",
    "aircraftfile",
    "trialplugin",
    "bridgeplugin",
    "weatherplugin",
    "hostapplicationid",
    "sdkversion",
)
CAMPAIGN_SHARED_TYPES = {
    "CampaignBudgetLimit",
    "ExecutionTarget",
    "CampaignPurpose",
    "PinnedFile",
    "SearchGroupConfig",
    "SearchGroupKind",
    "CampaignConfig",
    "TrainingGuardScenarioConfig",
    "TrainingSuiteConfig",
}
CAMPAIGN_ADAPTER_TYPES = {
    "XPlaneCampaignConfig",
    "XPlaneSupportBundleConfig",
    "XPlaneRuntimePluginConfig",
    "AviateCampaignConfig",
}
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\Z")


class ParseError(Exception):
    """A Rust source file cannot be tokenized or divided into type bodies."""


def report(message: str) -> None:
    """Write one stable guard diagnostic."""
    print(f"FORBIDDEN: {message}", file=sys.stderr)


def normalized(identifier: str) -> str:
    """Return the comparison form for one Rust identifier."""
    return identifier.lower().replace("_", "").replace("-", "")


def is_forbidden_identifier(identifier: str) -> bool:
    """Test one identifier against all simulator-specific name fragments."""
    value = normalized(identifier)
    return any(fragment in value for fragment in FORBIDDEN_IDENTIFIERS)


def check_manifest(manifest: Path, root: Path) -> bool:
    """Use Cargo's resolved dependency view for one neutral package."""
    if not manifest.is_file():
        return True
    result = subprocess.run(
        [
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
            str(manifest),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    relative = manifest.relative_to(root)
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()
        suffix = f": {detail[-1]}" if detail else ""
        report(f"{relative} cargo metadata failed{suffix}")
        return False
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        report(f"{relative} cargo metadata is not valid JSON: {error}")
        return False

    resolved = manifest.resolve()
    package = next(
        (
            item
            for item in metadata.get("packages", [])
            if Path(item.get("manifest_path", "")).resolve() == resolved
        ),
        None,
    )
    if package is None:
        report(f"{relative} is absent from cargo metadata")
        return False

    valid = True
    for dependency in package.get("dependencies", []):
        if (
            dependency.get("kind") != "dev"
            and dependency.get("name") in FORBIDDEN_PACKAGES
        ):
            report(f"{relative} has runtime dependency {dependency['name']}")
            valid = False
    return valid


def raw_string_end(source: str, start: int) -> int | None:
    """Return the end of a Rust raw string that starts at one byte offset."""
    prefix_length = 0
    if source.startswith("br", start):
        prefix_length = 2
    elif source.startswith("r", start):
        prefix_length = 1
    else:
        return None
    cursor = start + prefix_length
    hashes = 0
    while cursor < len(source) and source[cursor] == "#":
        hashes += 1
        cursor += 1
    if cursor >= len(source) or source[cursor] != '"':
        return None
    terminator = '"' + ("#" * hashes)
    end = source.find(terminator, cursor + 1)
    if end < 0:
        raise ParseError("an unterminated raw string")
    return end + len(terminator)


def quoted_end(source: str, start: int, quote: str) -> int:
    """Return the end of one escaped Rust string."""
    cursor = start + 1
    while cursor < len(source):
        if source[cursor] == "\\":
            cursor += 2
            continue
        if source[cursor] == quote:
            return cursor + 1
        cursor += 1
    raise ParseError("an unterminated string")


def character_end(source: str, start: int) -> int | None:
    """Return the end of one Rust character literal, but not a lifetime."""
    cursor = start + 1
    if cursor >= len(source) or source[cursor] == "\n":
        return None
    if source[cursor] == "\\":
        cursor += 1
        while cursor < len(source) and source[cursor] not in {"'", "\n"}:
            cursor += 1
    else:
        cursor += 1
    if cursor < len(source) and source[cursor] == "'":
        return cursor + 1
    return None


def rust_tokens(source: str) -> list[str]:
    """Return Rust identifier and punctuation tokens without comments or literals."""
    tokens: list[str] = []
    cursor = 0
    while cursor < len(source):
        if source[cursor].isspace():
            cursor += 1
            continue
        if source.startswith("//", cursor):
            end = source.find("\n", cursor + 2)
            cursor = len(source) if end < 0 else end + 1
            continue
        if source.startswith("/*", cursor):
            depth = 1
            cursor += 2
            while cursor < len(source) and depth > 0:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            if depth != 0:
                raise ParseError("an unterminated block comment")
            continue
        raw_end = raw_string_end(source, cursor)
        if raw_end is not None:
            cursor = raw_end
            continue
        if source[cursor] == '"':
            cursor = quoted_end(source, cursor, '"')
            continue
        if source[cursor] == "'":
            character_literal_end = character_end(source, cursor)
            if character_literal_end is not None:
                cursor = character_literal_end
                continue
        raw_identifier = re.match(r"r#([A-Za-z_][A-Za-z0-9_]*)", source[cursor:])
        if raw_identifier is not None:
            tokens.append(raw_identifier.group(1))
            cursor += len(raw_identifier.group(0))
            continue
        identifier = re.match(r"[A-Za-z_][A-Za-z0-9_]*", source[cursor:])
        if identifier is not None:
            tokens.append(identifier.group(0))
            cursor += len(identifier.group(0))
            continue
        tokens.append(source[cursor])
        cursor += 1
    return tokens


def group_end(tokens: list[str], start: int, opening: str, closing: str) -> int:
    """Return the token after one balanced group."""
    depth = 0
    for index in range(start, len(tokens)):
        if tokens[index] == opening:
            depth += 1
        elif tokens[index] == closing:
            depth -= 1
            if depth == 0:
                return index + 1
    raise ParseError(f"an unclosed {opening}{closing} group")


def type_body_start(tokens: list[str], start: int) -> int | None:
    """Find a named struct or enum body after its type name."""
    paren_depth = 0
    bracket_depth = 0
    angle_depth = 0
    for index in range(start, len(tokens)):
        token = tokens[index]
        if token == "(":
            paren_depth += 1
        elif token == ")":
            paren_depth = max(0, paren_depth - 1)
        elif token == "[":
            bracket_depth += 1
        elif token == "]":
            bracket_depth = max(0, bracket_depth - 1)
        elif token == "<":
            angle_depth += 1
        elif token == ">":
            angle_depth = max(0, angle_depth - 1)
        elif token == "{" and paren_depth == bracket_depth == angle_depth == 0:
            return index
        elif token == ";" and paren_depth == bracket_depth == angle_depth == 0:
            return None
    raise ParseError("a type declaration has no body terminator")


def top_level_segments(tokens: list[str]) -> list[list[str]]:
    """Divide one type body at top-level commas."""
    segments: list[list[str]] = []
    current: list[str] = []
    depths = {"(": 0, "[": 0, "{": 0, "<": 0}
    closing = {")": "(", "]": "[", "}": "{", ">": "<"}
    for token in tokens:
        if token == "," and all(depth == 0 for depth in depths.values()):
            if current:
                segments.append(current)
            current = []
            continue
        current.append(token)
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
    if current:
        segments.append(current)
    return segments


def strip_prefix(tokens: list[str]) -> list[str]:
    """Remove field or variant attributes and visibility tokens."""
    cursor = 0
    while cursor + 1 < len(tokens) and tokens[cursor : cursor + 2] == ["#", "["]:
        cursor = group_end(tokens, cursor + 1, "[", "]")
    if cursor < len(tokens) and tokens[cursor] == "pub":
        cursor += 1
        if cursor < len(tokens) and tokens[cursor] == "(":
            cursor = group_end(tokens, cursor, "(", ")")
    return tokens[cursor:]


def named_field_identifiers(body: list[str]) -> list[tuple[str, str]]:
    """Return named field identifiers from one braced body."""
    records: list[tuple[str, str]] = []
    for segment in top_level_segments(body):
        candidate = strip_prefix(segment)
        if ":" not in candidate:
            continue
        colon = candidate.index(":")
        names = [token for token in candidate[:colon] if IDENTIFIER.fullmatch(token)]
        if names:
            records.append(("field", names[-1]))
    return records


def contract_identifiers(kind: str, body: list[str]) -> list[tuple[str, str]]:
    """Return field or variant identifiers from one named type body."""
    if kind == "struct":
        return named_field_identifiers(body)

    records: list[tuple[str, str]] = []
    for segment in top_level_segments(body):
        candidate = strip_prefix(segment)
        if candidate and IDENTIFIER.fullmatch(candidate[0]):
            records.append(("variant", candidate[0]))
            if len(candidate) > 1 and candidate[1] == "{":
                body_end = group_end(candidate, 1, "{", "}")
                records.extend(named_field_identifiers(candidate[2 : body_end - 1]))
    return records


def public_types(tokens: list[str]) -> list[tuple[str, str, list[tuple[str, str]]]]:
    """Return public named structs and enums with their contract identifiers."""
    records: list[tuple[str, str, list[tuple[str, str]]]] = []
    cursor = 0
    public = False
    while cursor < len(tokens):
        if cursor + 1 < len(tokens) and tokens[cursor : cursor + 2] == ["#", "["]:
            cursor = group_end(tokens, cursor + 1, "[", "]")
            continue
        if tokens[cursor] == "pub":
            public = True
            cursor += 1
            if cursor < len(tokens) and tokens[cursor] == "(":
                cursor = group_end(tokens, cursor, "(", ")")
            continue
        if tokens[cursor] not in {"struct", "enum"}:
            public = False
            cursor += 1
            continue
        kind = tokens[cursor]
        if cursor + 1 >= len(tokens) or not IDENTIFIER.fullmatch(tokens[cursor + 1]):
            raise ParseError(f"a public {kind} has no identifier")
        name = tokens[cursor + 1]
        body_start = type_body_start(tokens, cursor + 2)
        if body_start is None:
            public = False
            cursor += 2
            continue
        body_end = group_end(tokens, body_start, "{", "}")
        if public:
            records.append(
                (
                    kind,
                    name,
                    contract_identifiers(kind, tokens[body_start + 1 : body_end - 1]),
                )
            )
        public = False
        cursor = body_end
    return records


def is_production_path(path: Path, source_root: Path) -> bool:
    """Exclude explicit Rust test modules from a production source scan."""
    relative = path.relative_to(source_root)
    return (
        path.name not in {"tests.rs", "test_support.rs"}
        and "tests" not in relative.parts
        and "test_support" not in relative.parts
    )


def read_public_types(
    path: Path, root: Path
) -> list[tuple[str, str, list[tuple[str, str]]]] | None:
    """Read and parse one Rust source file or report a fail-closed error."""
    try:
        return public_types(rust_tokens(path.read_text(encoding="utf-8")))
    except (OSError, UnicodeError, ParseError) as error:
        report(f"{path.relative_to(root)} cannot be parsed: {error}")
        return None


def check_type_identifiers(
    path: Path,
    root: Path,
    records: list[tuple[str, str, list[tuple[str, str]]]],
) -> bool:
    """Reject simulator-specific fields and variants in public shared types."""
    valid = True
    relative = path.relative_to(root)
    for _, type_name, identifiers in records:
        for kind, identifier in identifiers:
            if is_forbidden_identifier(identifier):
                report(
                    f"{relative} {type_name} has simulator-specific {kind} {identifier}"
                )
                valid = False
    return valid


def check_source_root(source_root: Path, root: Path) -> bool:
    """Check all production Rust files below one shared runtime root."""
    if not source_root.is_dir():
        return True
    valid = True
    for path in sorted(source_root.rglob("*.rs")):
        if not is_production_path(path, source_root):
            continue
        records = read_public_types(path, root)
        if records is None:
            valid = False
        elif not check_type_identifiers(path, root, records):
            valid = False
    return valid


def check_campaign_file(path: Path, root: Path) -> bool:
    """Check classified public contracts in one campaign configuration file."""
    records = read_public_types(path, root)
    if records is None:
        return False
    valid = True
    for kind, type_name, identifiers in records:
        if type_name in CAMPAIGN_ADAPTER_TYPES:
            continue
        if type_name not in CAMPAIGN_SHARED_TYPES:
            report(
                f"{path.relative_to(root)} has unclassified public campaign contract {type_name}"
            )
            valid = False
        if not check_type_identifiers(path, root, [(kind, type_name, identifiers)]):
            valid = False
    return valid


def check_campaign_config(root: Path) -> bool:
    """Check the campaign document and all production configuration modules."""
    config_file = root / "tools/flight-tune-campaign/src/config.rs"
    config_root = root / "tools/flight-tune-campaign/src/config"
    paths: list[Path] = []
    if config_file.is_file():
        paths.append(config_file)
    if config_root.is_dir():
        paths.extend(
            path
            for path in sorted(config_root.rglob("*.rs"))
            if is_production_path(path, config_root)
        )
    valid = True
    for path in paths:
        if not check_campaign_file(path, root):
            valid = False
    return valid


def main() -> int:
    """Run all simulator-neutral contract checks."""
    if len(sys.argv) != 2:
        print("usage: check-flight-tune-contracts.py ROOT", file=sys.stderr)
        return 2
    root = Path(sys.argv[1]).resolve()
    valid = True
    for manifest in (
        root / "crates/pilotage-trial/Cargo.toml",
        root / "tools/flight-tune/Cargo.toml",
    ):
        if not check_manifest(manifest, root):
            valid = False
    for source_root in (
        root / "crates/pilotage-trial/src",
        root / "tools/flight-tune/src",
    ):
        if not check_source_root(source_root, root):
            valid = False
    if not check_campaign_config(root):
        valid = False
    return 0 if valid else 1


if __name__ == "__main__":
    raise SystemExit(main())
