"""Tests for bump-version.py:  python3 -m unittest discover -s .github/scripts"""

import importlib.util
import json
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "bump_version", Path(__file__).with_name("bump-version.py")
)
bv = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(bv)

MANIFEST = """\
[workspace]
members = ["crates/schema", "crates/core", "crates/cli"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
arrow-schema = { version = "58", features = ["canonical_extension_types"] }
tokio = { version = "1.53.1", features = ["rt"] }
cityparquet = { path = "crates/core", version = "0.1.0" }
cityparquet-schema = { path = "crates/schema", version = "0.1.0" }
city3d-stac-types = "0.1.1"
"""


class Bump(unittest.TestCase):
    def test_kinds(self):
        self.assertEqual(bv.bump("0.1.0", "patch"), "0.1.1")
        self.assertEqual(bv.bump("0.1.9", "minor"), "0.2.0")
        self.assertEqual(bv.bump("0.9.3", "major"), "1.0.0")
        self.assertEqual(bv.bump("0.1.0", "none"), "0.1.0")

    def test_refuses_a_prerelease(self):
        with self.assertRaises(ValueError):
            bv.bump("0.1.0-rc.1", "patch")

    def test_refuses_an_unknown_kind(self):
        with self.assertRaises(ValueError):
            bv.bump("0.1.0", "huge")


class RustManifest(unittest.TestCase):
    def test_sets_every_slot_and_nothing_else(self):
        text, version = bv.bump_rust_manifest(MANIFEST, "minor")
        self.assertEqual(version, "0.2.0")
        expected = (
            MANIFEST.replace('version = "0.1.0"\nedition', 'version = "0.2.0"\nedition')
            .replace('"crates/core", version = "0.1.0"', '"crates/core", version = "0.2.0"')
            .replace('"crates/schema", version = "0.1.0"', '"crates/schema", version = "0.2.0"')
        )
        self.assertEqual(text, expected)

    def test_none_changes_nothing(self):
        text, version = bv.bump_rust_manifest(MANIFEST, "none")
        self.assertEqual((text, version), (MANIFEST, "0.1.0"))

    def test_refuses_versions_that_disagree(self):
        drifted = MANIFEST.replace(
            '"crates/schema", version = "0.1.0"', '"crates/schema", version = "0.0.9"'
        )
        with self.assertRaisesRegex(ValueError, "disagree"):
            bv.bump_rust_manifest(drifted, "patch")

    def test_refuses_a_missing_slot(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            bv.bump_rust_manifest(MANIFEST.replace("cityparquet = {", "# gone = {"), "patch")

    def test_the_real_manifest_parses(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / bv.RUST_MANIFEST).read_text(encoding="utf-8")
        _, version = bv.bump_rust_manifest(text, "none")
        self.assertRegex(version, bv.VERSION)


class PackageJson(unittest.TestCase):
    def test_bumps_the_version_only(self):
        source = json.dumps({"name": "x", "version": "0.1.0", "bin": {"x": "a.js"}}, indent=2) + "\n"
        text, version = bv.bump_package_json(source, "patch")
        self.assertEqual(version, "0.1.1")
        self.assertEqual(json.loads(text), {"name": "x", "version": "0.1.1", "bin": {"x": "a.js"}})
        self.assertTrue(text.endswith("}\n"))

    def test_the_real_manifest_round_trips(self):
        root = Path(__file__).resolve().parents[2]
        source = (root / bv.MCP_MANIFEST).read_text(encoding="utf-8")
        text, _ = bv.bump_package_json(source, "none")
        self.assertEqual(text, source)


if __name__ == "__main__":
    unittest.main()
