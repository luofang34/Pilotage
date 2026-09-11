#!/usr/bin/env python3
"""Package selected FAA procedure PDFs for local development."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile
import xml.etree.ElementTree as ET


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def effective_time(value):
    return int(datetime.strptime(" ".join(value.split()), "%H%MZ %m/%d/%y")
               .replace(tzinfo=timezone.utc).timestamp())


def prepare(metadata, source, output, release_id, airport, pdf_names):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", release_id) or output.exists():
        raise ValueError("Use a valid release ID and a new output directory")
    if not pdf_names or len(set(pdf_names)) != len(pdf_names):
        raise ValueError("Select each PDF once")
    root = ET.parse(metadata).getroot()
    cycle = root.attrib["cycle"]
    records = {}
    for facility in root.iter("airport_name"):
        if facility.get("icao_ident") == airport:
            for record in facility.findall("record"):
                name = record.findtext("pdf_name", "")
                if name in pdf_names:
                    records[name] = record
    if set(records) != set(pdf_names):
        raise ValueError("Each PDF must be listed for the selected airport and cycle")
    for name in pdf_names:
        if not re.fullmatch(r"[A-Za-z0-9_-]+\.PDF", name):
            raise ValueError("Invalid PDF name")
        with (source / name).open("rb") as stream:
            if stream.read(5) != b"%PDF-":
                raise ValueError(f"The selected file is not a PDF: {name}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent, prefix="procedures-") as temporary:
        target = Path(temporary) / release_id
        target.mkdir()
        charts = [{"id": f"{airport}-{name}", "airport": airport,
                   "name": records[name].findtext("chart_name"),
                   "chart_code": records[name].findtext("chart_code"), "artifact": name}
                  for name in sorted(pdf_names)]
        index = target / "procedures.json"
        index.write_text(json.dumps({"schema_version": 1, "edition": cycle, "charts": charts}, indent=2) + "\n")
        shutil.copyfile(metadata, target / "source-metafile.xml")
        for name in pdf_names:
            shutil.copyfile(source / name, target / name)
        artifacts = [{"path": path.name, "source": path.name,
                      "format": "pdf" if path.suffix == ".PDF" else "resource",
                      "bytes": path.stat().st_size, "sha256": digest(path)}
                     for path in sorted(target.iterdir())]
        release = {
            "schema_version": 1, "id": release_id, "product": "procedures", "authority": "FAA",
            "revision": 1, "edition": cycle, "source_set": digest(metadata),
            "channel": "development", "distribution": "unspecified",
            "validity": {"effective_at": effective_time(root.attrib["from_edate"]),
                         "expires_at": effective_time(root.attrib["to_edate"])},
            "coverage": {"name": f"{airport} selected procedure charts", "bounds": [-180, -90, 180, 90],
                         "min_zoom": 0, "max_zoom": 0, "complete": False,
                         "exclusions": ["Only the charts in procedures.json are included.",
                                        "This package contains PDF charts. It has no executable procedure legs."]},
            "artifacts": artifacts, "dependencies": [], "renderer_capabilities": ["procedure-pdf-v1"],
            "attributions": [f"FAA Digital Terminal Procedures Publication, cycle {cycle}",
                             f"https://aeronav.faa.gov/d-tpp/{cycle}/xml_data/d-tpp_Metafile.xml"],
        }
        (target / "release.json").write_text(json.dumps(release, indent=2) + "\n")
        target.rename(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("pdf_directory", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--id", required=True)
    parser.add_argument("--airport", required=True)
    parser.add_argument("--pdf", action="append", required=True)
    args = parser.parse_args()
    prepare(args.metadata, args.pdf_directory, args.output, args.id, args.airport, args.pdf)
    print(f"Prepared {args.id}")


if __name__ == "__main__":
    main()
