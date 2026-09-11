#!/usr/bin/env python3
"""Check immutable geographic release preparation."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("prepare-geographic-release.py")


class GeographicReleaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.archive = self.root / "source.mbtiles"
        self.archive.write_bytes(b"archive bytes")
        self.manifest = self.root / "source.json"
        self.manifest.write_text(json.dumps({
            "archive_bytes": self.archive.stat().st_size,
            "archive_sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "bands": [{"min_zoom": 0, "max_zoom": 5}, {"min_zoom": 6, "max_zoom": 10}],
            "attribution": "Example geographic source",
        }))
        self.output = self.root / "release"

    def run_preparation(self, identifier="base-example", revision=1):
        return subprocess.run([
            sys.executable, str(SCRIPT), str(self.archive), str(self.manifest), str(self.output),
            "--id", identifier, "--product", "basemap", "--authority", "Example", "--edition", "1", "--revision", str(revision),
        ], capture_output=True, text=True, check=False)

    def test_release_records_exact_bytes_and_partial_coverage(self):
        result = self.run_preparation()
        self.assertEqual(result.returncode, 0, result.stderr)
        release = json.loads((self.output / "release.json").read_text())
        self.assertEqual(release["channel"], "development")
        self.assertEqual(release["distribution"], "unspecified")
        self.assertEqual(release["coverage"]["max_zoom"], 10)
        self.assertFalse(release["coverage"]["complete"])
        self.assertEqual(release["attributions"], ["Example geographic source"])
        for artifact in release["artifacts"]:
            data = (self.output / artifact["path"]).read_bytes()
            self.assertEqual(len(data), artifact["bytes"])
            self.assertEqual(hashlib.sha256(data).hexdigest(), artifact["sha256"])

    def test_release_revision_is_recorded(self):
        result = self.run_preparation(revision=2)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads((self.output / "release.json").read_text())["revision"], 2)

    def test_release_revision_must_be_positive(self):
        self.assertNotEqual(self.run_preparation(revision=0).returncode, 0)
        self.assertFalse(self.output.exists())

    def test_changed_source_cannot_create_a_release(self):
        self.archive.write_bytes(b"altered bytes")
        self.assertNotEqual(self.run_preparation().returncode, 0)
        self.assertFalse(self.output.exists())

    def test_existing_release_cannot_be_replaced(self):
        self.assertEqual(self.run_preparation().returncode, 0)
        before = (self.output / "release.json").read_bytes()
        self.assertNotEqual(self.run_preparation().returncode, 0)
        self.assertEqual((self.output / "release.json").read_bytes(), before)

    def test_invalid_identity_cannot_escape_the_output(self):
        self.assertNotEqual(self.run_preparation("../outside").returncode, 0)
        self.assertFalse(self.output.exists())
        self.assertFalse((self.root / "outside").exists())


if __name__ == "__main__":
    unittest.main()
