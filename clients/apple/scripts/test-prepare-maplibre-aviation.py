#!/usr/bin/env python3
"""Check the source and resource guards used by the native renderer build."""

import difflib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("prepare-maplibre-aviation.py")
SPEC = importlib.util.spec_from_file_location("aviation_renderer", SCRIPT)
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class PreparationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.source, self.package = root / "source", root / "package"
        self.project, self.ifr = root / "project", root / "ifr"
        self.package.mkdir()
        (self.source / "apple/visionos").mkdir(parents=True)
        (self.project / "clients/apple").mkdir(parents=True)
        (self.project / "clients/apple/MAPLIBRE_GLOBE_REVISION").write_text("fixture\n")
        old, new = "[dependencies]\n", '[dependencies]\npilotage-map-archives = { path = "@PILOTAGE_MAP_ARCHIVES@" }\n'
        (self.source / "apple/visionos/Cargo.toml").write_text(old)
        patch = "".join(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                          fromfile="a/apple/visionos/Cargo.toml", tofile="b/apple/visionos/Cargo.toml"))
        (self.package / "change.patch").write_text(patch)
        (self.package / "Cargo.lock").write_text("fixture lock\n")
        resource = self.ifr / "tools/maplibre-rs-web/glyphs/test.pbf"
        resource.parent.mkdir(parents=True)
        resource.write_bytes(b"glyph bytes")
        self.config = {
            "schema_version": 1, "renderer_revision": "fixture",
            "patch": {"path": "change.patch", "sha256": PREPARE.digest(patch.encode())},
            "lock": {"path": "Cargo.lock", "sha256": PREPARE.digest(b"fixture lock\n")},
            "files": [{"path": "apple/visionos/Cargo.toml", "before_sha256": PREPARE.digest(old.encode()),
                       "after_sha256": PREPARE.digest(new.encode())}],
            "resources": [{"path": "glyphs/test.pbf", "target": "data/test.pbf",
                           "sha256": PREPARE.digest(b"glyph bytes"), "output_sha256": PREPARE.digest(b"glyph bytes")}],
        }
        (self.package / "build.json").write_text(json.dumps(self.config))

    def prepare(self, verify=False):
        return PREPARE.prepare_blocking(self.source, self.ifr, self.package, self.project, verify)

    def test_clean_source_is_prepared_and_repeat_runs_only_verify(self):
        self.assertEqual(self.prepare(), "prepared")
        self.assertEqual(self.prepare(True), "verified")
        self.assertEqual(self.prepare(), "verified")
        manifest = (self.source / "apple/visionos/Cargo.toml").read_text()
        self.assertIn(str(self.project / "crates/pilotage-map-archives"), manifest)
        self.assertEqual((self.source / "data/test.pbf").read_bytes(), b"glyph bytes")

    def test_changed_base_is_rejected_before_resources_are_written(self):
        path = self.source / "apple/visionos/Cargo.toml"
        path.write_text("user change\n")
        with self.assertRaisesRegex(ValueError, "does not match the base"):
            self.prepare()
        self.assertEqual(path.read_text(), "user change\n")
        self.assertFalse((self.source / "data/test.pbf").exists())

    def test_bad_input_resource_cannot_partially_apply_the_patch(self):
        (self.ifr / "tools/maplibre-rs-web/glyphs/test.pbf").write_bytes(b"different")
        with self.assertRaisesRegex(ValueError, "resource does not match"):
            self.prepare()
        self.assertEqual((self.source / "apple/visionos/Cargo.toml").read_text(), "[dependencies]\n")

    def test_changed_installed_resource_and_lock_fail_verification(self):
        self.prepare()
        resource = self.source / "data/test.pbf"
        resource.write_bytes(b"changed")
        with self.assertRaisesRegex(ValueError, "resource does not match"):
            self.prepare(True)
        resource.write_bytes(b"glyph bytes")
        (self.source / "apple/visionos/Cargo.lock").write_text("changed")
        with self.assertRaisesRegex(ValueError, "lock file does not match"):
            self.prepare(True)


if __name__ == "__main__":
    unittest.main()
