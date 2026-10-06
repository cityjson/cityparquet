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
    def test_quick_profile_measures_the_full_dataset_set_at_7_repetitions(self):
        for families in (["formats"], ["bloom"], ["databases"]):
            self.assertEqual(bench_suite.dataset_selection(self.manifest, families, "", "quick"),
                             bench_suite.dataset_selection(self.manifest, families, "", "full"))
        self.assertEqual(bench_suite.read_repeat("quick"), 7)
    def test_quick_profile_results_have_their_own_directories(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.result_dir(locations, "formats", "quick"), locations["formats"] / "quick" / "results")
        self.assertEqual(bench_suite.result_dir(locations, "bloom", "quick"), locations["formats"] / "quick" / "bloom_results")

class MemoryCeilingTests(unittest.TestCase):
    def test_the_default_ceiling_is_64_decimal_gigabytes_except_under_smoke(self):
        for profile in ("full", "quick", "short"):
            self.assertEqual(bench_suite.memory_ceiling(None, profile), 64_000_000_000)
        self.assertIsNone(bench_suite.memory_ceiling(None, "smoke"))
    def test_the_ceiling_can_be_overridden_or_switched_off(self):
        self.assertEqual(bench_suite.memory_ceiling("8000000000", "full"), 8_000_000_000)
        self.assertIsNone(bench_suite.memory_ceiling("off", "full"))
        with self.assertRaises(SystemExit): bench_suite.memory_ceiling("64G", "full")

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

    def _database_command(self, profile, memory_max):
        from unittest import mock
        commands = []
        isolation = {"numa_node": "auto", "memory_max": memory_max, "max_load": "auto", "max_load_wait_s": 600}
        root = Path("/runs")
        with mock.patch.object(bench_suite, "command", lambda *a: commands.append(list(a))), \
             mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: Path(__file__)):
            manifest = {"datasets": {"s": {"role": "slice"}}, "suite": {"slice_dataset": "s", "small_database_dataset": "r"}}
            locations = {"prepared": root / "data/readbench", "formats": root / "formats", "databases": root / "databases"}
            bench_suite.run_suite(manifest, locations, ["databases"], ["s"], profile, isolation=isolation)
        (cmd,) = commands
        return cmd

    def test_the_database_family_runs_the_profiles_repetitions_into_its_own_directory(self):
        cmd = self._database_command("quick", 64_000_000_000)
        self.assertEqual(cmd[cmd.index("--repeat") + 1], "7")
        self.assertEqual(cmd[cmd.index("--output-dir") + 1], "/runs/databases/quick")
        self.assertEqual(self._database_command("full", None)[self._database_command("full", None).index("--repeat") + 1], "25")

    def test_the_database_family_records_the_memory_ceiling(self):
        cmd = self._database_command("full", 64_000_000_000)
        self.assertEqual(cmd[cmd.index("--memory-max") + 1], "64000000000")
        self.assertNotIn("--memory-max", self._database_command("full", None))

    def test_an_off_ceiling_reaches_the_read_recipes_as_off(self):
        from unittest import mock
        calls = []
        with mock.patch.object(bench_suite, "just", lambda *a: calls.append(a)), \
             mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: Path("x.city.jsonl")), \
             mock.patch.object(bench_suite, "stage", lambda *a: Path("stage")), \
             mock.patch.object(bench_suite, "result_dir", lambda *a: Path("out")), \
             mock.patch.object(bench_suite, "write_run_manifest", lambda *a, **k: None):
            bench_suite.run_suite({"datasets": {"d": {"role": "corpus"}}}, {"prepared": Path("p")}, ["formats"], ["d"], "full",
                                  isolation={"numa_node": "auto", "memory_max": None, "max_load": "auto", "max_load_wait_s": 600})
        self.assertEqual(calls[0][-3], "off")


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


class CompressionTests(unittest.TestCase):
    def test_the_suite_names_every_dataset_for_the_compression_script(self):
        inputs = [Path("/corpus/tokyo.city.json"), Path("/3dbag/3dbag_n1000000.city.jsonl")]
        argv = bench_suite.compression_command(inputs, Path("/prepared"), Path("/out"))
        self.assertEqual(argv[:5], ["uv", "run", "--project", "benchmark/databases", "python"])
        self.assertEqual(argv[5], "benchmark/scripts/compression_contribution.py")
        datasets = [argv[index + 1] for index, item in enumerate(argv) if item == "--dataset"]
        self.assertEqual(datasets, ["tokyo", "3dbag_n1000000"])
        self.assertEqual(argv[argv.index("--prepared") + 1], "/prepared")
        self.assertEqual(argv[argv.index("--out") + 1], "/out")

    def test_the_summary_places_the_breakdown_beside_the_size_factors(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            results = Path(tmp) / "results"
            results.mkdir()
            (results / "compression.csv").write_text("dataset\n", encoding="utf-8")
            figures = Path(tmp) / "figures"
            placed = bench_suite.place_compression_table(results, figures)
            self.assertEqual(placed, figures / "formats" / "compression.csv")
            self.assertEqual(placed.read_text(encoding="utf-8"), "dataset\n")

    def test_the_summary_skips_a_breakdown_that_was_not_measured(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            self.assertIsNone(bench_suite.place_compression_table(Path(tmp) / "results", Path(tmp) / "figures"))
            self.assertFalse((Path(tmp) / "figures").exists())


class SizesCommandTests(unittest.TestCase):
    def test_the_suite_names_the_dataset_for_the_size_script(self):
        argv = bench_suite.sizes_command(Path("/corpus/tokyo.city.json"), Path("/prepared"), Path("/out/sizes.csv"))
        self.assertEqual(argv[1], "benchmark/scripts/measure_sizes.py")
        self.assertEqual(argv[argv.index("--dataset") + 1], "tokyo")
        self.assertEqual(argv[argv.index("--prepared") + 1], "/prepared")
        self.assertEqual(argv[argv.index("--out") + 1], "/out/sizes.csv")
        self.assertNotIn("--input", argv)
