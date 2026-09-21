import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parents[1]))
import bench_suite

class SelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.manifest = bench_suite.load_manifest()
    def test_format_default_replaces_small_3dbag_with_largest_prefix(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["formats"], "", False)[-1], "3dbag_n1000000")
    def test_configuration_default_is_the_entire_scaling_series(self):
        self.assertEqual(len(bench_suite.dataset_selection(self.manifest, ["codec"], "", False)), 7)
    def test_smoke_3dbag_selects_only_small_prefix(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["codec"], "3dbag", True), ["3dbag_n1000"])
    def test_unknown_family_is_rejected(self):
        with self.assertRaises(SystemExit): bench_suite.family_selection("wrong")
    def test_paths_are_under_data_root(self):
        root = bench_suite.DEFAULT_DATA_ROOT
        self.assertEqual(bench_suite.paths(root)["prepared"], root / "data/readbench")
if __name__ == "__main__": unittest.main()

class ProvenanceTests(unittest.TestCase):
    def test_per_dataset_write_samples_keep_manifest_hash_stable(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "slice.city.jsonl"; source.write_text("source\n")
            result = root / "slice.csv"; result.write_text("dataset\n")
            Path(f"{result}.samples.json").write_text("[]\n")
            Path(f"{result}.params.json").write_text("{}\n")
            own = result.with_suffix(".write.samples.csv"); own.write_text("own\n")
            shared = root / "write.samples.csv"; shared.write_text("first\n")
            bench_suite.write_run_manifest(source, result, family="formats", repeat=1, write_repeat=1, smoke=True, fixed_configuration="test")
            first = json.loads(result.with_suffix(".run.json").read_text())
            shared.write_text("first\nsecond\n")
            bench_suite.write_run_manifest(source, result, family="formats", repeat=1, write_repeat=1, smoke=True, fixed_configuration="test")
            second = json.loads(result.with_suffix(".run.json").read_text())
            self.assertEqual(first["result"]["files_sha256"], second["result"]["files_sha256"])
            self.assertIn("slice.write.samples.csv", second["result"]["files_sha256"])
            self.assertNotIn("write.samples.csv", {name for name in second["result"]["files_sha256"] if name == "write.samples.csv"})
