"""The orchestrator's contract: only an unreachable root stops the run.

Every test here runs offline against stubs. The real converter, the real
`city3dstac` and the real catalogue are all absent by design — what is under
test is the isolation of failures, not the behaviour of the tools being
isolated.
"""

from __future__ import annotations

import json
import re
import threading

import httpx
import pytest

from catalog2cityparquet import __main__ as driver
from catalog2cityparquet import convert
from catalog2cityparquet.discover import Item
from catalog2cityparquet.ledger import Ledger

#: A port nothing listens on, so a request fails instantly with ECONNREFUSED.
#: Loopback only: the suite never leaves the machine.
UNREACHABLE = "http://127.0.0.1:1"


def _config(tmp_path, **overrides):
    """A Config pointing at throwaway paths; the binaries never run."""
    overrides.setdefault("out", tmp_path / "out")
    return driver.Config(binary=tmp_path / "b", tool=tmp_path / "t", **overrides)


def _stub_conversion(monkeypatch, *, run_convert=None, seen=None):
    """Replace the download/normalise/convert chain with in-memory stubs."""
    record = {} if seen is None else seen

    def fake_download(url, dest, client, timeout):
        record["dest"] = dest
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(b"{}")
        return 2

    def default_run_convert(binary, inputs, out_dir, crs, timeout):
        record["out_dir"] = out_dir
        out_dir.mkdir(parents=True, exist_ok=True)
        return 3

    monkeypatch.setattr(driver.fetch, "download", fake_download)
    monkeypatch.setattr(driver.fetch, "normalise", lambda path, workdir: [path])
    monkeypatch.setattr(driver.convert, "run_convert", run_convert or default_run_convert)
    monkeypatch.setattr(driver.convert, "stamp", lambda pkg_dir, item: None)
    return record


def _break_ledger(monkeypatch, ledger, when=lambda rec: True):
    """Make `ledger.record` raise, the way a full or read-only disk would."""
    attempts = []
    original = ledger.record

    def exploding(rec):
        attempts.append(rec)
        if when(rec):
            raise OSError("[Errno 28] No space left on device")
        return original(rec)

    monkeypatch.setattr(ledger, "record", exploding)
    return attempts


# --- the central requirement -------------------------------------------------


def test_a_failing_collection_does_not_stop_the_next(tmp_path, monkeypatch):
    # The brief's hard requirement: "if generation fails on a particular
    # collection, skip and go to the next; we don't terminate."
    attempted = []

    def fake_convert_collection(cid, **kwargs):
        attempted.append(cid)
        if cid == "boom":
            raise RuntimeError("collection exploded")

    monkeypatch.setattr(driver, "convert_collection", fake_convert_collection)

    ledger = Ledger(tmp_path / "_reports")
    driver.run_collections(["alpha", "boom", "omega"], ledger=ledger, config=_config(tmp_path))

    assert attempted == ["alpha", "boom", "omega"], "every collection must be attempted"
    assert ledger.counts("boom")["failed"] == 1


def test_a_failing_item_does_not_stop_the_next(tmp_path, monkeypatch):
    processed = []

    def fake_process(item, **kwargs):
        processed.append(item.item_id)
        if item.item_id == "bad":
            raise RuntimeError("item exploded")
        return 1

    monkeypatch.setattr(driver, "process_item", fake_process)
    items = [Item("c", i, "u", None, None) for i in ("good1", "bad", "good2")]
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(items, ledger=ledger, config=_config(tmp_path, jobs=1))

    assert processed == ["good1", "bad", "good2"]
    counts = ledger.counts("c")
    assert counts["converted"] == 2
    assert counts["failed"] == 1


def test_every_item_is_attempted_under_a_thread_pool(tmp_path, monkeypatch):
    # Isolation must not depend on the serial path: with jobs > 1 the failures
    # travel back through the pool, where an unhandled one would surface only
    # when the result is consumed.
    def fake_process(item, **kwargs):
        if item.item_id.startswith("bad"):
            raise RuntimeError("item exploded")

    monkeypatch.setattr(driver, "process_item", fake_process)
    items = [Item("c", f"{'bad' if n % 2 else 'good'}{n}", "u", None, None) for n in range(10)]
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(items, ledger=ledger, config=_config(tmp_path, jobs=4))

    assert ledger.counts("c") == {"converted": 5, "failed": 5}


def test_jobs_really_buys_concurrency(tmp_path, monkeypatch):
    # Tallies alone cannot tell a pool from a loop, so the items rendezvous:
    # a serial implementation never assembles four at once and the barrier
    # times out, failing every item instead of converting it.
    width = 4
    barrier = threading.Barrier(width, timeout=10)
    lock = threading.Lock()
    in_flight = 0
    peak = 0

    def fake_process(item, **kwargs):
        nonlocal in_flight, peak
        with lock:
            in_flight += 1
            peak = max(peak, in_flight)
        try:
            barrier.wait()
        finally:
            with lock:
                in_flight -= 1

    monkeypatch.setattr(driver, "process_item", fake_process)
    items = [Item("c", f"i{n}", "u", None, None) for n in range(width)]
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(items, ledger=ledger, config=_config(tmp_path, jobs=width))

    assert peak == width, f"--jobs {width} must run {width} items at once, saw {peak}"
    assert ledger.counts("c") == {"converted": width}


# --- resumption --------------------------------------------------------------


def test_already_converted_items_are_skipped_on_resume(tmp_path, monkeypatch):
    out = tmp_path / "out"
    pkg = out / "c" / "items" / "done"
    pkg.mkdir(parents=True)
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "stac_version": "1.1.0", "id": "done"})
    )

    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    items = [Item("c", "done", "u", None, None), Item("c", "todo", "u", None, None)]
    ledger = Ledger(tmp_path / "_reports")
    driver.convert_items(
        items, ledger=ledger, config=_config(tmp_path, out=out, jobs=1, skip_existing=True)
    )

    assert processed == ["todo"], "a package with a valid Item must not be redone"
    # A skipped-existing item is not an outcome of this run, so it is not one
    # of this run's records.
    assert ledger.counts("c") == {"converted": 1}


def test_a_half_written_package_is_reattempted(tmp_path, monkeypatch):
    # The Rust writer renames metadata.json last, so anything unparseable —
    # or absent beside a directory full of Parquet — means the conversion never
    # finished and must be redone.
    out = tmp_path / "out"
    (out / "c" / "items" / "truncated").mkdir(parents=True)
    (out / "c" / "items" / "truncated" / "metadata.json").write_text('{"type": "Fea')
    (out / "c" / "items" / "nometa").mkdir(parents=True)
    (out / "c" / "items" / "nometa" / "building.parquet").write_bytes(b"PAR1")
    # Well-formed JSON that is not a STAC Item at all. Accepting it because it
    # parses would skip an item that was never converted.
    (out / "c" / "items" / "notanitem").mkdir(parents=True)
    (out / "c" / "items" / "notanitem" / "metadata.json").write_text('{"foo": 1}')
    # A Feature without stac_version is likewise not a STAC Item.
    (out / "c" / "items" / "noversion").mkdir(parents=True)
    (out / "c" / "items" / "noversion" / "metadata.json").write_text('{"type": "Feature"}')
    # Valid JSON, but not an object.
    (out / "c" / "items" / "notadoc").mkdir(parents=True)
    (out / "c" / "items" / "notadoc" / "metadata.json").write_text("[]")

    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    unfinished = ["truncated", "nometa", "notanitem", "noversion", "notadoc"]
    driver.convert_items(
        [Item("c", name, "u", None, None) for name in unfinished],
        ledger=Ledger(tmp_path / "_reports"),
        config=_config(tmp_path, out=out, jobs=1, skip_existing=True),
    )

    assert processed == unfinished


def test_skip_existing_can_be_turned_off(tmp_path, monkeypatch):
    out = tmp_path / "out"
    pkg = out / "c" / "items" / "done"
    pkg.mkdir(parents=True)
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "stac_version": "1.1.0", "id": "done"})
    )
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    driver.convert_items(
        [Item("c", "done", "u", None, None)],
        ledger=Ledger(tmp_path / "_reports"),
        config=_config(tmp_path, out=out, jobs=1, skip_existing=False),
    )

    assert processed == ["done"]


# --- what is never downloaded ------------------------------------------------


def test_duplicate_bundles_are_skipped_before_download(tmp_path, monkeypatch):
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))

    items = [
        Item("japan-plateau-3d", "x_citygml_1_op", "u", None, None),
        Item("japan-plateau-3d", "48395630_bldg_6697_op", "u", None, None),
    ]
    ledger = Ledger(tmp_path / "_reports")
    driver.convert_items(items, ledger=ledger, config=_config(tmp_path, jobs=1))

    assert processed == ["48395630_bldg_6697_op"]
    assert ledger.histogram()["duplicate_bundle"] == 1


# --- reason attribution ------------------------------------------------------


def test_failure_reasons_are_attributed_separately(tmp_path, monkeypatch):
    # The histogram is the deliverable: transport failures must not inflate the
    # converter's count, and a classified ConvertError must keep its own reason.
    failures = {
        "transport": httpx.ConnectError("no route to host"),
        "disk": OSError("no space left on device"),
        "crs": convert.ConvertError("no_crs", "the source declares no CRS"),
        "odd": ValueError("something else entirely"),
    }

    def fake_process(item, **kwargs):
        raise failures[item.item_id]

    monkeypatch.setattr(driver, "process_item", fake_process)
    ledger = Ledger(tmp_path / "_reports")
    driver.convert_items(
        [Item("c", name, "u", None, None) for name in failures],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert ledger.histogram() == {"download_failed": 2, "no_crs": 1, "convert_failed": 1}


# --- one item, end to end ----------------------------------------------------


def test_the_download_is_named_from_the_url(tmp_path, monkeypatch):
    # estonia-3d hides the filename in a query parameter; saving the payload as
    # an extensionless blob would make `normalise` discard the whole collection.
    seen = _stub_conversion(monkeypatch)
    url = "https://example.invalid/dl.ashx?f=tallinn.gml"
    item = Item("estonia-3d", "tallinn", url, None, None)

    driver.process_item(item, config=_config(tmp_path), client=None)

    assert seen["dest"].name == "tallinn.gml"


def test_a_payload_with_nothing_convertible_is_a_classified_failure(tmp_path, monkeypatch):
    _stub_conversion(monkeypatch)
    monkeypatch.setattr(driver.fetch, "normalise", lambda path, workdir: [])

    with pytest.raises(convert.ConvertError) as excinfo:
        driver.process_item(
            Item("c", "i", "https://example.invalid/a.zip", None, None),
            config=_config(tmp_path),
            client=None,
        )

    assert excinfo.value.reason == "unsupported_archive"


def test_the_per_collection_crs_fallback_reaches_the_converter(tmp_path, monkeypatch):
    seen = {}

    def fake_run_convert(binary, inputs, out_dir, crs, timeout):
        seen["crs"] = crs
        return 1

    _stub_conversion(monkeypatch, run_convert=fake_run_convert)
    config = _config(tmp_path, crs_by_collection={"estonia-3d": "EPSG:3301"})

    driver.process_item(
        Item("estonia-3d", "i", "https://example.invalid/a.gml", None, None),
        config=config,
        client=None,
    )
    assert seen["crs"] == "EPSG:3301"

    driver.process_item(
        Item("other-3d", "i", "https://example.invalid/a.gml", None, None),
        config=config,
        client=None,
    )
    assert seen["crs"] is None


def test_the_working_directory_is_removed_even_when_the_item_failed(tmp_path, monkeypatch):
    def boom(binary, inputs, out_dir, crs, timeout):
        raise convert.ConvertError("convert_failed", "nope")

    seen = _stub_conversion(monkeypatch, run_convert=boom)

    with pytest.raises(convert.ConvertError):
        driver.process_item(
            Item("c", "i", "https://example.invalid/a.gml", None, None),
            config=_config(tmp_path),
            client=None,
        )

    assert not seen["dest"].parent.exists(), "a failed item must not leave its download behind"


def test_keep_downloads_retains_the_working_directory(tmp_path, monkeypatch):
    seen = _stub_conversion(monkeypatch)

    driver.process_item(
        Item("c", "i", "https://example.invalid/a.gml", None, None),
        config=_config(tmp_path, keep_downloads=True),
        client=None,
    )

    assert seen["dest"].is_file()


def test_the_downloaded_byte_count_reaches_the_ledger(tmp_path, monkeypatch):
    # The ledger carries a `bytes` column; leaving it at zero would waste the
    # one cheap measurement of how much of the catalogue a run actually moved.
    _stub_conversion(monkeypatch)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", "i", "https://example.invalid/a.gml", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    written = json.loads((tmp_path / "_reports" / "c.jsonl").read_text())
    assert written["status"] == "converted"
    assert written["bytes"] == 2


def test_bytes_moved_before_a_failure_are_still_recorded(tmp_path, monkeypatch):
    def boom(binary, inputs, out_dir, crs, timeout):
        raise convert.ConvertError("convert_failed", "nope")

    _stub_conversion(monkeypatch, run_convert=boom)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", "i", "https://example.invalid/a.gml", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    written = json.loads((tmp_path / "_reports" / "c.jsonl").read_text())
    assert written["status"] == "failed"
    assert written["bytes"] == 2


# --- one collection ----------------------------------------------------------


def _stub_collection(monkeypatch, items, note=None, indexed=True):
    """Stub a collection's discovery and aggregation; returns the call log."""
    calls = {}
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})
    monkeypatch.setattr(
        driver.discover,
        "enumerate_items",
        lambda base, api, cid, collection, client: (list(items), note),
    )
    monkeypatch.setattr(
        driver, "convert_items", lambda items, **k: calls.setdefault("items", items)
    )
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)

    def fake_update_collection(tool, items_dir, config, out, **kwargs):
        calls["items_dir"] = items_dir
        return indexed(items_dir.parent.name) if callable(indexed) else indexed

    monkeypatch.setattr(driver.aggregate, "update_collection", fake_update_collection)
    return calls


def test_limit_per_collection_truncates_the_work(tmp_path, monkeypatch):
    items = [Item("c", f"i{n}", "u", None, None) for n in range(5)]
    calls = _stub_collection(monkeypatch, items)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c"], ledger=ledger, config=_config(tmp_path, limit_per_collection=2))

    assert [i.item_id for i in calls["items"]] == ["i0", "i1"]


def test_an_empty_collection_is_recorded_not_converted(tmp_path, monkeypatch):
    calls = _stub_collection(monkeypatch, [])
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c"], ledger=ledger, config=_config(tmp_path))

    assert "items" not in calls
    assert ledger.histogram() == {"empty_collection": 1}


def test_a_stale_item_index_is_recorded_and_reported(tmp_path, monkeypatch, capsys):
    items = [Item("c", "i0", "u", None, None)]
    _stub_collection(monkeypatch, items, note="stale item index: 306 vs 60471")
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c"], ledger=ledger, config=_config(tmp_path))

    assert ledger.histogram()["stale_item_index"] == 1
    assert "stale item index" in capsys.readouterr().err


def test_collections_without_a_geoparquet_index_are_counted(tmp_path, monkeypatch):
    items = [Item("c", "i0", "u", None, None)]
    _stub_collection(monkeypatch, items, indexed=lambda cid: cid != "beta")
    ledger = Ledger(tmp_path / "_reports")

    state = driver.run_collections(["alpha", "beta"], ledger=ledger, config=_config(tmp_path))

    assert state.no_index == ["beta"]


# --- the whole run -----------------------------------------------------------


def test_an_unreachable_catalogue_root_is_the_only_non_zero_exit(tmp_path, monkeypatch):
    def unreachable(base_url, client):
        raise httpx.ConnectError("no route to host")

    monkeypatch.setattr(driver.discover, "collection_ids", unreachable)
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    assert driver.run(_config(tmp_path)) == 1


def test_a_run_whose_every_collection_failed_still_exits_zero(tmp_path, monkeypatch, capsys):
    # A run with failures is a successful run that measured failures.
    monkeypatch.setattr(
        driver.discover, "collection_ids", lambda base, client: (["alpha", "beta"], None)
    )
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    def fail(cid, **kwargs):
        raise RuntimeError("nope")

    monkeypatch.setattr(driver, "convert_collection", fail)

    assert driver.run(_config(tmp_path)) == 0
    assert "alpha" in capsys.readouterr().err


def test_an_unusable_child_link_note_is_surfaced(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(
        driver.discover,
        "collection_ids",
        lambda base, client: (["alpha"], "skipped child link(s) with an unusable collection id"),
    )
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)

    driver.run(_config(tmp_path))

    assert "unusable collection id" in capsys.readouterr().err


def test_the_summary_reports_counts_reasons_and_missing_indexes(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    def one_each(cid, *, ledger, config, client, state):
        from catalog2cityparquet.ledger import Record

        ledger.record(Record(cid, "a", "converted"))
        ledger.record(Record(cid, "b", "failed", reason="no_crs"))
        if cid == "beta":
            state.note_no_index(cid)

    monkeypatch.setattr(driver, "convert_collection", one_each)

    assert driver.run(_config(tmp_path), collections=["alpha", "beta"]) == 0

    out = capsys.readouterr().out
    # Rendered values, not merely the labels: a hard-coded zero must not pass.
    assert re.search(r"^alpha\s+1\s+1\s+0$", out, re.M), out
    assert re.search(r"^beta\s+1\s+1\s+0$", out, re.M), out
    assert re.search(r"^TOTAL\s+2\s+2\s+0$", out, re.M), out
    assert re.search(r"^\s+2\s+no_crs$", out, re.M), out
    assert "collections without a GeoParquet index: 1" in out
    assert re.search(r"^\s+- beta$", out, re.M), out
    # The roll-up is written for later analysis, not only printed.
    assert (tmp_path / "out" / "_reports" / "summary.csv").is_file()


def test_explicit_collections_never_touch_the_catalogue_root(tmp_path, monkeypatch):
    def unreachable(base_url, client):
        raise AssertionError("--collection must not need the catalogue root")

    monkeypatch.setattr(driver.discover, "collection_ids", unreachable)
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    seen = []
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: seen.append(cid))

    assert driver.run(_config(tmp_path), collections=["alpha"]) == 0
    assert seen == ["alpha"]


# --- the isolation handler must not defeat itself ----------------------------


def test_a_ledger_failure_does_not_stop_the_collection_run(tmp_path, monkeypatch, capsys):
    # The recovery path writes to the same disk that is failing. A multi-day
    # run hits ENOSPC eventually, and the handler must not be the thing that
    # ends it.
    attempted = []

    def fake_convert_collection(cid, **kwargs):
        attempted.append(cid)
        raise RuntimeError("collection exploded")

    monkeypatch.setattr(driver, "convert_collection", fake_convert_collection)
    ledger = Ledger(tmp_path / "_reports")
    _break_ledger(monkeypatch, ledger)

    driver.run_collections(["alpha", "beta", "omega"], ledger=ledger, config=_config(tmp_path))

    assert attempted == ["alpha", "beta", "omega"]
    assert "No space left on device" in capsys.readouterr().err


def test_a_ledger_failure_does_not_lose_the_rest_of_the_collection(tmp_path, monkeypatch, capsys):
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    ledger = Ledger(tmp_path / "_reports")
    _break_ledger(monkeypatch, ledger, when=lambda rec: rec.item_id == "first")

    driver.convert_items(
        [Item("c", name, "u", None, None) for name in ("first", "second", "third")],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert processed == ["first", "second", "third"]
    assert "could not be recorded" in capsys.readouterr().err


def test_an_unrecordable_item_does_not_take_the_collection_with_it(tmp_path, monkeypatch):
    # `items_from_parquet` builds Item(collection=r[3] or ""), so one null cell
    # in a published items.parquet used to make the ledger reject the first
    # item and kill the whole collection.
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    ledger = Ledger(tmp_path / "_reports")

    items = [Item("", "first", "u", None, None), Item("c", "second", "u", None, None)]
    driver.convert_items(items, ledger=ledger, config=_config(tmp_path, jobs=1), collection="c")

    assert processed == ["first", "second"]
    assert ledger.counts("c") == {"converted": 2}


def test_items_are_recorded_against_the_collection_being_converted(tmp_path, monkeypatch):
    # The catalogue's own `collection` field is not authoritative: aggregation
    # reads out/<cid>/items, so a package written anywhere else is invisible.
    seen = _stub_conversion(monkeypatch)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("wrong", "i", "https://example.invalid/a.gml", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        collection="right",
    )

    assert ledger.counts("right") == {"converted": 1}
    assert seen["out_dir"] == tmp_path / "out" / "right" / "items" / "i"


def test_a_duplicate_bundle_is_judged_by_the_collection_being_converted(tmp_path, monkeypatch):
    # Same null-collection cell, but here trusting it would defeat the filter
    # and download hundreds of gigabytes of re-packaged data.
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("", "x_citygml_1_op", "u", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        collection="japan-plateau-3d",
    )

    assert processed == []
    assert ledger.histogram() == {"duplicate_bundle": 1}


def test_a_hostile_item_id_cannot_escape_the_output_directory(tmp_path):
    out = tmp_path / "out"
    config = _config(tmp_path, out=out)

    resolved = driver.package_dir(config, Item("c", "../../../etc/evil", "u", None, None)).resolve()

    assert resolved.is_relative_to(out.resolve()), resolved


def test_an_unusable_item_id_is_one_failed_item_not_a_dead_collection(tmp_path, monkeypatch):
    _stub_conversion(monkeypatch)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [
            Item("c", "..", "https://example.invalid/a.gml", None, None),
            Item("c", "fine", "https://example.invalid/a.gml", None, None),
        ],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert ledger.counts("c") == {"failed": 1, "converted": 1}
    assert ledger.histogram() == {"convert_failed": 1}


def test_a_summary_write_failure_still_prints_the_tallies(tmp_path, monkeypatch, capsys):
    ledger = Ledger(tmp_path / "_reports")
    ledger.record(driver.Record("c", "a", "converted"))

    def exploding():
        raise OSError("[Errno 30] Read-only file system")

    monkeypatch.setattr(ledger, "write_summary", exploding)

    driver.print_summary(ledger, driver.RunState())

    captured = capsys.readouterr()
    assert re.search(r"^c\s+1\s+0\s+0$", captured.out, re.M), captured.out
    assert "Read-only file system" in captured.err


# --- catalogue aggregation ---------------------------------------------------


def test_aggregate_all_tolerates_an_unreachable_root(tmp_path, monkeypatch, capsys):
    seen = {}
    (tmp_path / "out" / "c").mkdir(parents=True)
    (tmp_path / "out" / "c" / "collection.json").write_text("{}")
    monkeypatch.setattr(
        driver.aggregate, "update_catalog", lambda tool, jsons, out, config: seen.setdefault("n", 1)
    )

    driver.aggregate_all(_config(tmp_path, base_url=UNREACHABLE), driver.RunState())

    assert seen == {"n": 1}, "the collections must still be linked into a catalogue"
    assert "catalogue root metadata unavailable" in capsys.readouterr().err


def test_aggregate_all_tolerates_a_missing_tool(tmp_path, capsys):
    (tmp_path / "out" / "c").mkdir(parents=True)
    (tmp_path / "out" / "c" / "collection.json").write_text("{}")

    # `tool` points at a path that does not exist, as it would on a machine
    # where `just catalog-tools` was never run.
    driver.aggregate_all(_config(tmp_path, base_url=UNREACHABLE), driver.RunState())

    assert "catalogue aggregation failed" in capsys.readouterr().err


def test_aggregate_all_skips_a_run_that_produced_no_collection(tmp_path, monkeypatch, capsys):
    called = []
    monkeypatch.setattr(driver.aggregate, "update_catalog", lambda *a, **k: called.append(1))
    (tmp_path / "out").mkdir(parents=True)

    driver.aggregate_all(_config(tmp_path, base_url=UNREACHABLE), driver.RunState())

    assert called == []
    assert "no collection.json" in capsys.readouterr().err


# --- the command line --------------------------------------------------------


def test_the_documented_flags_become_a_config():
    args = driver.parse_args(
        [
            "--out",
            "mirror",
            "--collection",
            "alpha",
            "--collection",
            "beta",
            "--limit-per-collection",
            "5",
            "--jobs",
            "2",
            "--keep-downloads",
            "--no-skip-existing",
            "--crs",
            "estonia-3d=EPSG:3301",
            "--crs",
            "montreal-3d=EPSG:2950",
            "--binary",
            "bin/cityparquet",
            "--tool",
            "bin/city3dstac",
            "--base-url",
            "http://example.invalid/cat",
            "--bucket-api",
            "http://example.invalid/api",
        ]
    )
    config = driver.config_from_args(args)

    assert args.collections == ["alpha", "beta"]
    assert config.out.name == "mirror"
    assert config.jobs == 2
    assert config.limit_per_collection == 5
    assert config.keep_downloads is True
    assert config.skip_existing is False
    assert config.crs_by_collection == {"estonia-3d": "EPSG:3301", "montreal-3d": "EPSG:2950"}
    assert config.base_url == "http://example.invalid/cat"
    assert config.bucket_api == "http://example.invalid/api"
    assert str(config.binary) == "bin/cityparquet"
    assert str(config.tool) == "bin/city3dstac"


def test_skip_existing_is_on_by_default():
    config = driver.config_from_args(driver.parse_args([]))
    assert config.skip_existing is True
    assert config.keep_downloads is False


def test_a_malformed_crs_mapping_is_rejected_before_any_work():
    with pytest.raises(SystemExit):
        driver.config_from_args(driver.parse_args(["--crs", "EPSG:3301"]))


@pytest.mark.parametrize("bad", ["bad id!", "../etc", "a/b", ""])
def test_an_unusable_collection_id_is_rejected_before_any_work(bad):
    # The id names a ledger file and an output directory. Failing at parse time
    # beats failing forty collections into a multi-day run.
    with pytest.raises(SystemExit):
        driver.config_from_args(driver.parse_args(["--collection", bad]))


@pytest.mark.parametrize("bad", ["0", "-1", "-2"])
def test_a_useless_limit_is_rejected(bad):
    with pytest.raises(SystemExit):
        driver.parse_args(["--limit-per-collection", bad])


def test_no_limit_means_no_limit(tmp_path, monkeypatch):
    items = [Item("c", f"i{n}", "u", None, None) for n in range(5)]
    calls = _stub_collection(monkeypatch, items)

    driver.run_collections(["c"], ledger=Ledger(tmp_path / "_reports"), config=_config(tmp_path))

    assert len(calls["items"]) == 5


def test_aggregate_only_converts_nothing(tmp_path, monkeypatch):
    called = []
    monkeypatch.setattr(driver, "run_collections", lambda cids, **k: called.append("convert"))
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: called.append("aggregate"))

    code = driver.main(
        ["--out", str(tmp_path / "out"), "--aggregate-only", "--base-url", UNREACHABLE]
    )

    assert code == 0
    assert called == ["aggregate"]


def test_main_converts_when_aggregate_only_is_absent(tmp_path, monkeypatch):
    seen = []
    monkeypatch.setattr(driver, "run_collections", lambda cids, **k: seen.append(list(cids)))
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    code = driver.main(
        ["--out", str(tmp_path / "out"), "--collection", "alpha", "--base-url", UNREACHABLE]
    )

    assert code == 0
    assert seen == [["alpha"]]


def test_main_reports_an_unreachable_root(tmp_path, monkeypatch):
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    assert driver.main(["--out", str(tmp_path / "out"), "--base-url", UNREACHABLE]) == 1


# --- the working directory ---------------------------------------------------


def test_the_working_directory_is_configurable(tmp_path, monkeypatch):
    elsewhere = tmp_path / "scratch"
    seen = _stub_conversion(monkeypatch)

    driver.process_item(
        Item("c", "i", "https://example.invalid/a.gml", None, None),
        config=_config(tmp_path, work_dir=elsewhere, keep_downloads=True),
        client=None,
    )

    assert elsewhere in seen["dest"].parents


def test_stale_working_directories_are_swept_at_the_start(tmp_path):
    config = _config(tmp_path, out=tmp_path / "out")
    work = driver.work_root(config)
    (work / "c2cp-abandoned").mkdir(parents=True)
    (work / "c2cp-abandoned" / "half.gml").write_bytes(b"x")
    (work / "notours").mkdir()

    driver.sweep_work_root(config)

    assert not (work / "c2cp-abandoned").exists(), "an aborted run must not litter the mirror"
    assert (work / "notours").exists(), "only our own leftovers are ours to delete"


def test_keeping_downloads_keeps_them_across_runs(tmp_path):
    config = _config(tmp_path, out=tmp_path / "out", keep_downloads=True)
    work = driver.work_root(config)
    (work / "c2cp-kept").mkdir(parents=True)

    driver.sweep_work_root(config)

    assert (work / "c2cp-kept").exists(), "--keep-downloads means keep them"
