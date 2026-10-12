"""Tests for ``compression_contribution.py``.

The classification tests import the module directly: DuckDB is imported only
when a package is read, so they run in benchmark/plot's environment like the
other script tests. The end-to-end tests read a real package that the
``cityparquet`` CLI writes from the real fixture ``lod3_railway.city.json``
(`just fixtures` in lib/cityparquet-rs fetches it; `cargo build --release -p
cityparquet-cli` there builds the CLI), and run the script the way the
justfile does, in benchmark/databases' uv environment, which provides DuckDB.
"""

import csv
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).parents[1]
REPO = Path(__file__).parents[3]
sys.path.insert(0, str(SCRIPTS))
import compression_contribution
from measure_sizes import artefact_bytes

RS = REPO / "lib/cityparquet-rs"
FIXTURE = RS / "tests/fixtures/lod3_railway.city.json"
CLI = Path(os.environ.get("CITYPARQUET_BIN", RS / "target/release/cityparquet"))


class GroupTests(unittest.TestCase):
    def test_reserved_columns_fall_into_their_groups(self):
        group = compression_contribution.column_group
        self.assertEqual(group("geometry_lod2_0"), "geometry")
        self.assertEqual(group("geometry_lod1_2"), "geometry")
        self.assertEqual(group("geometry_properties_lod2_0"), "geometry_properties")
        self.assertEqual(group("material_lod2_0"), "appearance")
        self.assertEqual(group("texture_lod3_0"), "appearance")
        for name in ("id", "feature_id", "object_type", "parents", "children", "children_roles"):
            self.assertEqual(group(name), "identifiers_structure", name)
        self.assertEqual(group("bbox"), "bbox")
        self.assertEqual(group("other"), "other")
        self.assertEqual(group("implicit_geometry"), "other")

    def test_every_other_column_is_an_attribute(self):
        group = compression_contribution.column_group
        for name in ("address", "b3_dak_type", "measuredHeight", "storeysAboveGround", "地区計画", "geometry_note"):
            self.assertEqual(group(name), "attributes", name)

    def test_a_nested_leaf_rolls_up_to_its_top_level_column(self):
        top = compression_contribution.top_level_column
        columns = ["address", "bbox", "geometry_properties_lod2_0", "geometry_properties_lod2_0_x"]
        self.assertEqual(top("address, list, item, city", columns), "address")
        self.assertEqual(top("bbox", columns), "bbox")
        self.assertEqual(top("geometry_properties_lod2_0, shells, list, item, list, item", columns), "geometry_properties_lod2_0")
        self.assertEqual(top("geometry_properties_lod2_0_x, type", columns), "geometry_properties_lod2_0_x")
        with self.assertRaises(ValueError):
            top("unknown, item", columns)


def run_script(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["uv", "run", "--quiet", "--project", str(REPO / "benchmark/databases"), "python", str(SCRIPTS / "compression_contribution.py"), *args],
        cwd=REPO,
        capture_output=True,
        text=True,
    )


def read_rows(path: Path) -> list[dict]:
    with path.open(newline="") as stream:
        return list(csv.DictReader(stream))


class PackageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not FIXTURE.exists():
            raise AssertionError(f"missing fixture {FIXTURE}; run `just fixtures` in lib/cityparquet-rs")
        if not CLI.exists():
            raise AssertionError(f"missing CLI {CLI}; run `cargo build --release -p cityparquet-cli` in lib/cityparquet-rs")
        cls.tmp = tempfile.TemporaryDirectory()
        cls.prepared = Path(cls.tmp.name) / "prepared"
        cls.package = cls.prepared / "railway.parquet"
        subprocess.run([str(CLI), "convert", str(FIXTURE), "-o", str(cls.package)], check=True, capture_output=True)
        cls.out = Path(cls.tmp.name) / "out"
        cls.result = run_script("--prepared", str(cls.prepared), "--out", str(cls.out))
        if cls.result.returncode != 0:
            raise AssertionError(cls.result.stderr)
        cls.rows = read_rows(cls.out / compression_contribution.OUTPUT_NAME)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def level(self, level: str) -> list[dict]:
        return [row for row in self.rows if row["level"] == level]

    def test_header_contract(self):
        with (self.out / compression_contribution.OUTPUT_NAME).open(newline="") as stream:
            self.assertEqual(next(csv.reader(stream)), compression_contribution.HEADER)

    def test_the_parts_sum_to_the_directory_size(self):
        columns = sum(int(row["compressed_bytes"]) for row in self.level("group"))
        parts = sum(int(row["compressed_bytes"]) for row in self.level("part"))
        (package,) = self.level("package")
        self.assertEqual(columns + parts, artefact_bytes(self.package))
        self.assertEqual(int(package["compressed_bytes"]), artefact_bytes(self.package))

    def test_the_columns_sum_to_their_groups(self):
        for key in ("compressed_bytes", "uncompressed_bytes"):
            groups = sum(int(row[key]) for row in self.level("group"))
            columns = sum(int(row[key]) for row in self.level("column"))
            self.assertEqual(groups, columns, key)
        shares = sum(float(row["share_of_column_bytes"]) for row in self.level("group"))
        self.assertAlmostEqual(shares, 1.0, places=4)

    def test_every_object_table_is_read_and_the_sidecars_are_a_part(self):
        names = {row["name"] for row in self.level("part")}
        self.assertEqual(names, {"footer_and_page_indexes", "bloom_filters", "sidecar_tables", "metadata", "other_files"})
        sidecars = sum((self.package / name).stat().st_size for name in ("materials.parquet", "textures.parquet", "implicit_geometries.parquet"))
        (row,) = [row for row in self.level("part") if row["name"] == "sidecar_tables"]
        self.assertEqual(int(row["compressed_bytes"]), sidecars)
        geometry = [row for row in self.level("column") if row["group"] == "geometry"]
        self.assertTrue(geometry)
        self.assertTrue(all(row["name"].startswith("geometry_lod") for row in geometry))

    def test_a_column_row_carries_its_encodings(self):
        (row,) = [row for row in self.level("column") if row["name"] == "object_type"]
        self.assertIn("RLE_DICTIONARY", row["encodings"].split("|"))
        self.assertGreater(float(row["compression_ratio"]), 0)

    def test_a_missing_package_is_reported_never_zero(self):
        out = Path(self.tmp.name) / "missing"
        result = run_script("--prepared", str(self.prepared), "--out", str(out), "--dataset", "railway", "--dataset", "ghost")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("ghost", result.stderr)
        ghost = [row for row in read_rows(out / compression_contribution.OUTPUT_NAME) if row["dataset"] == "ghost"]
        self.assertEqual(len(ghost), 1)
        self.assertEqual(ghost[0]["status"], "missing")
        self.assertEqual(ghost[0]["compressed_bytes"], "")


if __name__ == "__main__":
    unittest.main()
