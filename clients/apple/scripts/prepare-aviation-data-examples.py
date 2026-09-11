#!/usr/bin/env python3
"""Copy verified local releases into the Apple development bundle."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import sqlite3
import tempfile


def copy_release(connection, store, release_id, target):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", release_id):
        raise ValueError(f"Invalid release ID: {release_id}")
    row = connection.execute("SELECT manifest FROM releases WHERE id = ?", (release_id,)).fetchone()
    if row is None:
        raise ValueError(f"Release is not installed: {release_id}")
    release = json.loads(row[0])
    if release["channel"] != "development":
        raise ValueError(f"Example must use the development channel: {release_id}")
    source = (store / "releases" / release_id).resolve(strict=True)
    output = target / release_id
    output.mkdir()
    for artifact in release["artifacts"]:
        relative = Path(artifact["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"Invalid artifact path: {relative}")
        original = (source / relative).resolve(strict=True)
        if not original.is_relative_to(source):
            raise ValueError(f"Artifact is outside the release: {relative}")
        copied = output / relative
        copied.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, copied)
        with copied.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        if digest != artifact["sha256"] or copied.stat().st_size != artifact["bytes"]:
            raise ValueError(f"Artifact verification failed: {relative}")
    (output / "release.json").write_text(json.dumps(release, indent=2) + "\n")
    return release_id + "/release.json"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("store", type=Path, help="Verified local package store")
    parser.add_argument("releases", nargs="+", help="Release IDs in dependency order")
    args = parser.parse_args()
    store = args.store.resolve(strict=True)
    build = Path(__file__).resolve().parent.parent / ".build"
    build.mkdir(exist_ok=True)
    output = build / "AviationDataExamples"
    with tempfile.TemporaryDirectory(prefix="aviation-examples-", dir=build) as temporary:
        target = Path(temporary) / "AviationDataExamples"
        target.mkdir()
        with sqlite3.connect((store / "catalog.sqlite").as_uri() + "?mode=ro", uri=True) as connection:
            index = [copy_release(connection, store, release_id, target) for release_id in args.releases]
        (target / "index.json").write_text(json.dumps(index, indent=2) + "\n")
        if output.exists():
            shutil.rmtree(output)
        target.rename(output)
    print(f"Prepared {len(index)} development releases at {output}")


if __name__ == "__main__":
    main()
