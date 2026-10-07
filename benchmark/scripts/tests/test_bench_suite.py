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
        self.assertEqual(len(selected), 7)
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
    def test_bloom_default_is_the_slice_alone(self):
        # A filter rules out whole row groups; only the slice has enough of them.
        for profile in ("full", "quick"):
            self.assertEqual(bench_suite.dataset_selection(self.manifest, ["bloom"], "", profile), ["3dbag_n1000000"])
    def test_profiles_without_the_slice_select_no_bloom_input(self):
        self.assertEqual(bench_suite.dataset_selection(self.manifest, ["bloom"], "", "short"), [])
    def test_bloom_runs_on_the_slice_and_skips_corpus_inputs(self):
        from unittest import mock
        calls = []
        with mock.patch.object(bench_suite, "just", lambda *a: calls.append(a)), \
             mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: Path(entry["file"])), \
             mock.patch.object(bench_suite, "stage", lambda locations, name, inputs: Path("+".join(str(i) for i in inputs))), \
             mock.patch.object(bench_suite, "result_dir", lambda *a: Path("out")), \
             mock.patch.object(bench_suite, "write_run_manifest", lambda *a, **k: None):
            manifest = {"datasets": {"c": {"role": "corpus", "file": "c.city.json"}, "s": {"role": "slice", "file": "s.city.jsonl"}}}
            bench_suite.run_suite(manifest, {"prepared": Path("p"), "data": Path("d")}, ["bloom"], ["c", "s"], "full")
            self.assertEqual([call[1] for call in calls if call[0] == "bloom-bench"], ["s.city.jsonl"])
            calls.clear()
            bench_suite.run_suite(manifest, {"prepared": Path("p"), "data": Path("d")}, ["bloom"], ["c"], "smoke")
            self.assertEqual([call for call in calls if call[0] == "bloom-bench"], [])
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
        self.assertEqual(len(formats), 6)
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

class VariantArtefactTests(unittest.TestCase):
    def test_the_bloom_axis_dataset_carries_the_nobloom_package(self):
        manifest = tomllib_load()
        entry = manifest["datasets"][manifest["suite"]["slice_dataset"]]
        self.assertEqual(bench_suite.variant_artefacts(entry), ["cityparquet-nobloom"])
        self.assertEqual(bench_suite.variant_artefacts(manifest["datasets"]["rotterdam"]), [])

    def test_variant_id_is_the_local_infix(self):
        self.assertEqual(bench_suite.variant_id("cityparquet-nobloom"), "cityparquet+nobloom")


def tomllib_load():
    import tomllib
    with open(bench_suite.MANIFEST, "rb") as handle:
        return tomllib.load(handle)


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
            origin = {"origin": "bucket", "manifest_url": "https://h/v7/manifest.json", "manifest_sha256": "abc"}
            bench_suite.write_run_manifest(source, result, family="formats", repeat=1, smoke=True, fixed_configuration="test", corpus=origin)
            manifest = json.loads(result.with_suffix(".run.json").read_text())
            self.assertEqual(manifest["corpus"], origin)
            self.assertEqual(set(manifest["result"]["files_sha256"]), {"slice.csv", "slice.csv.samples.json", "slice.csv.params.json"})
            self.assertEqual(manifest["measurement"], {"read_repeat": 1, "cell_budget_s": None, "min_repeat": 7, "fixed_configuration": "test",
                                                       "isolation": {"requested": bench_suite.DEFAULT_ISOLATION, "applied": {"pinning": {"status": "not applied: not Linux"}}}})

    def test_a_network_run_manifest_carries_the_network_record(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "r.city.jsonl"; source.write_text("source\n")
            result = root / "r.csv"; result.write_text("dataset\n")
            network = {"target": "real", "snapshot": "real object storage, one snapshot: one network path at one time",
                       "base_url": "https://h/v8", "host": "h", "resolved_ips": ["192.0.2.1"],
                       "objects": [{"url": "https://h/v8/citygml/r.gml", "first": {"status": 206, "cf_cache_status": "HIT", "age": "3"}, "last": {"status": 206, "cf_cache_status": "HIT", "age": "9"}}]}
            Path(f"{result}.params.json").write_text(json.dumps({"network": network}))
            bench_suite.write_run_manifest(source, result, family="network", repeat=1, smoke=True, fixed_configuration="network=real; target=real")
            self.assertEqual(json.loads(result.with_suffix(".run.json").read_text())["network"], network)
            other = root / "f.csv"; other.write_text("dataset\n")
            bench_suite.write_run_manifest(source, other, family="formats", repeat=1, smoke=True, fixed_configuration="test")
            self.assertNotIn("network", json.loads(other.with_suffix(".run.json").read_text()))

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


class PrepModeTest(unittest.TestCase):
    """`bench-prep`'s modes: download (default), --no-cache, --rebuild-sources, --local."""

    def setUp(self):
        import tempfile
        from unittest import mock
        self.mock = mock
        self.root = Path(tempfile.mkdtemp())
        self.locations = bench_suite.paths(self.root)
        self.manifest = {"datasets": {"rotterdam": {"role": "corpus", "source": "rotterdam_delfshaven.city.json"}}}
        self.calls = []

    def run_prep(self, mode, force=False, families=("formats",)):
        mock = self.mock
        bucket = bench_suite.corpus_bucket
        hosted = {"schema": 1, "chain_version": 7, "datasets": {"rotterdam_delfshaven": {"source": {"url": "u", "sha256": "s"}, "artefacts": {}}}}
        def fake_download(cfg, chain, manifest, dataset_id, artefacts, prepared):
            self.calls.append(("download", dataset_id, tuple(artefacts)))
            for artefact in artefacts:
                (prepared / bucket.local_name(artefact, dataset_id)).parent.mkdir(parents=True, exist_ok=True)
                (prepared / bucket.local_name(artefact, dataset_id)).write_text(artefact)
            return []
        with mock.patch.object(bench_suite, "just", lambda *a: self.calls.append(("just",) + a)), \
             mock.patch.object(bench_suite, "command", lambda *a: self.calls.append(("command",) + a)), \
             mock.patch.object(bucket, "fetch_manifest", lambda cfg, chain: (hosted, "https://h/v7/manifest.json", "abc")), \
             mock.patch.object(bucket, "download_dataset", fake_download), \
             mock.patch.object(bucket, "dataset_entry", lambda prepared, dataset_id, artefacts, source, built: {"artefacts": list(artefacts), "source": source}), \
             mock.patch.object(bucket, "remote_manifest", lambda cfg, chain: hosted), \
             mock.patch.object(bucket, "upload_dataset", lambda cfg, chain, prepared, entry, dataset_id, existing, force=False: self.calls.append(("upload", dataset_id, tuple(entry["artefacts"]), force, entry["source"]["url"])) or {"uploaded": [], "skipped": []}), \
             mock.patch.object(bucket, "publish_manifest", lambda cfg, chain, entries: self.calls.append(("publish", sorted(entries)))), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: self.root / entry["source"]), \
             mock.patch.object(bench_suite, "fetched_source", lambda entry, path: {"url": "published", "sha256": "x"}):
            (self.root / "rotterdam_delfshaven.city.json").write_text("{}")
            bench_suite.prepare(self.manifest, self.locations, list(families), ["rotterdam"], "full", mode=mode, force_upload=force)

    def provenance(self):
        import json
        return json.loads((self.locations["prepared"] / bench_suite.CORPUS_ORIGIN).read_text())

    def test_smoke_and_local_flag_prepare_locally_and_plain_prep_downloads(self):
        self.assertEqual(bench_suite.prep_mode(local=False, no_cache=False, rebuild_sources=False, profile="smoke"), "local")
        self.assertEqual(bench_suite.prep_mode(local=True, no_cache=False, rebuild_sources=False, profile="full"), "local")
        self.assertEqual(bench_suite.prep_mode(local=False, no_cache=False, rebuild_sources=False, profile="full"), "download")
        self.assertEqual(bench_suite.prep_mode(local=False, no_cache=True, rebuild_sources=False, profile="full"), "no-cache")
        self.assertEqual(bench_suite.prep_mode(local=False, no_cache=False, rebuild_sources=True, profile="full"), "rebuild-sources")
        with self.assertRaises(SystemExit):
            bench_suite.prep_mode(local=True, no_cache=True, rebuild_sources=False, profile="full")

    def test_download_builds_nothing_and_records_the_hosted_manifest(self):
        self.run_prep("download")
        self.assertEqual([c for c in self.calls if c[0] in ("just", "command")], [])
        self.assertEqual(self.calls, [("download", "rotterdam_delfshaven", ("citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet"))])
        self.assertEqual(self.provenance()["origin"], "bucket")
        self.assertEqual(self.provenance()["manifest_sha256"], "abc")
        stamp = self.locations["prepared"] / ".readbench-chain" / "rotterdam_delfshaven"
        self.assertEqual(stamp.read_text().strip(), str(bench_suite.chain_version()))

    def test_no_cache_downloads_the_sources_builds_the_rest_and_uploads(self):
        self.run_prep("no-cache", force=True)
        self.assertEqual(self.calls[0], ("download", "rotterdam_delfshaven", ("cityjson", "citygml")))
        prepares = [c for c in self.calls if c[:2] == ("just", "readbench-prepare")]
        self.assertEqual(prepares[0][3:], (str(self.locations["prepared"]), "cityjsonseq,flatcitybuf,cityparquet"))
        uploads = [c for c in self.calls if c[0] == "upload"]
        self.assertEqual(uploads, [("upload", "rotterdam_delfshaven", ("citygml", "cityjson", "cityjsonseq", "flatcitybuf", "cityparquet"), True, "u")])
        self.assertEqual(self.calls[-1], ("publish", ["rotterdam_delfshaven"]))

    def test_rebuild_sources_builds_everything_and_records_the_published_source(self):
        self.run_prep("rebuild-sources")
        self.assertIn(("just", "fetch-data", str(self.locations["corpus"])), self.calls)
        self.assertNotIn("download", [c[0] for c in self.calls])
        uploads = [c for c in self.calls if c[0] == "upload"]
        self.assertEqual(uploads[0][3:], (False, "published"))
        self.assertEqual(self.calls[-1][0], "publish")

    def test_local_mode_touches_no_bucket(self):
        self.run_prep("local")
        self.assertFalse([c for c in self.calls if c[0] in ("download", "upload", "publish")])
        self.assertEqual(self.provenance(), {"origin": "local"})


class PublishedSourceTest(unittest.TestCase):
    def test_the_published_url_comes_from_the_fetch_table(self):
        self.assertTrue(bench_suite.published_source_url("rotterdam_delfshaven.city.json").endswith("/3-20-DELFSHAVEN.city.json"))
        self.assertIsNone(bench_suite.published_source_url("nothing.city.json"))


class NetworkFamilyTest(unittest.TestCase):
    manifest = bench_suite.load_manifest()

    def test_network_is_a_family_and_typical_is_the_default_profile(self):
        self.assertIn("network", bench_suite.family_selection("all"))
        self.assertEqual(bench_suite.network_profiles(self.manifest, "typical", None, None), [("typical", 100.0, 20.0)])

    def test_profiles_are_manifest_data_and_all_selects_three(self):
        self.assertEqual(bench_suite.network_profiles(self.manifest, "all", None, None),
                         [("fast", 1000.0, 5.0), ("typical", 100.0, 20.0), ("slow", 20.0, 50.0)])

    def test_explicit_bandwidth_and_latency_make_a_custom_profile(self):
        self.assertEqual(bench_suite.network_profiles(self.manifest, "typical", 50.0, 10.0), [("custom", 50.0, 10.0)])

    def test_an_unknown_profile_names_the_known_ones(self):
        with self.assertRaises(SystemExit) as raised:
            bench_suite.network_profiles(self.manifest, "dialup", None, None)
        self.assertIn("fast, typical, slow", str(raised.exception))

    def test_network_results_land_per_suite_and_network_profile(self):
        locations = bench_suite.paths(bench_suite.DEFAULT_DATA_ROOT)
        self.assertEqual(bench_suite.network_dir(locations, "short", "slow"), locations["network"] / "short" / "slow")

    def test_simulated_runs_pass_the_profile_and_the_slice_adds_the_bloom_pair(self):
        from unittest import mock
        calls = []
        with mock.patch.object(bench_suite, "just", lambda *a: calls.append(a)), \
             mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
             mock.patch.object(bench_suite, "source", lambda entry, locations: Path(entry["file"])), \
             mock.patch.object(bench_suite, "stage", lambda locations, name, inputs: Path("+".join(str(i) for i in inputs))), \
             mock.patch.object(bench_suite, "write_model_times", lambda *a: None), \
             mock.patch.object(bench_suite, "write_run_manifest", lambda *a, **k: None):
            manifest = {"suite": {}, "network_profiles": self.manifest["network_profiles"],
                        "datasets": {"c": {"role": "corpus", "file": "c.city.json"}, "s": {"role": "slice", "file": "s.city.jsonl", "bloom_attributes": ["a"]}}}
            locations = {"prepared": Path("p"), "network": Path("n")}
            bench_suite.run_suite(manifest, locations, ["network"], ["c", "s"], "full", network={"profiles": "slow"})
        bench = [c for c in calls if c[0] == "bench"]
        self.assertEqual(len(bench), 1)
        self.assertIn("--transport http --network-profile slow --network-bandwidth-mbps 20.0 --network-latency-ms 50.0", bench[0][-1])
        self.assertEqual(bench[0][2], str(Path("n/full/slow")))
        self.assertEqual(bench[0][5], "3")  # repeat: the deterministic network needs few samples
        bloom = [c for c in calls if c[0] == "variant-bench"]
        self.assertEqual(len(bloom), 1)
        self.assertEqual(bloom[0][1], "s.city.jsonl")
        self.assertIn("--network-latency-ms 50.0", bloom[0][-1])

    def test_the_whole_file_scenarios_come_from_the_manifest_unless_overridden(self):
        from unittest import mock

        def net_args(manifest_network, override):
            calls = []
            with mock.patch.object(bench_suite, "just", lambda *a: calls.append(a)), \
                 mock.patch.object(bench_suite, "require_prepared", lambda *a: None), \
                 mock.patch.object(bench_suite, "source", lambda entry, locations: Path(entry["file"])), \
                 mock.patch.object(bench_suite, "stage", lambda locations, name, inputs: Path("+".join(str(i) for i in inputs))), \
                 mock.patch.object(bench_suite, "write_model_times", lambda *a: None), \
                 mock.patch.object(bench_suite, "write_run_manifest", lambda *a, **k: None):
                manifest = {"suite": {}, "network_profiles": self.manifest["network_profiles"],
                            "datasets": {"c": {"role": "corpus", "file": "c.city.json"}}, **manifest_network}
                network = {"profiles": "typical", **({"whole_file_scenarios": override} if override else {})}
                bench_suite.run_suite(manifest, {"prepared": Path("p"), "network": Path("n")}, ["network"], ["c"], "full", network=network)
            return [c for c in calls if c[0] == "bench"][0][-1]

        self.assertIn("--whole-file-scenarios full-read,id-lookup", net_args({}, None))
        self.assertIn("--whole-file-scenarios full-read", net_args({"network": {"whole_file_scenarios": "full-read"}}, None))
        self.assertIn("--whole-file-scenarios all", net_args({"network": {"whole_file_scenarios": "full-read"}}, "all"))
        self.assertEqual(bench_suite.whole_file_scenarios({"network": {"whole_file_scenarios": "full-read,id-lookup"}}, None), "full-read,id-lookup")
        with self.assertRaises(SystemExit):
            bench_suite.whole_file_scenarios({}, "count")

    def test_the_model_time_is_transfer_plus_latency_per_request(self):
        import csv, json, tempfile
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "d.csv"
            out.write_text("dataset,format,scenario,time_median_s,notes,bytes_read,http_requests\nd,cityparquet,count,0.05,,1250000,2\n")
            Path(f"{out}.params.json").write_text(json.dumps({"network": {"target": "simulated", "profile": "typical", "bandwidth_mbps": 100.0, "latency_ms": 20.0}}))
            bench_suite.write_model_times(out)
            rows = list(csv.DictReader(out.with_suffix(".model.csv").open()))
        self.assertEqual(rows[0]["model_s"], "0.140000")
        self.assertEqual(rows[0]["network_profile"], "typical")
