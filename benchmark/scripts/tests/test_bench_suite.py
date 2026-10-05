import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parents[1]))
import bench_suite

class SelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.manifest = bench_suite.load_manifest()
    def test_format_default_replaces_small_3dbag_with_largest_prefix(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["formats"], "", "full")[-1], "3dbag_n1000000")
    def test_smoke_3dbag_selects_only_small_prefix(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["bloom"], "3dbag", "smoke"), ["3dbag_n1000"])
    def test_unknown_family_is_rejected(self):
        with self.assertRaises(SystemExit): bench_suite.family_selection("wrong")
    def test_paths_are_under_data_root(self):
        root = bench_suite.DEFAULT_DATA_ROOT
        self.assertEqual(bench_suite.paths(root)["prepared"], root / "data/readbench")
    def test_bloom_default_is_the_scaling_series_and_the_corpus(self):
        selected = bench_suite.dataset_selection(self.manifest, ["bloom"], "", "full")
        self.assertEqual(len(selected), 12)
        self.assertIn("rotterdam", selected)
        self.assertIn("3dbag_n1000000", selected)
    def test_bloom_results_have_their_own_directory(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "bloom", "full").name, "scaling_bloom_results")
    def test_bloom_is_a_family(self):
        self.assertIn("bloom", bench_suite.family_selection("all"))
    def test_short_profile_stands_the_100k_slice_in_for_the_largest(self):
        formats = bench_suite.dataset_selection(self.manifest, ["formats"], "", "short")
        self.assertEqual(formats[-1], "3dbag_n100000")
        self.assertNotIn("3dbag_n1000000", formats)
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["databases"], "", "short"), ["3dbag_n100000"])
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["formats"], "largest", "short"), ["3dbag_n100000"])
    def test_short_profile_caps_the_bloom_scaling_series(self):
        selected = bench_suite.dataset_selection(self.manifest, ["bloom"], "", "short")
        self.assertIn("3dbag_n100000", selected)
        self.assertNotIn("3dbag_n500000", selected)
        self.assertNotIn("3dbag_n1000000", selected)
    def test_short_profile_results_never_land_in_the_full_directories(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "formats", "short"), locations["formats"] / "short" / "results")
        self.assertEqual(bench_suite.result_dir(locations, "formats", "full"), locations["formats"] / "results")
if __name__ == "__main__": unittest.main()

class ProvenanceTests(unittest.TestCase):
    def test_the_run_manifest_hashes_the_result_and_its_sidecars(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "slice.city.jsonl"; source.write_text("source\n")
            result = root / "slice.csv"; result.write_text("dataset\n")
            Path(f"{result}.samples.json").write_text("[]\n")
            Path(f"{result}.params.json").write_text("{}\n")
            bench_suite.write_run_manifest(source, result, family="formats", repeat=1, smoke=True, fixed_configuration="test")
            manifest = json.loads(result.with_suffix(".run.json").read_text())
            self.assertEqual(set(manifest["result"]["files_sha256"]), {"slice.csv", "slice.csv.samples.json", "slice.csv.params.json"})
            self.assertEqual(manifest["measurement"], {"read_repeat": 1, "fixed_configuration": "test"})
