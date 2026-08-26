from citybench.manifest import collect, required_keys


def test_manifest_records_everything_needed_to_cite_a_number():
    m = collect(
        dataset_name="delft",
        ingest={"cjdb": 12.5, "3dcitydb": 40.1},
        sizes={"cjdb": (100, 80), "3dcitydb": (200, 150)},
        versions={"postgres": "16.4", "postgis": "3.4"},
        pg_settings={"shared_buffers": "8GB"},
    )
    for key in required_keys():
        assert key in m, f"manifest missing {key}"


def test_ingest_times_are_marked_non_comparable():
    m = collect(
        dataset_name="d", ingest={"cjdb": 1.0}, sizes={}, versions={}, pg_settings={},
    )
    assert "not comparable" in m["ingest"]["caveat"].lower()


# --- Tests beyond the brief -------------------------------------------
#
# manifest.collect() is pure and every field it produces ends up quoted
# verbatim in a published number's provenance, so each piece of shape and
# pass-through behaviour is worth locking down individually rather than
# trusting the two tests above (which only check presence of top-level
# keys) to catch a typo or a swapped argument.


def test_required_keys_is_exactly_the_eight_documented_fields():
    # Locks the set itself: a key silently added to or dropped from
    # collect()'s output would otherwise only be caught by chance, by
    # whichever individual test happens to check for it. `patches` was
    # added for cjdb's ground-surfaces-tie patch (Task 12 fix round 3) —
    # see manifest.py's own module docstring for why it is a dedicated
    # section rather than folded into `versions`. `srid` was added for
    # Task 14 (the heterogeneity corpus): 3DCityDB's SRID is baked in at
    # schema creation and cannot be changed afterwards, and getting it
    # wrong does not error, so the SRID each PostgreSQL-backed system
    # actually landed on is recorded, not just requested.
    assert required_keys() == (
        "dataset", "host", "versions", "pg_settings", "ingest", "sizes",
        "patches", "srid",
    )


def test_patches_defaults_to_empty_when_nothing_was_patched():
    m = collect(dataset_name="d", ingest={}, sizes={}, versions={}, pg_settings={})
    assert m["patches"] == {}


def test_patches_carries_through_a_disclosed_patch_verbatim():
    patches = {
        "cjdb": {
            "upstream_version": "2.2.0",
            "patched": "true",
            "patch_file": "vendor/cjdb/ground-surfaces-tie.patch",
            "patch_summary": "retains tied-Z footprint faces instead of dropping them",
            "built_from": "/some/path/cjdb-2.2.0+abc123",
        }
    }
    m = collect(
        dataset_name="d", ingest={}, sizes={}, versions={}, pg_settings={},
        patches=patches,
    )
    assert m["patches"] == patches
    assert m["patches"] is patches


def test_dataset_name_is_stamped_verbatim():
    m = collect(dataset_name="rotterdam", ingest={}, sizes={}, versions={}, pg_settings={})
    assert m["dataset"] == "rotterdam"


def test_host_block_has_platform_processor_and_python_as_non_empty_strings():
    m = collect(dataset_name="d", ingest={}, sizes={}, versions={}, pg_settings={})
    host = m["host"]
    assert set(host) == {"platform", "processor", "python"}
    assert isinstance(host["platform"], str) and host["platform"]
    assert isinstance(host["python"], str) and host["python"]


def test_versions_and_pg_settings_pass_through_unchanged():
    versions = {"duckdb": "1.3.2", "postgres": "16.4"}
    pg_settings = {"cjdb": {"shared_buffers": "8GB"}, "3dcitydb": {"shared_buffers": "8GB"}}
    m = collect(dataset_name="d", ingest={}, sizes={}, versions=versions, pg_settings=pg_settings)
    assert m["versions"] == versions
    assert m["pg_settings"] == pg_settings
    # Not merely equal in content — the same object, so a caller mutating
    # one cannot silently desync from the other.
    assert m["versions"] is versions
    assert m["pg_settings"] is pg_settings


def test_ingest_wall_clock_s_carries_every_system_untouched():
    ingest = {"cjdb": 12.5, "3dcitydb": 40.1, "duckdb-cityparquet": 0.0}
    m = collect(dataset_name="d", ingest=ingest, sizes={}, versions={}, pg_settings={})
    assert m["ingest"]["wall_clock_s"] == ingest


def test_sizes_are_relabelled_to_total_and_no_index_bytes_not_left_as_a_tuple():
    m = collect(
        dataset_name="d", ingest={}, sizes={"cjdb": (900, 700)}, versions={}, pg_settings={},
    )
    assert m["sizes"]["cjdb"] == {"total_bytes": 900, "no_index_bytes": 700}


def test_sizes_handles_multiple_systems_independently():
    m = collect(
        dataset_name="d", ingest={},
        sizes={"cjdb": (900, 700), "cityparquet": (400, 400)},
        versions={}, pg_settings={},
    )
    assert m["sizes"]["cjdb"]["total_bytes"] == 900
    assert m["sizes"]["cityparquet"]["total_bytes"] == 400
    assert m["sizes"]["cityparquet"]["no_index_bytes"] == 400


def test_empty_dicts_everywhere_still_produce_every_required_key():
    # The degenerate all-empty case: nothing to report, but the shape must
    # still be complete — a downstream reader should never need to guess
    # whether a missing key means "empty" or "this run predates the field".
    m = collect(dataset_name="d", ingest={}, sizes={}, versions={}, pg_settings={})
    for key in required_keys():
        assert key in m
    assert m["ingest"]["wall_clock_s"] == {}
    assert m["sizes"] == {}
    assert m["srid"] == {}


def test_srid_defaults_to_empty_when_nothing_was_supplied():
    m = collect(dataset_name="d", ingest={}, sizes={}, versions={}, pg_settings={})
    assert m["srid"] == {}


def test_srid_carries_through_the_landed_value_per_system():
    # Read back from each adapter's own database query, not restated from
    # whatever CITYDB_SRID was requested — see collect()'s own docstring.
    srid = {"cjdb": 2950, "3dcitydb": 2950}
    m = collect(
        dataset_name="Montreal", ingest={}, sizes={}, versions={}, pg_settings={},
        srid=srid,
    )
    assert m["srid"] == srid
    assert m["srid"] is srid
