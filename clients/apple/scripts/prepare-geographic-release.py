#!/usr/bin/env python3
"""Package a verified geographic archive for local development."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--id", required=True)
    parser.add_argument("--product", choices=["terrain", "basemap"], required=True)
    parser.add_argument("--authority", required=True)
    parser.add_argument("--edition", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", args.id) or args.output.exists():
        raise ValueError("Use a valid release ID and a new output directory")
    source = json.loads(args.manifest.read_text())
    if args.archive.stat().st_size != source["archive_bytes"] or digest(args.archive) != source["archive_sha256"]:
        raise ValueError("Archive does not match its source manifest")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent, prefix="geographic-") as temporary:
        target = Path(temporary) / args.id
        target.mkdir()
        artifacts = []
        for original, name, format_name in [(args.archive, "archive.mbtiles", "mbtiles"),
                                             (args.manifest, "source-manifest.json", "resource")]:
            path = target / name
            shutil.copyfile(original, path)
            artifacts.append({"path": name, "source": name, "format": format_name,
                              "bytes": path.stat().st_size, "sha256": digest(path)})
        release = {
            "schema_version": 1, "id": args.id, "product": args.product, "authority": args.authority,
            "revision": 1, "edition": args.edition, "source_set": digest(args.manifest),
            "channel": "development", "distribution": "unspecified", "validity": None,
            "coverage": {"name": "World overview and regional detail", "bounds": [-180, -85.051129, 180, 85.051129],
                         "min_zoom": min(band["min_zoom"] for band in source["bands"]),
                         "max_zoom": max(band["max_zoom"] for band in source["bands"]), "complete": False,
                         "exclusions": ["Detailed data covers the regions in source-manifest.json.",
                                        "World coverage is limited to overview zoom levels."]},
            "artifacts": artifacts, "dependencies": [], "renderer_capabilities": [],
            "attributions": [source.get("attribution", args.authority)],
        }
        (target / "release.json").write_text(json.dumps(release, indent=2) + "\n")
        target.rename(args.output)
    print(f"Prepared {args.id}")


if __name__ == "__main__":
    main()
