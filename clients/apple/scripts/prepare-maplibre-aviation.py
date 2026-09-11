#!/usr/bin/env python3
"""Apply the pinned aviation renderer changes to an extracted source tree."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


CLIENT_ROOT = Path(__file__).resolve().parents[1]
PROJECT_ROOT = CLIENT_ROOT.parents[1]
PACKAGE = CLIENT_ROOT / "renderer" / "aviation"
DEPENDENCY_TOKEN = '"@PILOTAGE_MAP_ARCHIVES@"'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def dependency_path(project_root):
    return json.dumps(str(project_root / "crates" / "pilotage-map-archives"))


def source_digest_blocking(source, item, project_root, normalized):
    path = source / item["path"]
    if not path.is_file():
        return None
    data = path.read_bytes()
    if normalized and item["path"] == "apple/visionos/Cargo.toml":
        data = data.decode().replace(dependency_path(project_root), DEPENDENCY_TOKEN).encode()
    return digest(data)


def verify_resources_blocking(source, config):
    for resource in config["resources"]:
        path = source / resource["target"]
        if not path.is_file() or digest(path.read_bytes()) != resource["output_sha256"]:
            raise ValueError(f"Renderer resource does not match its digest: {path}")


def verify_lock_blocking(source, lock):
    path = source / "apple/visionos/Cargo.lock"
    if not path.is_file() or path.read_bytes() != lock:
        raise ValueError("The native renderer lock file does not match its pinned input.")


def resource_inputs_blocking(source, ifr_source, config):
    package = ifr_source / "tools" / "maplibre-rs-web"
    resources = []
    for resource in config["resources"]:
        path = package / resource["path"]
        data = path.read_bytes()
        if digest(data) != resource["sha256"]:
            raise ValueError(f"IFR Map Lab resource does not match its digest: {path}")
        if "base" in resource:
            base = (source / resource["base"]).read_bytes()
            if digest(base) != resource["base_sha256"]:
                raise ValueError(f"Renderer base resource does not match: {resource['base']}")
            data = base + data
        if digest(data) != resource["output_sha256"]:
            raise ValueError(f"Combined renderer resource does not match: {resource['target']}")
        resources.append((source / resource["target"], data))
    return resources


def run_patch_blocking(source, patch, dry_run):
    command = ["patch", "-p1", "--batch", "--forward", "--fuzz=0", "--no-backup-if-mismatch"]
    if dry_run:
        command.append("--dry-run")
    result = subprocess.run(command, cwd=source, input=patch, capture_output=True, check=False)
    if result.returncode != 0:
        raise ValueError(result.stdout.decode() + result.stderr.decode())


def prepare_blocking(source, ifr_source, package, project_root, verify_only=False):
    config = json.loads((package / "build.json").read_text())
    revision = (project_root / "clients/apple/MAPLIBRE_GLOBE_REVISION").read_text().strip()
    if config["schema_version"] != 1 or config["renderer_revision"] != revision:
        raise ValueError("The aviation renderer configuration does not match the pinned revision.")
    patch = (package / config["patch"]["path"]).read_bytes()
    if digest(patch) != config["patch"]["sha256"]:
        raise ValueError("The aviation renderer patch does not match its digest.")
    lock = (package / config["lock"]["path"]).read_bytes()
    if digest(lock) != config["lock"]["sha256"]:
        raise ValueError("The native renderer lock input does not match its digest.")
    complete = all(source_digest_blocking(source, item, project_root, True) == item["after_sha256"]
                   for item in config["files"])
    if complete:
        verify_resources_blocking(source, config)
        lock_path = source / "apple/visionos/Cargo.lock"
        if not verify_only and not lock_path.exists():
            lock_path.write_bytes(lock)
        verify_lock_blocking(source, lock)
        return "verified"
    if verify_only:
        raise ValueError("The renderer source does not match the aviation configuration.")
    for item in config["files"]:
        if source_digest_blocking(source, item, project_root, False) != item["before_sha256"]:
            raise ValueError(f"Renderer source does not match the base: {item['path']}")
    resources = resource_inputs_blocking(source, ifr_source, config)
    patch = patch.decode().replace(DEPENDENCY_TOKEN, dependency_path(project_root)).encode()
    run_patch_blocking(source, patch, True)
    run_patch_blocking(source, patch, False)
    for path, data in resources:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    (source / "apple/visionos/Cargo.lock").write_bytes(lock)
    prepare_blocking(source, ifr_source, package, project_root, verify_only=True)
    return "prepared"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--ifr-source", type=Path, default=PROJECT_ROOT.parent / "ifr-map-lab")
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args()
    state = prepare_blocking(args.source.resolve(), args.ifr_source.resolve(), PACKAGE,
                             PROJECT_ROOT, args.verify_only)
    print(f"Aviation renderer source {state}.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error
