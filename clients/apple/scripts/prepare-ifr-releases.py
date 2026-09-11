#!/usr/bin/env python3
"""Build development chart releases from verified IFR Map Lab output."""

import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def archive_info(path, expected):
    if path.stat().st_size != expected["bytes"] or digest(path) != expected["sha256"]:
        raise ValueError(f"Archive does not match the producer manifest: {path.name}")
    with path.open("rb") as stream:
        header = stream.read(127)
        if header[:8] != b"PMTiles\x03" or header[97] not in (1, 2):
            raise ValueError(f"Unsupported PMTiles header: {path.name}")
        offset, length = struct.unpack_from("<QQ", header, 24)
        if length > 8 * 1024 * 1024 or offset + length > path.stat().st_size:
            raise ValueError(f"Invalid PMTiles metadata range: {path.name}")
        stream.seek(offset)
        data = stream.read(length)
        metadata = json.loads(gzip.decompress(data) if header[97] == 2 else data)
    return {
        "layers": {layer["id"] for layer in metadata["vector_layers"]},
        "minzoom": header[100], "maxzoom": header[101],
        "bounds": [value / 1e7 for value in struct.unpack_from("<iiii", header, 102)],
    }


def local_style(original, profiles):
    style = json.loads(json.dumps(original))
    style["sources"] = {
        name: {"type": "vector", "tiles": [f"pilotage://{name}/{{z}}/{{x}}/{{y}}"],
               "minzoom": info["minzoom"], "maxzoom": info["maxzoom"]}
        for name, info in profiles.items()
    }
    for layer in style["layers"]:
        if "source" not in layer:
            continue
        owners = [name for name, info in profiles.items() if layer.get("source-layer") in info["layers"]]
        if len(owners) != 1:
            raise ValueError(f"Style layer has no unique archive: {layer['id']}")
        layer["source"] = owners[0]
    style.pop("glyphs", None)
    style["sprite"] = "pilotage://symbols/point-sprites"
    # The producer metadata records style provenance. The release records data validity.
    metadata = style.setdefault("metadata", {})
    metadata.pop("pilotage:resources", None)
    metadata["pilotage:resource-artifacts"] = [
        {"uri": f"pilotage://{name}", "path": f"{name}.pmtiles", "format": "pmtiles"}
        for name in profiles
    ] + [
        {"uri": f"pilotage://symbols/point-sprites.{suffix}",
         "path": f"symbols/point-sprites.{suffix}", "format": "file"}
        for suffix in ("png", "json")
    ]
    return style


def copy_artifact(source, directory, relative, format_name):
    target = directory / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    return artifact(target, directory, format_name)


def artifact(path, directory, format_name):
    relative = str(path.relative_to(directory))
    return {"path": relative, "source": relative, "format": format_name,
            "bytes": path.stat().st_size, "sha256": digest(path)}


def build_release(args, target, profile, producer, profiles):
    release_id = f"ifr-{profile}-{args.edition}-r{args.revision}"
    directory = target / release_id
    directory.mkdir()
    artifacts = [copy_artifact(args.producer / producer["delivery"]["profiles"][name]["archive"],
                              directory, f"{name}.pmtiles", "pmtiles") for name in profiles]
    artifacts += [copy_artifact(args.producer / f"point-sprites.{suffix}", directory,
                               f"symbols/point-sprites.{suffix}", "resource") for suffix in ("png", "json")]
    artifacts.append(copy_artifact(args.producer / "manifest.json", directory, "source-manifest.json", "resource"))
    original = args.styles / f"{profile}-style.json"
    artifacts.append(copy_artifact(original, directory, "producer-style.json", "resource"))
    calibrated = subprocess.run([
        "node", str(Path(__file__).with_name("calibrate-ifr-style.mjs")), str(args.ifr_source),
        profile, str(args.reference_latitude), str(original),
    ], check=True, capture_output=True, text=True)
    write_json(directory / "style.json", local_style(json.loads(calibrated.stdout), profiles))
    artifacts.append(artifact(directory / "style.json", directory, "map_style"))
    calibration = {name: digest(args.ifr_source / name) for name in [
        "web/rust/chart-view.js", "web/rust/paper-scale.js", "web/raster-reference.js"]}
    write_json(directory / "style-inputs.json", calibration)
    artifacts.append(artifact(directory / "style-inputs.json", directory, "resource"))
    effective = datetime.strptime(producer["effective"], "%Y/%m/%d").replace(
        hour=9, minute=1, tzinfo=timezone.utc)
    expires = datetime.strptime(args.expires, "%Y-%m-%d").replace(hour=9, minute=1, tzinfo=timezone.utc)
    if expires <= effective:
        raise ValueError("Expiry must follow the producer effective date")
    release = {
        "schema_version": 1, "id": release_id, "product": f"ifr_{profile}",
        "authority": "FAA NASR and CIFP · IFR Map Lab", "revision": args.revision,
        "edition": args.edition, "source_set": digest(args.producer / "manifest.json"),
        "channel": "development", "distribution": "unspecified",
        "validity": {"effective_at": int(effective.timestamp()), "expires_at": int(expires.timestamp())},
        "coverage": {"name": "United States IFR chart sample", "bounds": profiles["network"]["bounds"],
                     "min_zoom": max(info["minzoom"] for info in profiles.values()),
                     "max_zoom": min(info["maxzoom"] for info in profiles.values()), "complete": False,
                     "exclusions": ["The producer declares incomplete chart coverage.",
                                    "The producer style is a diagnostic style. It has no conformance certification.",
                                    "Procedure legs and procedure charts are not included."]},
        "artifacts": artifacts, "dependencies": [], "renderer_capabilities": ["ifr-chart-v1"],
        "attributions": ["FAA NASR, CIFP, and d-TPP; IFR Map Lab chart portrayal.",
                         "Font distribution permission has not been established for this development build."],
    }
    write_json(directory / "release.json", release)
    return release_id


def build(args):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,80}", args.edition) or args.revision < 1:
        raise ValueError("Edition and revision must form an immutable release ID")
    producer = json.loads((args.producer / "manifest.json").read_text())
    profiles = {}
    for name in ("network", "detail"):
        entry = producer["delivery"]["profiles"][name]
        relative = Path(entry["archive"])
        if relative.is_absolute() or len(relative.parts) != 1:
            raise ValueError("Archive names must refer to producer files")
        profiles[name] = archive_info(args.producer / relative, entry["report"]["artifact"])
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.output.exists():
        raise ValueError(f"Output already exists: {args.output}")
    with tempfile.TemporaryDirectory(prefix="ifr-releases-", dir=args.output.parent) as temporary:
        target = Path(temporary) / "releases"
        target.mkdir()
        index = [build_release(args, target, profile, producer, profiles) for profile in ("low", "high")]
        write_json(target / "index.json", index)
        target.rename(args.output)
    return index


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("producer", type=Path)
    parser.add_argument("styles", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--edition", required=True)
    parser.add_argument("--ifr-source", required=True, type=Path, help="IFR Map Lab source checkout")
    parser.add_argument("--reference-latitude", required=True, type=float, help="Latitude for producer paper calibration")
    parser.add_argument("--expires", required=True, help="First invalid date at 09:01 UTC, YYYY-MM-DD")
    parser.add_argument("--revision", type=int, default=1)
    print("Prepared " + ", ".join(build(parser.parse_args())))


if __name__ == "__main__":
    main()
