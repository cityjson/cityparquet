import csv
from concurrent.futures import ThreadPoolExecutor

import pytest

from catalog2cityparquet.ledger import CONFORMANCE_REASONS, ENVIRONMENT, REASONS, Ledger, Record


def test_records_are_appended_as_jsonl_per_collection(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("rotterdam-3d", "a", "converted", seconds=1.5, bytes=10))
    ledger.record(Record("rotterdam-3d", "b", "failed", reason="convert_failed", error="boom"))

    lines = (tmp_path / "rotterdam-3d.jsonl").read_text().strip().splitlines()
    assert len(lines) == 2


def test_histogram_counts_reasons_across_collections(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "failed", reason="no_crs"))
    ledger.record(Record("b", "2", "failed", reason="no_crs"))
    ledger.record(Record("b", "3", "failed", reason="download_failed"))

    assert ledger.histogram() == {"no_crs": 2, "download_failed": 1}


def test_summary_csv_rolls_up_per_collection(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "converted"))
    ledger.record(Record("a", "2", "failed", reason="no_crs"))
    ledger.record(Record("a", "3", "skipped", reason="duplicate_bundle"))

    path = ledger.write_summary()
    rows = list(csv.DictReader(path.open()))
    assert len(rows) == 1
    assert rows[0]["collection"] == "a"
    assert rows[0]["converted"] == "1"
    assert rows[0]["failed"] == "1"
    assert rows[0]["skipped"] == "1"


def test_an_unknown_reason_is_rejected(tmp_path):
    # The vocabulary is closed so the histogram stays meaningful; a typo must
    # fail loudly rather than create a new silent category.
    ledger = Ledger(tmp_path)
    with pytest.raises(ValueError, match="unknown reason"):
        ledger.record(Record("a", "1", "failed", reason="whoops"))


def test_the_vocabulary_is_the_documented_closed_set():
    assert CONFORMANCE_REASONS == {  # noqa: SIM300 — a set literal on the right reads plainly
        "download_failed",
        "unsupported_archive",
        "unsupported_citygml_version",
        "unsupported_cityjson_version",
        "no_crs",
        "geographic_crs",
        "convert_failed",
        "empty_collection",
        "duplicate_bundle",
        "stale_item_index",
    }
    # The environment reason is in the closed set but outside the conformance
    # vocabulary: it says what this machine did, never what the data is.
    assert sorted(REASONS - CONFORMANCE_REASONS) == [ENVIRONMENT]
    assert CONFORMANCE_REASONS.issubset(REASONS)


def test_an_environment_failure_never_enters_the_conformance_histogram(tmp_path):
    # The histogram is what the paper quotes. A full disk or a missing tool is
    # a fact about this machine; counting it as a conversion outcome would make
    # the published number a fabrication.
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "failed", reason="no_crs"))
    ledger.record(Record("a", "-", ENVIRONMENT, reason=ENVIRONMENT, error="disk full"))

    assert ledger.histogram() == {"no_crs": 1}
    assert ledger.environment_failures() == {"a": 1}
    assert ledger.counts("a") == {"failed": 1, ENVIRONMENT: 1}


def test_the_environment_status_and_reason_travel_together(tmp_path):
    # Two columns, one concept: allowing them to drift would let an environment
    # failure be tallied in the `failed` column after all.
    ledger = Ledger(tmp_path)
    with pytest.raises(ValueError, match="travel together"):
        ledger.record(Record("a", "1", "failed", reason=ENVIRONMENT))
    with pytest.raises(ValueError, match="travel together"):
        ledger.record(Record("a", "1", ENVIRONMENT, reason="no_crs"))
    with pytest.raises(ValueError, match="travel together"):
        ledger.record(Record("a", "1", ENVIRONMENT))


def test_summary_csv_keeps_environment_out_of_the_failed_column(tmp_path):
    ledger = Ledger(tmp_path)
    ledger.record(Record("a", "1", "converted"))
    ledger.record(Record("a", "-", ENVIRONMENT, reason=ENVIRONMENT, error="read-only"))

    rows = list(csv.DictReader(ledger.write_summary().open()))
    assert rows[0]["converted"] == "1"
    assert rows[0]["failed"] == "0", "the machine failing is not the data failing"
    assert rows[0][ENVIRONMENT] == "1"


@pytest.mark.parametrize("cid", ["../escape", "a/b", "..", "", "a\\b"])
def test_a_collection_id_cannot_escape_the_reports_dir(tmp_path, cid):
    # The collection id is interpolated into `<collection>.jsonl`, and it comes
    # from a published catalogue rather than from us; a separator or `..` must
    # fail loudly instead of writing outside the reports directory.
    ledger = Ledger(tmp_path / "reports")
    with pytest.raises(ValueError, match="collection id"):
        ledger.record(Record(cid, "1", "converted"))
    assert list((tmp_path / "reports").iterdir()) == []


def test_concurrent_records_are_not_interleaved(tmp_path):
    # Items are converted through a thread pool, so every worker writes into
    # the same collection log; each record must land as one intact line.
    ledger = Ledger(tmp_path)
    total = 200
    with ThreadPoolExecutor(max_workers=8) as pool:
        list(
            pool.map(
                lambda n: ledger.record(Record("a", str(n), "converted", bytes=n)),
                range(total),
            )
        )

    lines = (tmp_path / "a.jsonl").read_text().strip().splitlines()
    assert len(lines) == total
    assert ledger.counts("a") == {"converted": total}
