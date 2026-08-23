"""The plotter's CSV contract must match what the coordinator actually writes.

These drifted apart once already: the coordinator grew `bytes_read` and
`http_requests` for the HTTP transport, and neither the plotter nor the DuckDB
baseline script followed. The plotter's gate is a strict equality check, so the
symptom was silent - every coordinator CSV was skipped as "not a
read-benchmark CSV" and the charts came out empty rather than wrong.

Both tests read the header literal out of its own source file rather than
restating the column list here: a fourth copy of the list is exactly what let
the first three drift apart.
"""

import re
from pathlib import Path

from readbench_plot import CSV_HEADER

# benchmark/plot/tests/ -> the monorepo root; the harness crate this contract
# is read from lives in the other half of the tree.
REPO = Path(__file__).resolve().parents[3] / "lib" / "cityparquet-rs"


def _coordinator_header() -> list[str]:
    """The header literal the Rust coordinator writes, read from its source."""
    src = (REPO / "crates/cityparquet-readbench/src/coordinator.rs").read_text()
    m = re.search(r'const CSV_HEADER: &str = "(.*?)";', src, re.S)
    assert m, "could not find CSV_HEADER in coordinator.rs"
    # The literal is line-continued with a trailing backslash; rejoin it.
    return m.group(1).replace("\\\n", "").strip().split(",")


def test_plotter_header_matches_the_coordinator():
    # Coordinator first: it is the authority, so it reads as the expectation
    # (and ruff's SIM300 rejects the other order).
    assert _coordinator_header() == CSV_HEADER


def test_duckdb_script_header_matches_the_coordinator():
    src = (REPO / "scripts/readbench_duckdb.sh").read_text()
    m = re.search(r'^CSV_HEADER=(?:"|\')(.+?)(?:"|\')$', src, re.M)
    assert m, "could not find CSV_HEADER in readbench_duckdb.sh"
    assert m.group(1).split(",") == _coordinator_header()


def _write_csv(path: Path, header: list[str], rows: list[list[str]]) -> Path:
    lines = [",".join(header), *(",".join(r) for r in rows)]
    path.write_text("\n".join(lines) + "\n")
    return path


# One well-formed row per gate test: the values are irrelevant to the gate,
# only the header shape is under test.
_ROW = [
    "delft.city.jsonl",
    "cityparquet",
    "full-read",
    "",
    "2231",
    "0.25",
    "0.0",
    "38435421",
    "55263232",
    "1",
    "",
    "",
    "",
]


def test_loader_accepts_a_coordinator_csv(tmp_path):
    """The exact shape the coordinator writes must plot, not be skipped."""
    from readbench_plot.plot import load_csv

    path = _write_csv(tmp_path / "rb.csv", _coordinator_header(), [_ROW])
    df = load_csv(path)
    assert df is not None
    assert len(df) == 1


def test_loader_accepts_a_future_trailing_column(tmp_path):
    """A column appended later degrades gracefully rather than blanking the chart."""
    from readbench_plot.plot import load_csv

    header = [*_coordinator_header(), "some_future_column"]
    path = _write_csv(tmp_path / "rb.csv", header, [[*_ROW, "42"]])
    df = load_csv(path)
    assert df is not None
    assert len(df) == 1


def test_loader_still_skips_a_foreign_csv(tmp_path):
    """The prefix check must not start swallowing write-benchmark CSVs."""
    from readbench_plot.plot import load_csv

    path = _write_csv(
        tmp_path / "write-bench.csv",
        ["dataset", "variant", "bytes"],
        [["a", "b", "1"]],
    )
    assert load_csv(path) is None
