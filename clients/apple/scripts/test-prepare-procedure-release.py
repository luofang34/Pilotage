#!/usr/bin/env python3
"""Check procedure edition and artifact boundaries."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("prepare", Path(__file__).with_name("prepare-procedure-release.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProcedureReleaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.metadata = self.root / "metadata.xml"
        self.metadata.write_text('''<digital_tpp cycle="2609" from_edate="0901Z  09/03/26" to_edate="0901Z  10/01/26">
          <airport_name icao_ident="KTTN"><record><pdf_name>CHART.PDF</pdf_name>
          <chart_name>ILS RWY 06</chart_name><chart_code>IAP</chart_code></record></airport_name></digital_tpp>''')
        (self.root / "CHART.PDF").write_bytes(b"%PDF-1.7\nexample")
        self.output = self.root / "release"

    def prepare(self, names=None, airport="KTTN"):
        MODULE.prepare(self.metadata, self.root, self.output, "procedures-2609", airport, names or ["CHART.PDF"])

    def test_exact_source_bytes_and_faa_utc_interval_are_retained(self):
        self.prepare()
        release = json.loads((self.output / "release.json").read_text())
        self.assertEqual(release["edition"], "2609")
        self.assertEqual(release["validity"], {"effective_at": 1788426060, "expires_at": 1790845260})
        self.assertFalse(release["coverage"]["complete"])
        for artifact in release["artifacts"]:
            payload = (self.output / artifact["path"]).read_bytes()
            self.assertEqual(artifact["sha256"], hashlib.sha256(payload).hexdigest())
            self.assertEqual(artifact["bytes"], len(payload))

    def test_pdf_from_another_airport_cannot_enter_the_release(self):
        with self.assertRaises(ValueError):
            self.prepare(airport="KJFK")
        self.assertFalse(self.output.exists())

    def test_missing_and_duplicate_records_cannot_enter_the_release(self):
        for names in [["OTHER.PDF"], ["CHART.PDF", "CHART.PDF"]]:
            with self.assertRaises(ValueError):
                self.prepare(names)
            self.assertFalse(self.output.exists())

    def test_download_error_page_is_rejected(self):
        (self.root / "CHART.PDF").write_bytes(b"<html>Not found</html>")
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse(self.output.exists())

    def test_published_directory_cannot_be_replaced(self):
        self.prepare()
        with self.assertRaises(ValueError):
            self.prepare()


if __name__ == "__main__":
    unittest.main()
