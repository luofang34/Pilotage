#!/usr/bin/env python3
"""Check archive admission and complete local chart releases."""

import argparse
import importlib.util
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-ifr-releases.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class Releases(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.producer = root / "producer"
        self.styles = root / "styles"
        self.producer.mkdir()
        self.styles.mkdir()
        profiles = {}
        for name, layer in [("network", "airways"), ("detail", "labels")]:
            metadata = json.dumps({"vector_layers": [{"id": layer}]}).encode()
            header = bytearray(127)
            header[:8] = b"PMTiles\x03"
            struct.pack_into("<QQ", header, 24, 127, len(metadata))
            header[97], header[100], header[101] = 1, 3, 8
            struct.pack_into("<iiii", header, 102, -1800000000, -850000000, 1800000000, 850000000)
            path = self.producer / f"{name}.pmtiles"
            path.write_bytes(header + metadata)
            profiles[name] = {"archive": path.name, "report": {"artifact": {
                "bytes": path.stat().st_size, "sha256": prepare.digest(path)}}}
        prepare.write_json(self.producer / "manifest.json", {
            "effective": "2026/09/03", "delivery": {"profiles": profiles}})
        for suffix in ["png", "json"]:
            (self.producer / f"point-sprites.{suffix}").write_text("test")
        for profile in ["low", "high"]:
            prepare.write_json(self.styles / f"{profile}-style.json", {
                "metadata": {"edition": "style-date"}, "sources": {"chart": {}},
                "layers": [{"id": "route", "source": "chart", "source-layer": "airways"},
                           {"id": "label", "source": "chart", "source-layer": "labels"}]})
        self.args = argparse.Namespace(producer=self.producer, styles=self.styles, output=root / "output",
                                       edition="2609", expires="2026-10-01", revision=1,
                                       ifr_source=root, reference_latitude=40.5)
        for name in ["chart-view.js", "paper-scale.js", "../raster-reference.js"]:
            path = root / "web/rust" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("test")
        mock = patch.object(prepare.subprocess, "run", side_effect=lambda args, **kwargs:
                            argparse.Namespace(stdout=Path(args[-1]).read_text()))
        mock.start()
        self.addCleanup(mock.stop)

    def test_styles_resolve_both_archives_and_keep_shared_bytes(self):
        ids = prepare.build(self.args)
        releases = [json.loads((self.args.output / name / "release.json").read_text()) for name in ids]
        self.assertEqual([release["product"] for release in releases], ["ifr_low", "ifr_high"])
        self.assertEqual(releases[0]["artifacts"][0]["sha256"], releases[1]["artifacts"][0]["sha256"])
        style = json.loads((self.args.output / ids[0] / "style.json").read_text())
        self.assertEqual([layer["source"] for layer in style["layers"]], ["network", "detail"])
        self.assertEqual(style["metadata"]["edition"], "style-date")
        self.assertNotIn("pilotage:resources", style["metadata"])
        self.assertEqual(releases[0]["validity"], {"effective_at": 1788426060, "expires_at": 1790845260})

    def test_changed_archive_is_rejected_without_output(self):
        (self.producer / "network.pmtiles").write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "does not match"):
            prepare.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_missing_style_layer_is_rejected_without_partial_release(self):
        path = self.styles / "high-style.json"
        style = json.loads(path.read_text())
        style["layers"][0]["source-layer"] = "absent"
        prepare.write_json(path, style)
        with self.assertRaisesRegex(ValueError, "unique archive"):
            prepare.build(self.args)
        self.assertFalse(self.args.output.exists())

    def test_existing_release_output_is_immutable(self):
        prepare.build(self.args)
        with self.assertRaisesRegex(ValueError, "already exists"):
            prepare.build(self.args)


if __name__ == "__main__":
    unittest.main()
