"""The orchestrator's contract: only an unreachable root stops the run.

Every test here runs offline against stubs. The real converter, the real
`city3dstac` and the real catalogue are all absent by design — what is under
test is the isolation of failures, not the behaviour of the tools being
isolated.
"""

from __future__ import annotations

import ast
import contextlib
import csv
import errno
import gzip
import json
import os
import re
import stat
import subprocess
import sys
import threading
from dataclasses import replace
from pathlib import Path

import httpx
import pytest

from catalog2cityparquet import __main__ as driver
from catalog2cityparquet import convert
from catalog2cityparquet.discover import Item
from catalog2cityparquet.ledger import HostFailure, Ledger

#: A port nothing listens on, so a request fails instantly with ECONNREFUSED.
#: Loopback only: the suite never leaves the machine.
UNREACHABLE = "http://127.0.0.1:1"


def _config(tmp_path, **overrides):
    """A Config pointing at throwaway paths; the binaries never run."""
    overrides.setdefault("out", tmp_path / "out")
    return driver.Config(binary=tmp_path / "b", tool=tmp_path / "t", **overrides)


def _write_package(config, item, collection=None) -> Path:
    """Leave behind what a successful conversion leaves behind.

    A package is what aggregation looks for, and the driver now declines to
    aggregate a collection that produced none — so a stub that converts without
    writing anything would exercise the wrong branch. `collection` overrides
    the item's own, exactly as `convert_items` does: the id we asked for is the
    id the package is written under.
    """
    if collection is not None:
        item = replace(item, collection=collection)
    pkg = driver.package_dir(config, item)
    pkg.mkdir(parents=True, exist_ok=True)
    (pkg / "metadata.json").write_text(
        json.dumps({"type": "Feature", "stac_version": "1.1.0", "id": item.item_id}),
        encoding="utf-8",
    )
    return pkg


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
        # The real converter writes the package's STAC Item last; the driver
        # reads that file both to resume and to decide whether there is
        # anything to aggregate.
        (out_dir / "metadata.json").write_text(
            json.dumps({"type": "Feature", "stac_version": "1.1.0"}), encoding="utf-8"
        )
        return 3

    monkeypatch.setattr(driver.fetch, "download", fake_download)
    monkeypatch.setattr(driver.fetch, "normalise", lambda path, workdir: [path])
    monkeypatch.setattr(driver.convert, "run_convert", run_convert or default_run_convert)
    monkeypatch.setattr(driver.convert, "stamp", lambda pkg_dir, item: None)
    return record


class _FailingStream:
    """A stream whose every write fails.

    The shape of a full volume (ENOSPC — what `/dev/full` gives), a read-only
    one, and of the `BrokenPipeError` raised when a run is piped through
    `head`.
    """

    def write(self, text):
        raise OSError("[Errno 28] No space left on device")

    def flush(self):
        raise OSError("[Errno 28] No space left on device")


class _UnprintableError(Exception):
    """An exception that cannot even be rendered into a message."""

    def __str__(self):
        raise RuntimeError("even the message is broken")


class _UnprintableId:
    """A record field that cannot be rendered into a message either."""

    def __str__(self):
        raise RuntimeError("even the id is broken")

    def __repr__(self):
        raise RuntimeError("even the id is broken")


def _break_stderr(monkeypatch):
    monkeypatch.setattr(sys, "stderr", _FailingStream())


def _break_stdout(monkeypatch):
    monkeypatch.setattr(sys, "stdout", _FailingStream())


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
        # A real full volume carries ENOSPC; the errno is what tells it from a
        # corrupt payload, which raises an `OSError` with no errno at all.
        "disk": OSError(errno.ENOSPC, "No space left on device"),
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

    # The full disk is *not* among them: it is this machine, and it is counted
    # in the environment column instead (see the environment section below).
    assert ledger.histogram() == {"download_failed": 1, "no_crs": 1, "convert_failed": 1}
    assert ledger.counts("c")[driver.ENVIRONMENT] == 1


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
        lambda base, api, cid, collection, client, dropped=None: (list(items), note),
    )

    def fake_convert_items(items, **kwargs):
        calls.setdefault("items", items)
        for item in items:
            _write_package(kwargs["config"], item, kwargs.get("collection"))

    monkeypatch.setattr(driver, "convert_items", fake_convert_items)
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
    # The trailing column is the environment tally, which a clean run leaves 0.
    # The leading column is what enumeration discovered; this stub records
    # outcomes without going through it, so it stays 0 while the tallies do not.
    assert re.search(r"^alpha\s+0\s+1\s+1\s+0\s+0$", out, re.M), out
    assert re.search(r"^beta\s+0\s+1\s+1\s+0\s+0$", out, re.M), out
    assert re.search(r"^TOTAL\s+0\s+2\s+2\s+0\s+0$", out, re.M), out
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
    assert re.search(r"^c\s+0\s+1\s+0\s+0\s+0$", captured.out, re.M), captured.out
    assert "Read-only file system" in captured.err


def test_a_failing_stderr_does_not_stop_the_collection_run(tmp_path, monkeypatch):
    # The handler's diagnostic goes to the same place its record does. Piping a
    # run through `head` closes stderr, and the report of a failure must not
    # become a bigger failure than the one it reports.
    attempted = []

    def fake_convert_collection(cid, **kwargs):
        attempted.append(cid)
        raise RuntimeError("collection exploded")

    monkeypatch.setattr(driver, "convert_collection", fake_convert_collection)
    ledger = Ledger(tmp_path / "_reports")
    _break_stderr(monkeypatch)

    driver.run_collections(["alpha", "beta", "omega"], ledger=ledger, config=_config(tmp_path))

    assert attempted == ["alpha", "beta", "omega"]
    assert ledger.counts("alpha") == {"failed": 1}


def test_a_failing_stderr_and_ledger_together_lose_no_items(tmp_path, monkeypatch):
    # The last-resort reporter reporting the ledger's failure is itself on the
    # failing disk. Under the pool, an escape here takes the rest of the
    # collection with it.
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    ledger = Ledger(tmp_path / "_reports")
    _break_ledger(monkeypatch, ledger)
    _break_stderr(monkeypatch)

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(6)],
        ledger=ledger,
        config=_config(tmp_path, jobs=2),
    )

    assert sorted(processed) == [f"i{n}" for n in range(6)]


def test_a_failing_stderr_does_not_stop_the_whole_run(tmp_path, monkeypatch):
    monkeypatch.setattr(
        driver.discover, "collection_ids", lambda base, client: (["alpha"], "an unusable link")
    )
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver, "print_summary", lambda ledger, state: None)
    _break_stderr(monkeypatch)

    assert driver.run(_config(tmp_path)) == 0


def test_a_failing_stdout_does_not_fabricate_a_collection_failure(tmp_path, monkeypatch):
    # The gravest shape of this bug: the progress line is written inside the
    # collection's isolation try, so a full log volume was caught and recorded
    # as a *data* failure. The deliverable of this project is a measured
    # statement about which collections convert; inventing failures for all of
    # them is worse than crashing.
    converted = []
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})
    monkeypatch.setattr(
        driver.discover,
        "enumerate_items",
        lambda base, api, cid, collection, client, dropped=None: (
            [Item(cid, "i", "u", None, None)],
            None,
        ),
    )

    def convert_items(items, **kwargs):
        converted.append(kwargs.get("collection"))
        for item in items:
            _write_package(kwargs["config"], item, kwargs.get("collection"))

    monkeypatch.setattr(driver, "convert_items", convert_items)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)
    monkeypatch.setattr(driver.aggregate, "update_collection", lambda *a, **k: True)
    ledger = Ledger(tmp_path / "_reports")
    _break_stdout(monkeypatch)

    driver.run_collections(["c0", "c1", "c2"], ledger=ledger, config=_config(tmp_path))

    assert converted == ["c0", "c1", "c2"], "every collection must still be converted"
    assert ledger.histogram() == {}, "a broken log must not be recorded as a data failure"


def test_a_failing_stdout_does_not_stop_the_run(tmp_path, monkeypatch):
    monkeypatch.setattr(driver.discover, "collection_ids", lambda base, client: (["alpha"], None))
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    _break_stdout(monkeypatch)

    assert driver.run(_config(tmp_path)) == 0


def test_the_last_resort_report_survives_an_unprintable_error(tmp_path, monkeypatch):
    # `_record_safely`'s message is built inside its guard, so an exception
    # whose __str__ raises is caught there and nowhere else.
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    ledger = Ledger(tmp_path / "_reports")

    def exploding(rec):
        raise _UnprintableError()

    monkeypatch.setattr(ledger, "record", exploding)

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(3)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert processed == ["i0", "i1", "i2"]


@pytest.mark.skipif(not Path("/dev/full").exists(), reason="needs Linux /dev/full")
@pytest.mark.parametrize("unbuffered", [False, True])
def test_a_full_stdout_never_makes_a_measured_run_exit_non_zero(tmp_path, unbuffered):
    # Only a real interpreter shows this: CPython flushes sys.stdout while
    # finalising, and a failure there sets exit status 120 — a non-zero exit
    # for a run that measured everything, breaking the contract that non-zero
    # means nothing was measured.
    argv = [sys.executable]
    if unbuffered:
        argv.append("-u")
    argv += [
        "-m",
        "catalog2cityparquet",
        "--out",
        str(tmp_path / "out"),
        "--aggregate-only",
        "--base-url",
        UNREACHABLE,
    ]
    with Path("/dev/full").open("w") as full:
        proc = subprocess.run(argv, stdout=full, stderr=subprocess.PIPE, text=True)

    assert proc.returncode == 0, f"rc={proc.returncode} stderr={proc.stderr}"


@pytest.mark.skipif(not Path("/dev/full").exists(), reason="needs a POSIX shell pipeline")
def test_a_closed_stdout_pipe_never_makes_a_measured_run_exit_non_zero(tmp_path):
    argv = [
        sys.executable,
        "-m",
        "catalog2cityparquet",
        "--out",
        str(tmp_path / "out"),
        "--aggregate-only",
        "--base-url",
        UNREACHABLE,
    ]
    head = subprocess.Popen(["head", "-1"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL)
    proc = subprocess.run(argv, stdout=head.stdin, stderr=subprocess.PIPE, text=True)
    head.stdin.close()
    head.wait()

    assert proc.returncode == 0, f"rc={proc.returncode} stderr={proc.stderr}"


# --- the thread pool itself --------------------------------------------------


class _PoolThatDiesAfterOneItem:
    """A pool that runs one task, then fails the way an exhausted host does."""

    def __init__(self, max_workers=None):
        pass

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        return False

    def map(self, fn, iterable):
        pending = list(iterable)
        fn(pending[0])
        raise RuntimeError("can't start new thread")


def test_a_pool_failure_does_not_lose_the_collection(tmp_path, monkeypatch, capsys):
    # A pool-level failure must be contained the way an item failure is: the
    # items it never reached are finished serially rather than dropped.
    processed = []
    monkeypatch.setattr(driver, "process_item", lambda item, **k: processed.append(item.item_id))
    monkeypatch.setattr(driver, "ThreadPoolExecutor", _PoolThatDiesAfterOneItem)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(3)],
        ledger=ledger,
        config=_config(tmp_path, jobs=4),
    )

    assert processed == ["i0", "i1", "i2"], "no item lost, and none done twice"
    assert ledger.counts("c") == {"converted": 3}
    assert "can't start new thread" in capsys.readouterr().err


@pytest.mark.parametrize("bad", ["0", "-5"])
def test_a_useless_job_count_is_rejected(bad):
    with pytest.raises(SystemExit):
        driver.parse_args(["--jobs", bad])


# --- startup ------------------------------------------------------------------


def test_an_unwritable_output_directory_is_a_clean_startup_error(tmp_path, capsys):
    # `out`'s parent is a file, so mkdir fails for any user, root included.
    blocker = tmp_path / "blocker"
    blocker.write_text("not a directory")

    code = driver.run(_config(tmp_path, out=blocker / "mirror"))

    assert code == 1
    assert "output directory" in capsys.readouterr().err


# --- the working directory lock ----------------------------------------------


def _dead_pid():
    """A pid that has certainly exited."""
    proc = subprocess.Popen([sys.executable, "-c", ""])
    proc.wait()
    return proc.pid


def test_a_second_run_refuses_a_busy_working_directory(tmp_path, monkeypatch, capsys):
    # Two runs sharing one work root would sweep each other's live downloads,
    # and a vanished multi-gigabyte download resurfaces as `download_failed` —
    # indistinguishable from origin flakiness. Better a startup error.
    config = _config(tmp_path)
    other = _config(tmp_path, out=tmp_path / "other-out", work_dir=driver.work_root(config))
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)
    holder = driver.acquire_lock(driver.work_root(config), driver.WORKING_DIRECTORY)
    (driver.work_root(config) / "c2cp-live").mkdir()

    code = driver.run(other, collections=["alpha"])

    assert code == 1
    err = capsys.readouterr().err.lower()
    assert "another run" in err
    assert "--work-dir" in err, "here the advice is sound: the output directories differ"
    assert (driver.work_root(config) / "c2cp-live").exists(), "a live download must survive"
    driver.release_lock(holder)


def test_a_second_run_on_the_same_output_is_refused_whatever_its_work_dir(
    tmp_path, monkeypatch, capsys
):
    # The ledger, not the work root, is the resource that gets corrupted: two
    # runs write two JSONL lines per collection while summary.csv — rewritten
    # wholesale by whichever finishes last — says one.
    config = _config(tmp_path)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)
    config.out.mkdir(parents=True)
    holder = driver.acquire_lock(config.out, driver.OUTPUT_DIRECTORY)

    code = driver.run(_config(tmp_path, work_dir=tmp_path / "elsewhere"), collections=["alpha"])

    assert code == 1
    err = capsys.readouterr().err.lower()
    assert "another run" in err
    assert "--work-dir" not in err, "a second work dir does not make a shared ledger safe"
    driver.release_lock(holder)


def test_aggregate_only_also_claims_the_output(tmp_path, monkeypatch, capsys):
    # Aggregating while another run is mid-package would index half a package.
    config = _config(tmp_path)
    config.out.mkdir(parents=True)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)
    holder = driver.acquire_lock(config.out, driver.OUTPUT_DIRECTORY)

    assert driver.run(config, aggregate_only=True) == 1
    driver.release_lock(holder)


def test_a_lock_left_by_a_dead_run_is_reclaimed(tmp_path):
    root = driver.work_root(_config(tmp_path))
    root.mkdir(parents=True)
    (root / driver.LOCK_NAME).write_text(
        json.dumps({"pid": _dead_pid(), "host": driver.HOSTNAME, "started": 0})
    )

    holder = driver.acquire_lock(root, driver.WORKING_DIRECTORY)

    assert holder is not None
    driver.release_lock(holder)
    assert not (root / driver.LOCK_NAME).exists()


def test_a_lock_is_released_only_by_its_owner(tmp_path):
    # An operator who follows the busy message and deletes a stale-looking lock
    # must not have this run's exit delete their new run's live claim.
    root = tmp_path / "work"
    root.mkdir()
    holder = driver.acquire_lock(root, driver.WORKING_DIRECTORY)
    (root / driver.LOCK_NAME).write_text(
        json.dumps({"pid": 4242, "host": "someone-else", "started": 1})
    )

    driver.release_lock(holder)

    assert (root / driver.LOCK_NAME).exists(), "someone else's claim is not ours to drop"


def test_the_locks_are_released_when_the_run_ends(tmp_path, monkeypatch):
    config = _config(tmp_path)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)

    assert driver.run(config, collections=["alpha"]) == 0
    assert not (driver.work_root(config) / driver.LOCK_NAME).exists()
    assert not (config.out / driver.LOCK_NAME).exists()


def test_the_locks_are_released_even_when_the_root_is_unreachable(tmp_path, monkeypatch):
    config = _config(tmp_path, base_url=UNREACHABLE)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)

    assert driver.run(config) == 1
    assert not (config.out / driver.LOCK_NAME).exists()


def test_an_unreachable_root_leaves_no_working_directory(tmp_path, monkeypatch):
    config = _config(tmp_path, base_url=UNREACHABLE)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)

    assert driver.run(config) == 1
    assert not driver.work_root(config).exists(), "nothing ran, so nothing should be left behind"


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


# --- the environment is not the data -----------------------------------------


def _converting_collection(monkeypatch):
    """Stub discovery and conversion so every item of every collection converts."""
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})
    monkeypatch.setattr(
        driver.discover,
        "enumerate_items",
        lambda base, api, cid, collection, client, dropped=None: (
            [Item(cid, "it0", "u", None, None)],
            None,
        ),
    )

    def fake_process_item(item, *, config, **kwargs):
        _write_package(config, item)
        return 1

    monkeypatch.setattr(driver, "process_item", fake_process_item)
    monkeypatch.setattr(driver.aggregate, "update_collection", lambda *a, **k: True)


def test_an_unwritable_config_is_not_a_conversion_failure(tmp_path, monkeypatch):
    # The shape of a read-only or full volume under the mirror: `_configs`
    # cannot be made, so `write_config` raises from *inside* the collection's
    # isolation try. Every item converted; recording the collection as
    # `convert_failed` would put five fabricated data failures into the
    # histogram the paper quotes.
    _converting_collection(monkeypatch)
    config = _config(tmp_path)
    config.out.mkdir(parents=True)
    (config.out / "_configs").write_text("not a directory")
    ledger = Ledger(tmp_path / "_reports")

    state = driver.RunState()
    driver.run_collections(["c0", "c1"], ledger=ledger, config=config, state=state)

    assert ledger.histogram() == {}, "no environment failure may reach the conformance histogram"
    for cid in ("c0", "c1"):
        counts = ledger.counts(cid)
        assert counts.get("failed", 0) == 0, f"{cid} converted; the machine is what failed"
        assert counts[driver.ENVIRONMENT] == 1
    assert state.environment_seen == 2
    # Which step failed is the operator's remedy: a mirror that cannot be
    # aggregated needs a different fix from one that cannot be downloaded into.
    last = json.loads((tmp_path / "_reports" / "c0.jsonl").read_text().splitlines()[-1])
    assert last["error"].startswith("aggregation:"), last


def test_a_local_failure_reaching_the_collection_handler_is_not_a_conversion_failure(
    tmp_path, monkeypatch
):
    # Everything remote arrives as an `httpx` error and every filesystem
    # failure inside the item loop is handled there, so an OSError reaching the
    # collection handler is this machine — a host out of descriptors refusing
    # to open the next `httpx.Client`, say.
    def out_of_descriptors(cid, **kwargs):
        raise OSError(24, "Too many open files")

    monkeypatch.setattr(driver, "convert_collection", out_of_descriptors)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.run_collections(["c0", "c1"], ledger=ledger, config=_config(tmp_path), state=state)

    assert ledger.histogram() == {}
    assert ledger.counts("c0") == {driver.ENVIRONMENT: 1}
    assert state.environment_seen == 2


def test_a_missing_aggregation_tool_is_not_a_conversion_failure(tmp_path, monkeypatch):
    # `just catalog-tools` never run: every `city3dstac` call raises
    # FileNotFoundError. Half the catalogue would otherwise be published as
    # unconvertible on the strength of one absent binary.
    _converting_collection(monkeypatch)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)

    def missing_tool(*args, **kwargs):
        raise FileNotFoundError(2, "No such file or directory", "city3dstac")

    monkeypatch.setattr(driver.aggregate, "update_collection", missing_tool)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c0"], ledger=ledger, config=_config(tmp_path))

    assert ledger.histogram() == {}
    assert ledger.counts("c0") == {"converted": 1, driver.ENVIRONMENT: 1}


def test_an_unreachable_collection_is_still_a_conversion_failure(tmp_path, monkeypatch):
    # The boundary the environment reason must not creep past: no `httpx`
    # exception is an `OSError`, so a publisher being down keeps its place in
    # the histogram. Were that to change, half the measured failures would
    # quietly leave the published number.
    def unreachable(base, cid, client):
        raise httpx.ConnectError("no route to host")

    monkeypatch.setattr(driver.discover, "fetch_collection", unreachable)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c0"], ledger=ledger, config=_config(tmp_path))

    assert ledger.histogram() == {"convert_failed": 1}
    assert ledger.environment_failures() == {}


def test_a_tool_that_ran_and_refused_is_still_a_conversion_failure(tmp_path, monkeypatch):
    # The other half of the distinction: `city3dstac` reporting that a
    # collection has no spatial extent is a fact about the data, and must keep
    # its place in the histogram.
    _converting_collection(monkeypatch)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)

    def refused(*args, **kwargs):
        raise RuntimeError("update-collection failed: spatial extent bbox is required")

    monkeypatch.setattr(driver.aggregate, "update_collection", refused)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c0"], ledger=ledger, config=_config(tmp_path))

    assert ledger.histogram() == {"convert_failed": 1}


def test_a_tool_that_ran_out_of_disk_is_not_a_conversion_failure(tmp_path, monkeypatch):
    # `city3dstac` ran, and failed because *its* volume filled. It exits
    # non-zero exactly as it does when it refuses the data, so the tool's own
    # environment failure used to arrive here as a `RuntimeError` and be
    # published as `convert_failed`. This is the likelier shape of ENOSPC than
    # an unwritable `_configs`: a volume with room for a 200-byte YAML but not
    # for a multi-gigabyte items.parquet takes exactly this path.
    _converting_collection(monkeypatch)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)

    def out_of_disk(*args, **kwargs):
        raise driver.aggregate.HostFailure(
            "update-collection failed: I/O error: No space left on device (os error 28)"
        )

    monkeypatch.setattr(driver.aggregate, "update_collection", out_of_disk)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.run_collections(["c0", "c1"], ledger=ledger, config=_config(tmp_path), state=state)

    assert ledger.histogram() == {}, "the tool's full disk says nothing about these datasets"
    for cid in ("c0", "c1"):
        assert ledger.counts(cid) == {"converted": 1, driver.ENVIRONMENT: 1}
    assert state.environment_seen == 2
    last = json.loads((tmp_path / "_reports" / "c0.jsonl").read_text().splitlines()[-1])
    assert last["error"].startswith("aggregation:"), last


def test_a_local_failure_during_an_item_is_not_a_download_failure(tmp_path, monkeypatch):
    # A full volume under the working directory is this machine, not the
    # origin — and `download_failed` is the conformance reason most likely to
    # be read as "the publisher was unavailable", which is precisely the claim
    # the run must not fabricate.
    def full_volume(item, **kwargs):
        raise OSError(28, "No space left on device")

    monkeypatch.setattr(driver, "process_item", full_volume)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(3)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert ledger.histogram() == {}
    assert ledger.counts("c") == {driver.ENVIRONMENT: 3}
    assert state.environment_seen == 3


def test_a_transport_failure_during_an_item_is_still_a_download_failure(tmp_path, monkeypatch):
    # The other side of the split: an origin that is down is a fact about the
    # catalogue and keeps its place in the published histogram.
    def unreachable(item, **kwargs):
        raise httpx.ConnectError("no route to host")

    monkeypatch.setattr(driver, "process_item", unreachable)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", "i", "u", None, None)], ledger=ledger, config=_config(tmp_path, jobs=1)
    )

    assert ledger.histogram() == {"download_failed": 1}
    assert ledger.environment_failures() == {}


class _TransportErrorOnALocalFile(httpx.HTTPError, OSError):
    """A transport failure that is also an `OSError`.

    `httpx` wraps lower-level failures, so the two catches can overlap; the
    clause order decides which wins. A genuine network error reclassified as
    this machine would silently delete measured failures from the histogram.
    """


def test_a_transport_error_that_is_also_local_stays_a_download_failure(tmp_path, monkeypatch):
    def wrapped(item, **kwargs):
        raise _TransportErrorOnALocalFile("connection reset while reading")

    monkeypatch.setattr(driver, "process_item", wrapped)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", "i", "u", None, None)], ledger=ledger, config=_config(tmp_path, jobs=1)
    )

    assert ledger.histogram() == {"download_failed": 1}, "the httpx clause must come first"


def test_bytes_moved_before_an_environment_failure_are_still_recorded(tmp_path, monkeypatch):
    # Routing to the environment must not cost the cost measurement: how much
    # the run moved before the volume filled is exactly as interesting as how
    # much a successful item moved.
    def full_volume(binary, inputs, out_dir, crs, timeout):
        raise OSError(28, "No space left on device")

    _stub_conversion(monkeypatch, run_convert=full_volume)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", "i", "https://example.invalid/a.gml", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    written = json.loads((tmp_path / "_reports" / "c.jsonl").read_text())
    assert written["status"] == driver.ENVIRONMENT
    assert written["bytes"] == 2


def test_a_lost_record_is_counted_as_environment_trouble(tmp_path, monkeypatch):
    # When the ledger itself is what failed there is nowhere to write the fact,
    # so it is counted in process: a run that could not record its outcomes must
    # not print a clean-looking table at the end.
    monkeypatch.setattr(driver, "process_item", lambda item, **k: 1)
    ledger = Ledger(tmp_path / "_reports")
    _break_ledger(monkeypatch, ledger)
    state = driver.RunState()

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(3)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert state.environment_seen == 3


def test_the_summary_segregates_environment_failures(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    _converting_collection(monkeypatch)
    config = _config(tmp_path)
    config.out.mkdir(parents=True)
    (config.out / "_configs").write_text("not a directory")

    assert driver.run(config, collections=["c0"]) == 0

    captured = capsys.readouterr()
    # The tallies keep the two apart, column by column...
    assert re.search(r"^c0\s+1\s+1\s+0\s+0\s+1$", captured.out, re.M), captured.out
    # ...the reasons histogram stays empty of them...
    assert "convert_failed" not in captured.out
    # ...and the run says loudly that it was the machine, on both streams.
    assert "environment failure" in captured.out
    assert "not the data" in captured.out
    assert "environment failure" in captured.err


def test_a_broken_ledger_accessor_does_not_cost_the_exit_code(tmp_path, monkeypatch):
    # Pins `run`'s guard around `print_summary`: everything inside it is
    # separately guarded, so only a failure of the ledger's own accessors can
    # reach it — and that must still not turn a measured run into a traceback.
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)

    class _BrokenLedger(Ledger):
        def collections(self):
            raise RuntimeError("the ledger's own accessor is broken")

    monkeypatch.setattr(driver, "Ledger", _BrokenLedger)

    assert driver.run(_config(tmp_path), collections=["alpha"]) == 0


def test_an_unprintable_summary_failure_does_not_cost_the_exit_code(tmp_path, monkeypatch):
    # The same guard, with the exception the guard itself cannot render. The
    # message is built at the call site, outside `_warn` — so on this path, an
    # isolation path reached *after* a fully measured run, an unrenderable
    # error turned the whole measurement into a traceback and rc=1.
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)

    class _BrokenLedger(Ledger):
        def collections(self):
            raise _UnprintableError("the ledger's own accessor is broken")

    monkeypatch.setattr(driver, "Ledger", _BrokenLedger)

    assert driver.run(_config(tmp_path), collections=["alpha"]) == 0


# --- nothing may be built outside a guard ------------------------------------


def test_every_report_goes_through_the_guarded_helpers():
    # The invariant behind the whole class, asserted on the source rather than
    # on behaviour: a `print` added inside an isolation `try` — for debugging,
    # for progress, for anything — reopens it instantly, and a sink added
    # inside a function the subprocess falsifier stubs out is invisible to
    # every other test here. `_warn` and `_say` are the only writers.
    tree = ast.parse(Path(driver.__file__).read_text(encoding="utf-8"))
    prints = {
        node.lineno
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and getattr(node.func, "id", None) == "print"
    }
    guarded = {
        node.lineno
        for helper in ast.walk(tree)
        if isinstance(helper, ast.FunctionDef) and helper.name in ("_warn", "_say")
        for node in ast.walk(helper)
        if isinstance(node, ast.Call) and getattr(node.func, "id", None) == "print"
    }
    assert prints == guarded, (
        f"unguarded print() in __main__.py at line(s) {sorted(prints - guarded)}: "
        "every report must go through _warn or _say"
    )


def test_an_unprintable_collection_failure_does_not_stop_the_run(tmp_path, monkeypatch):
    # The handler's message and the record's `error=` are built *before* the
    # guarded calls that use them, so an exception whose __str__ raises escapes
    # the handler and the remaining collections are never attempted.
    attempted = []

    def fake_convert_collection(cid, **kwargs):
        attempted.append(cid)
        raise _UnprintableError()

    monkeypatch.setattr(driver, "convert_collection", fake_convert_collection)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["alpha", "beta", "omega"], ledger=ledger, config=_config(tmp_path))

    assert attempted == ["alpha", "beta", "omega"]
    assert ledger.counts("omega")["failed"] == 1


def test_an_unprintable_item_failure_does_not_stop_the_collection(tmp_path, monkeypatch):
    processed = []

    def fake_process(item, **kwargs):
        processed.append(item.item_id)
        raise _UnprintableError()

    monkeypatch.setattr(driver, "process_item", fake_process)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(3)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert processed == ["i0", "i1", "i2"]
    assert ledger.counts("c") == {"failed": 3}


def test_an_unrenderable_record_does_not_stop_the_run(tmp_path, monkeypatch):
    # What the last-resort reporter's own guard still covers now that the
    # exception is rendered by `_describe`: the *record's* fields. `Record` is a
    # plain dataclass and its ids come from a published catalogue read by
    # DuckDB, so nothing guarantees the id in a lost record can be printed.
    ledger = Ledger(tmp_path / "_reports")

    def exploding(rec):
        raise OSError("[Errno 28] No space left on device")

    monkeypatch.setattr(ledger, "record", exploding)

    driver._record_safely(ledger, driver.Record("c", _UnprintableId(), "converted"))


def test_an_unprintable_download_failure_does_not_stop_the_collection(tmp_path, monkeypatch):
    # The transport branch builds its detail outside the guard too.
    class _UnprintableTransportError(httpx.HTTPError):
        def __init__(self):
            super().__init__("")

        def __str__(self):
            raise RuntimeError("even the message is broken")

    processed = []

    def fake_process(item, **kwargs):
        processed.append(item.item_id)
        raise _UnprintableTransportError()

    monkeypatch.setattr(driver, "process_item", fake_process)
    ledger = Ledger(tmp_path / "_reports")

    driver.convert_items(
        [Item("c", f"i{n}", "u", None, None) for n in range(2)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
    )

    assert processed == ["i0", "i1"]
    assert ledger.histogram() == {"download_failed": 2}


# --- the locks, pinned -------------------------------------------------------


def test_a_lock_holding_invalid_utf8_is_released_without_raising(tmp_path):
    # `release_lock` runs from an ExitStack in `run`'s `finally`; anything it
    # raises escapes `main` as a traceback and a non-zero exit *after* a fully
    # measured run. `UnicodeDecodeError` is a ValueError, not an OSError.
    root = tmp_path / "work"
    root.mkdir()
    claim = driver.acquire_lock(root, driver.WORKING_DIRECTORY)
    (root / driver.LOCK_NAME).write_bytes(b"\xff\xfe not utf-8")

    driver.release_lock(claim)

    assert (root / driver.LOCK_NAME).exists(), "an unreadable claim is not ours to drop"


def test_a_corrupted_lock_does_not_cost_a_measured_run_its_exit_code(tmp_path, monkeypatch):
    config = _config(tmp_path)
    monkeypatch.setattr(driver, "aggregate_all", lambda c, state: None)

    def corrupt_the_lock(cid, **kwargs):
        (config.out / driver.LOCK_NAME).write_bytes(b"\xff\xfe")

    monkeypatch.setattr(driver, "convert_collection", corrupt_the_lock)

    assert driver.run(config, collections=["alpha"]) == 0


def test_a_lock_from_another_host_is_never_reclaimed(tmp_path):
    # Over a shared filesystem the pid in a lock file means nothing here, so a
    # foreign host's claim is always live. Guessing it dead would sweep another
    # machine's downloads mid-run.
    root = tmp_path / "work"
    root.mkdir()
    (root / driver.LOCK_NAME).write_text(
        json.dumps({"pid": _dead_pid(), "host": "some-other-host", "started": 0})
    )

    with pytest.raises(driver.LockBusy):
        driver.acquire_lock(root, driver.WORKING_DIRECTORY)


@pytest.mark.parametrize("pid", [None, "not-a-pid", 0, -1])
def test_a_lock_with_an_unusable_pid_is_never_reclaimed(tmp_path, pid):
    root = tmp_path / "work"
    root.mkdir()
    info = {"host": driver.HOSTNAME, "started": 0}
    if pid is not None:
        info["pid"] = pid
    (root / driver.LOCK_NAME).write_text(json.dumps(info))

    with pytest.raises(driver.LockBusy):
        driver.acquire_lock(root, driver.WORKING_DIRECTORY)


def test_a_process_we_cannot_signal_counts_as_live(tmp_path):
    # pid 1 exists and is not ours to signal; "cannot tell" must mean "busy".
    root = tmp_path / "work"
    root.mkdir()
    (root / driver.LOCK_NAME).write_text(
        json.dumps({"pid": 1, "host": driver.HOSTNAME, "started": 0})
    )

    with pytest.raises(driver.LockBusy):
        driver.acquire_lock(root, driver.WORKING_DIRECTORY)


# --- the standard streams may not outlive the run ----------------------------


def _stream_on_a_closed_pipe():
    """A buffered text stream whose writes reach a pipe nobody reads.

    The shape of `2>&1 | tee` with `tee` killed: the buffered writes succeed and
    only the flush fails, which is precisely what the interpreter's finalisation
    flush turns into exit status 120.
    """
    read_fd, write_fd = os.pipe()
    os.close(read_fd)
    return os.fdopen(write_fd, "w")


@pytest.mark.parametrize("name", ["stdout", "stderr"])
def test_a_stream_that_cannot_be_flushed_is_neutralised(name):
    # Needs no /dev/full, so it covers the exit-120 defence on any POSIX host:
    # a guarded flush alone leaves the bytes in the buffer and the interpreter
    # fails on them again while finalising.
    stream = _stream_on_a_closed_pipe()
    try:
        stream.write("x" * 64)

        driver._flush_stream(stream, name)

        stream.write("y" * 64)
        stream.flush()  # must not raise: the descriptor was repointed
    finally:
        with contextlib.suppress(Exception):
            stream.close()


def test_main_neutralises_both_streams(tmp_path, monkeypatch):
    # The wiring, not just the helper: deleting either call from `main` must
    # fail a test even on a machine without /dev/full.
    streams = {"stdout": _stream_on_a_closed_pipe(), "stderr": _stream_on_a_closed_pipe()}
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(sys, "stdout", streams["stdout"])
    monkeypatch.setattr(sys, "stderr", streams["stderr"])
    try:
        code = driver.main(["--out", str(tmp_path / "out"), "--aggregate-only"])
    finally:
        monkeypatch.undo()

    assert code == 0
    for name, stream in streams.items():
        stream.write("z" * 64)
        stream.flush()  # must not raise
        stream.close()
        assert name  # names the parametrised stream in a failure


# --- the falsifier: a broken stream may change nothing but the log -----------

#: The whole driver with only the network stubbed out, so a subprocess can run
#: it end to end against a real interpreter, real files and real descriptors.
_DRIVER_SCRIPT = """
import os
import sys

from catalog2cityparquet import __main__ as driver
from catalog2cityparquet.discover import Item

driver.discover.collection_ids = lambda base, client: (["c0", "c1", "c2"], "a note for stderr")
driver.discover.fetch_collection = lambda base, cid, client: {"id": cid}
driver.discover.enumerate_items = lambda base, api, cid, collection, client, dropped=None: (
    [Item(cid, "it0", "u", None, None)],
    None,
)
driver.aggregate_all = lambda config, state: None

if os.environ.get("C2CP_FULL_VOLUME"):
    # The working volume fills mid-download: a real write of a real payload,
    # refused by a real kernel, through the real `process_item`. SIGXFSZ is
    # ignored so the write raises EFBIG rather than killing the process, which
    # is how a full volume behaves. The ledger's own files stay far below the
    # limit, so what fails is the download and only the download.
    import resource
    import signal

    signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
    LIMIT = 4096
    resource.setrlimit(resource.RLIMIT_FSIZE, (LIMIT, LIMIT))

    def _download(url, dest, client, timeout):
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(b"x" * (LIMIT * 8))
        return LIMIT * 8

    driver.fetch.download = _download
else:
    # A conversion that leaves a package behind, since that is what the driver
    # looks for before it aggregates anything.
    def _process_item(item, *, config, **kwargs):
        pkg = driver.package_dir(config, item)
        pkg.mkdir(parents=True, exist_ok=True)
        (pkg / "metadata.json").write_text(
            '{"type": "Feature", "stac_version": "1.1.0"}', encoding="utf-8"
        )
        return 1

    driver.process_item = _process_item

if not os.environ.get("C2CP_REAL_AGGREGATION"):
    driver.aggregate.update_collection = lambda *a, **k: True

raise SystemExit(driver.main(sys.argv[1:]))
"""


def _driver_script(tmp_path):
    path = tmp_path / "run_driver.py"
    path.write_text(_DRIVER_SCRIPT, encoding="utf-8")
    return path


def _ledger_rows(out: Path):
    """Every recorded outcome, stripped of the fields a clock makes vary."""
    rows = []
    for path in sorted((out / "_reports").glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            rec = json.loads(line)
            rows.append((rec["collection"], rec["item_id"], rec["status"], rec["reason"]))
    return sorted(rows)


def _run_driver(tmp_path, out, *, stdout, stderr, unbuffered=False, tool=None, env=None):
    argv = [sys.executable]
    if unbuffered:
        argv.append("-u")
    argv += [str(_driver_script(tmp_path)), "--out", str(out), "--jobs", "1"]
    if tool is not None:
        argv += ["--tool", str(tool)]
    proc = subprocess.run(argv, stdout=stdout, stderr=stderr, env=env)
    return proc.returncode


@pytest.fixture
def baseline(tmp_path):
    """A healthy run of the same driver: the answer every broken run must match."""
    out = tmp_path / "baseline"
    with (tmp_path / "b.out").open("w") as o, (tmp_path / "b.err").open("w") as e:
        code = _run_driver(tmp_path, out, stdout=o, stderr=e)
    return code, _ledger_rows(out)


@pytest.mark.parametrize("stream", ["stdout", "stderr"])
@pytest.mark.parametrize("failure", ["full", "pipe"])
@pytest.mark.parametrize("unbuffered", [False, True])
def test_a_broken_stream_changes_neither_the_ledger_nor_the_exit_code(
    tmp_path, baseline, stream, failure, unbuffered
):
    # The acceptance test for the whole class: whatever the log does, the
    # measurement and the exit code are the ones the healthy run produced.
    if failure == "full" and not Path("/dev/full").exists():
        pytest.skip("needs Linux /dev/full")
    baseline_code, baseline_rows = baseline
    assert baseline_code == 0 and baseline_rows, "the baseline run must itself be sound"

    out = tmp_path / f"{stream}-{failure}-{unbuffered}"
    handles = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
    # Closed in the `finally` below; a `with` cannot span the two shapes.
    broken = (
        Path("/dev/full").open("w")  # noqa: SIM115
        if failure == "full"
        else _stream_on_a_closed_pipe()
    )
    handles[stream] = broken
    try:
        code = _run_driver(tmp_path, out, unbuffered=unbuffered, **handles)
    finally:
        with contextlib.suppress(Exception):
            broken.close()

    assert code == baseline_code, f"a broken {stream} changed the exit code to {code}"
    assert _ledger_rows(out) == baseline_rows, f"a broken {stream} changed the measurement"


def test_an_unwritable_config_changes_no_conversion_outcome(tmp_path, baseline):
    # Instance (b) end to end: `_configs` is a regular file, the shape of a
    # read-only or full volume under the mirror. Every item still converts, and
    # not one collection may be published as a conversion failure.
    baseline_code, baseline_rows = baseline
    out = tmp_path / "blocked"
    out.mkdir()
    (out / "_configs").write_text("not a directory")

    code = _run_driver(tmp_path, out, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    assert code == baseline_code
    rows = _ledger_rows(out)
    converted = [r for r in rows if r[2] == "converted"]
    assert converted == [r for r in baseline_rows if r[2] == "converted"]
    assert not [r for r in rows if r[2] == "failed"], f"no conversion failed: {rows}"
    assert {r[3] for r in rows if r[2] == driver.ENVIRONMENT} == {driver.ENVIRONMENT}


def test_a_tool_that_ran_out_of_disk_changes_no_conversion_outcome(tmp_path, baseline):
    # The same property one process further out, end to end: the real
    # `update_collection` runs a `city3dstac` stand-in that reports a full
    # volume and exits 1, exactly as the real tool would. Every item still
    # converts, and the collections it could not aggregate must not be
    # published as unconvertible.
    baseline_code, baseline_rows = baseline
    tool = tmp_path / "fake-city3dstac"
    tool.write_text(
        "#!/usr/bin/env python3\nimport sys\n"
        "sys.stderr.write('Error: I/O error: No space left on device (os error 28)\\n')\n"
        "sys.exit(1)\n",
        encoding="utf-8",
    )
    tool.chmod(tool.stat().st_mode | stat.S_IXUSR)
    out = tmp_path / "tool-enospc"

    code = _run_driver(
        tmp_path,
        out,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        tool=tool,
        env={**os.environ, "C2CP_REAL_AGGREGATION": "1"},
    )

    assert code == baseline_code
    rows = _ledger_rows(out)
    assert [r for r in rows if r[2] == "converted"] == [
        r for r in baseline_rows if r[2] == "converted"
    ]
    assert not [r for r in rows if r[2] == "failed"], f"no conversion failed: {rows}"
    assert {r[3] for r in rows if r[2] == driver.ENVIRONMENT} == {driver.ENVIRONMENT}


def test_a_full_volume_during_a_download_is_never_a_download_failure(tmp_path, baseline):
    # Nothing stubbed but the catalogue: real `process_item`, real writes, a
    # real kernel refusing them. `download_failed` here would be a published
    # claim that three origins were unavailable, on a run where none was asked.
    baseline_code, _ = baseline
    out = tmp_path / "full-volume"

    code = _run_driver(
        tmp_path,
        out,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env={**os.environ, "C2CP_FULL_VOLUME": "1"},
    )

    assert code == baseline_code
    rows = _ledger_rows(out)
    assert rows, "the run must still have recorded what happened"
    assert not [r for r in rows if r[3] == "download_failed"], f"the origin was never asked: {rows}"
    assert {r[2] for r in rows} == {driver.ENVIRONMENT}, rows


# --- the final review's fix wave ---------------------------------------------


def test_a_root_that_resolves_no_collections_is_a_non_zero_exit(tmp_path, monkeypatch):
    # "Reports success while having measured nothing": the root was reachable
    # and yielded nothing, so the run has no measurement to stand behind — and
    # an operator piping the summary somewhere sees an empty table and a 0.
    monkeypatch.setattr(driver.discover, "collection_ids", lambda base, client: ([], None))
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)

    assert driver.run(_config(tmp_path)) == 1


def test_explicit_collections_are_still_run_when_the_root_lists_none(tmp_path, monkeypatch):
    # The zero-collection guard is about *resolution*; an operator who named the
    # collections has already said what to measure.
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    seen = []
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: seen.append(cid))

    assert driver.run(_config(tmp_path), collections=["alpha"]) == 0
    assert seen == ["alpha"]


@pytest.mark.parametrize(
    "exc",
    [
        pytest.param(gzip.BadGzipFile("Not a gzipped file (b'<!')"), id="corrupt-gzip"),
        pytest.param(OSError(errno.ENAMETOOLONG, "File name too long"), id="over-long-member-name"),
        pytest.param(OSError(errno.EISDIR, "Is a directory"), id="member-shaped-like-a-directory"),
    ],
)
def test_a_broken_payload_is_a_data_failure_not_an_environment_one(tmp_path, monkeypatch, exc):
    # `gzip.BadGzipFile` IS an `OSError` (with no errno at all), and a zip
    # member with a 400-character name raises ENAMETOOLONG. Both are facts about
    # the payload, and routing them to the environment withholds real data
    # failures from the histogram — all 8,941 `netherlands-3d-bag` items are
    # `.json.gz`, so this is not a rare shape.
    monkeypatch.setattr(driver, "process_item", lambda item, **k: (_ for _ in ()).throw(exc))
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.convert_items(
        [Item("c", "i", "u", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert ledger.histogram() == {"convert_failed": 1}
    assert ledger.environment_failures() == {}
    assert state.environment_seen == 0


@pytest.mark.parametrize(
    "code", [errno.ECONNRESET, errno.EPIPE, errno.ETIMEDOUT, errno.ECONNREFUSED, errno.EHOSTUNREACH]
)
def test_a_raw_socket_failure_is_a_download_failure(tmp_path, monkeypatch, code):
    # Under an errno gate a socket `OSError` that `httpx` did not wrap would
    # fall through to the catch-all and be published as the converter's fault.
    # The origin dropping the connection is the origin's fault.
    def refused(item, **kwargs):
        raise OSError(code, os.strerror(code))

    monkeypatch.setattr(driver, "process_item", refused)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.convert_items(
        [Item("c", "i", "u", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert ledger.histogram() == {"download_failed": 1}
    assert state.environment_seen == 0


@pytest.mark.parametrize(
    "code",
    [errno.ENOSPC, errno.EROFS, errno.EDQUOT, errno.EMFILE, errno.ENFILE, errno.EFBIG, errno.EIO],
)
def test_a_host_errno_is_still_an_environment_failure(tmp_path, monkeypatch, code):
    def host(item, **kwargs):
        raise OSError(code, os.strerror(code))

    monkeypatch.setattr(driver, "process_item", host)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.convert_items(
        [Item("c", "i", "u", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert ledger.histogram() == {}
    assert ledger.environment_failures() == {"c": 1}
    assert state.environment_seen == 1


def test_a_converter_that_ran_out_of_disk_is_not_a_conversion_failure(tmp_path, monkeypatch):
    # The converter writes effectively all of the multi-terabyte output, so it
    # is what meets a full volume first. Recording that as `convert_failed`
    # stamps every remaining item unconvertible, with no banner and no
    # environment column to give the operator a clue.
    def out_of_disk(binary, inputs, out_dir, crs, timeout):
        raise HostFailure("update-cityparquet failed: No space left on device (os error 28)")

    _stub_conversion(monkeypatch, run_convert=out_of_disk)
    ledger = Ledger(tmp_path / "_reports")
    state = driver.RunState()

    driver.convert_items(
        [Item("c", "i", "https://example.invalid/a.gml", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )

    assert ledger.histogram() == {}, "the converter's full disk says nothing about this dataset"
    assert ledger.environment_failures() == {"c": 1}
    assert state.environment_seen == 1
    written = json.loads((tmp_path / "_reports" / "c.jsonl").read_text())
    assert written["status"] == driver.ENVIRONMENT
    assert written["bytes"] == 2, "what the run moved before the volume filled is still a cost"


def test_a_collection_that_converted_nothing_is_never_aggregated(tmp_path, monkeypatch):
    # `city3dstac` exits "No STAC item files provided" for an empty items dir,
    # and the orchestrator ledgered that as a second, collection-level
    # `convert_failed`. Roughly half the catalogue would gain one phantom
    # conformance record — a fact about the driver, not about the data.
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})
    monkeypatch.setattr(
        driver.discover,
        "enumerate_items",
        lambda base, api, cid, collection, client, dropped=None: (
            [Item(cid, "it0", "u", None, None)],
            None,
        ),
    )

    def always_fails(item, **kwargs):
        raise convert.ConvertError("no_crs", "declares no CRS")

    monkeypatch.setattr(driver, "process_item", always_fails)
    aggregated = []
    monkeypatch.setattr(
        driver.aggregate, "write_config", lambda config, dest: aggregated.append(dest) or dest
    )
    monkeypatch.setattr(
        driver.aggregate,
        "update_collection",
        lambda *a, **k: aggregated.append("update") or True,
    )
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["lux"], ledger=ledger, config=_config(tmp_path, jobs=1))

    assert aggregated == [], "there is nothing on disk to aggregate"
    assert ledger.histogram() == {"no_crs": 1}, "one item failed, so one record"
    assert ledger.counts("lux") == {"failed": 1}


def test_the_documents_the_listing_lost_each_reach_the_ledger(tmp_path, monkeypatch):
    # Reproduced by the reviewer: listed 5, returned 2 — and the other three
    # reached no record at all, so the histogram's denominator shrank silently.
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})

    def enumerate_items(base, api, cid, collection, client, dropped=None):
        if dropped is not None:
            dropped.extend([f"{cid}/items/lost{n}.json" for n in range(3)])
        return [Item(cid, f"i{n}", "u", None, None) for n in range(2)], None

    monkeypatch.setattr(driver.discover, "enumerate_items", enumerate_items)
    monkeypatch.setattr(driver, "process_item", lambda item, **k: 1)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)
    monkeypatch.setattr(driver.aggregate, "update_collection", lambda *a, **k: True)
    ledger = Ledger(tmp_path / "_reports")

    driver.run_collections(["c"], ledger=ledger, config=_config(tmp_path, jobs=1))

    assert ledger.histogram() == {"download_failed": 3}
    assert ledger.counts("c") == {"converted": 2, "failed": 3}
    rows = [
        json.loads(line) for line in (tmp_path / "_reports" / "c.jsonl").read_text().splitlines()
    ]
    assert {r["item_id"] for r in rows if r["status"] == "failed"} == {
        "lost0.json",
        "lost1.json",
        "lost2.json",
    }


def test_the_summary_reports_what_was_discovered(tmp_path, monkeypatch, capsys):
    # A collection whose enumeration was truncated must not be able to look
    # complete: five discovered against two recorded says so at a glance.
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver.discover, "fetch_collection", lambda base, cid, client: {"id": cid})
    monkeypatch.setattr(
        driver.discover,
        "enumerate_items",
        lambda base, api, cid, collection, client, dropped=None: (
            [Item(cid, f"i{n}", "u", None, None) for n in range(5)],
            None,
        ),
    )
    monkeypatch.setattr(driver, "process_item", lambda item, **k: 1)
    monkeypatch.setattr(driver.aggregate, "write_config", lambda config, dest: dest)
    monkeypatch.setattr(driver.aggregate, "update_collection", lambda *a, **k: True)

    config = _config(tmp_path, limit_per_collection=2)
    assert driver.run(config, collections=["c"]) == 0

    out = capsys.readouterr().out
    assert re.search(r"^c\s+5\s+2\s+0\s+0\s+0$", out, re.M), out
    rows = list(csv.DictReader((config.out / "_reports" / "summary.csv").open()))
    assert rows[0]["discovered"] == "5"


def test_the_environment_banner_survives_a_ledger_that_could_not_record_it(
    tmp_path, monkeypatch, capsys
):
    # The case the summary's docstring names loudest: the reports volume fills,
    # so the ledger cannot record the environment failure that filled it and
    # `totals[ENVIRONMENT]` is 0. The in-process count is then the only thing
    # standing between the operator and a clean-looking table.
    ledger = Ledger(tmp_path / "_reports")
    _break_ledger(monkeypatch, ledger)
    state = driver.RunState()

    driver.convert_items(
        [Item("c", "i", "u", None, None)],
        ledger=ledger,
        config=_config(tmp_path, jobs=1),
        state=state,
    )
    assert state.environment_seen == 1
    assert ledger.counts("c") == {}, "the ledger is the thing that failed"

    driver.print_summary(ledger, state)

    out = capsys.readouterr().out
    assert "1 environment failure" in out, out
    assert "this run is incomplete" in out, out


def test_a_crs_override_for_an_unusable_collection_id_is_rejected():
    # One typo silently turns a whole collection into `no_crs` conformance
    # facts — 8,941 items for 3DBAG.
    for bad in ["netherlands 3dbag=EPSG:28992", "../../etc=EPSG:1"]:
        with pytest.raises(SystemExit, match="--crs"):
            driver.config_from_args(driver.parse_args(["--crs", bad]))


@pytest.mark.parametrize("bad", ["rotterdam-3d=banana", "rotterdam-3d=epsg:28992", "a=EPSG:"])
def test_a_crs_override_that_is_not_an_epsg_code_is_rejected(bad):
    with pytest.raises(SystemExit, match="--crs"):
        driver.config_from_args(driver.parse_args(["--crs", bad]))


def test_a_valid_crs_override_still_reaches_the_config():
    config = driver.config_from_args(driver.parse_args(["--crs", "rotterdam-3d=EPSG:28992"]))
    assert config.crs_by_collection == {"rotterdam-3d": "EPSG:28992"}


def test_a_crs_override_naming_no_attempted_collection_is_reported(tmp_path, monkeypatch, capsys):
    # A `--crs` that matches nothing is a typo that will not be noticed for
    # hours, and its collection converts as `no_crs` throughout.
    monkeypatch.setattr(driver, "aggregate_all", lambda config, state: None)
    monkeypatch.setattr(driver, "convert_collection", lambda cid, **k: None)
    config = _config(tmp_path, crs_by_collection={"typo-3d": "EPSG:1234"})

    assert driver.run(config, collections=["rotterdam-3d"]) == 0
    assert "typo-3d" in capsys.readouterr().err


def test_the_histogram_subcommand_reduces_the_cumulative_ledger(tmp_path, capsys):
    # The README names the JSONL as the coverage evidence and tells the reader
    # to roll it up themselves; a resumed run appends a SECOND record for a
    # previously failed item, so an ad-hoc roll-up double-counts. The published
    # number must come from reviewed code.
    reports = tmp_path / "_reports"
    first = Ledger(reports)
    first.record(driver.Record("a", "1", "failed", reason="download_failed"))
    first.record(driver.Record("a", "2", "failed", reason="no_crs"))
    Ledger(reports).record(driver.Record("a", "1", "converted"))

    assert driver.main(["histogram", str(reports)]) == 0

    out = capsys.readouterr().out
    assert re.search(r"^\s+1\s+no_crs$", out, re.M), out
    assert "download_failed" not in out, "the retried item's stale failure must not be counted"
    assert re.search(r"^\s*items\s+2$", out, re.M), out


def test_the_histogram_subcommand_reports_a_missing_reports_directory(tmp_path, capsys):
    assert driver.main(["histogram", str(tmp_path / "nowhere")]) == 1
    assert "nowhere" in capsys.readouterr().err
