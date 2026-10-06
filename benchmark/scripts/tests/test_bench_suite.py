import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parents[1]))
import bench_suite

class SelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): cls.manifest = bench_suite.load_manifest()
    def test_format_default_is_the_corpus_and_the_3dbag_slice(self):
        selected = bench_suite.dataset_selection(self.manifest, ["formats"], "", "full")
        self.assertEqual(len(selected), 8)
        self.assertEqual(selected[-1], "3dbag_n1000000")
        self.assertIn("tokyo", selected)
        self.assertIn("montreal", selected)
    def test_the_3dbag_selector_names_the_one_slice(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["bloom"], "3dbag", "full"), ["3dbag_n1000000"])
    def test_the_manifest_holds_no_other_3dbag_dataset(self):
        slices = [key for key, entry in self.manifest["datasets"].items() if entry["role"] == "slice"]
        self.assertEqual(slices, ["3dbag_n1000000"])
    def test_read_repetitions_are_25_except_under_smoke(self):
        self.assertEqual(bench_suite.read_repeat("full"), 25)
        self.assertEqual(bench_suite.read_repeat("short"), 25)
        self.assertEqual(bench_suite.read_repeat("smoke"), 1)

    def test_smoke_selects_rotterdam_alone(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["formats", "bloom", "databases"], "", "smoke"), ["rotterdam"])
    def test_unknown_family_is_rejected(self):
        with self.assertRaises(SystemExit): bench_suite.family_selection("wrong")
    def test_unknown_dataset_is_rejected(self):
        with self.assertRaises(SystemExit): bench_suite.dataset_selection(self.manifest, ["formats"], "3dbag_n100000", "full")
    def test_paths_are_under_data_root(self):
        root = bench_suite.DEFAULT_DATA_ROOT
        self.assertEqual(bench_suite.paths(root)["prepared"], root / "data/readbench")
        self.assertEqual(bench_suite.paths(root)["3dbag"], root / "data/3dbag")
    def test_bloom_default_is_the_corpus_and_the_slice(self):
        selected = bench_suite.dataset_selection(self.manifest, ["bloom"], "", "full")
        self.assertEqual(selected, bench_suite.dataset_selection(self.manifest, ["formats"], "", "full"))
    def test_bloom_results_have_their_own_directory(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "bloom", "full").name, "bloom_results")
    def test_bloom_is_a_family(self):
        self.assertIn("bloom", bench_suite.family_selection("all"))
    def test_the_database_family_measures_the_slice_in_full(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["databases"], "", "full"), ["3dbag_n1000000"])
    def test_short_profile_drops_the_slice_and_measures_rotterdam_in_the_databases(self):
        formats = bench_suite.dataset_selection(self.manifest, ["formats"], "", "short")
        self.assertNotIn("3dbag_n1000000", formats)
        self.assertEqual(len(formats), 7)
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["databases"], "", "short"), ["rotterdam"])
    def test_short_profile_results_never_land_in_the_full_directories(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "formats", "short"), locations["formats"] / "short" / "results")
        self.assertEqual(bench_suite.result_dir(locations, "formats", "full"), locations["formats"] / "results")

class ProvenanceTests(unittest.TestCase):
    def test_the_run_manifest_hashes_the_result_and_its_sidecars(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "slice.city.jsonl"; source.write_text("source\n")
            result = root / "slice.csv"; result.write_text("dataset\n")
            Path(f"{result}.samples.json").write_text("[]\n")
            Path(f"{result}.params.json").write_text(json.dumps({"isolation": {"pinning": {"status": "not applied: not Linux"}}}))
            bench_suite.write_run_manifest(source, result, family="formats", repeat=1, smoke=True, fixed_configuration="test")
            manifest = json.loads(result.with_suffix(".run.json").read_text())
            self.assertEqual(set(manifest["result"]["files_sha256"]), {"slice.csv", "slice.csv.samples.json", "slice.csv.params.json"})
            self.assertEqual(manifest["measurement"], {"read_repeat": 1, "cell_budget_s": None, "min_repeat": 7, "fixed_configuration": "test",
                                                       "isolation": {"requested": bench_suite.DEFAULT_ISOLATION, "applied": {"pinning": {"status": "not applied: not Linux"}}}})

    def test_run_suite_forwards_the_isolation_to_the_read_recipes(self):
        from unittest import mock
        calls = []
        isolation = {"numa_node": "1", "memory_max": 8000000000, "max_load": "off", "max_load_wait_s": 60}
        with mock.patch.object(bench_suite, "just", lambda *a: calls.append(a)), \
             mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: Path("x.city.jsonl")), \
             mock.patch.object(bench_suite, "stage", lambda *a: Path("stage")), \
             mock.patch.object(bench_suite, "result_dir", lambda *a: Path("out")), \
             mock.patch.object(bench_suite, "write_run_manifest", lambda *a, **k: None):
            manifest = {"datasets": {"d": {"role": "corpus"}}}
            bench_suite.run_suite(manifest, {"prepared": Path("p"), "data": Path("d")}, ["formats", "bloom"], ["d"], "full", isolation=isolation)
        for call in calls:
            self.assertEqual(call[-4:], ("1", "8000000000", "off", "60"), call)


if __name__ == "__main__":
    unittest.main()


class SummaryStatisticTests(unittest.TestCase):
    def _summary_command(self, *extra):
        from unittest import mock
        calls = []
        argv = ["bench_suite.py", "summary", *extra]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(bench_suite, "command", lambda *a: calls.append(a)):
            bench_suite.main()
        (cmd,) = calls
        return list(cmd)

    def test_the_summary_plots_the_median_by_default(self):
        cmd = self._summary_command()
        self.assertEqual(cmd[cmd.index("--statistic") + 1], "median")

    def test_the_summary_forwards_the_mean(self):
        cmd = self._summary_command("--statistic", "mean")
        self.assertEqual(cmd[cmd.index("--statistic") + 1], "mean")
