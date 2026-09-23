"""The read-results CSV header is written in four places; they must agree.

`benchmark/readbench/src/coordinator.rs`'s `CSV_HEADER` is the single
authority: the coordinator writes the file, and the other two writers only
ever append rows to a CSV it created. `benchmark/scripts/readbench_duckdb.sh`
adds the `duckdb-parquet` rows, `benchmark/scripts/format_write.py` the
`write` ones, and `benchviz.prep.READ_COLUMNS` is what the renderer reads back
by name. Nothing at run time compares the four, and a drifted copy shifts every
column after it silently — which is what this file exists to stop.

It lives in the plotting project because that is the only Python test suite the
benchmark tree has (`just plot-test`); it reads the other three out of their
own sources rather than restating the header itself, so it cannot go stale in
the way it is checking for.
"""

import re
from pathlib import Path

from benchviz import prep

BENCHMARK = Path(__file__).parents[2]
COORDINATOR = BENCHMARK / "readbench" / "src" / "coordinator.rs"
DUCKDB_SH = BENCHMARK / "scripts" / "readbench_duckdb.sh"
FORMAT_WRITE = BENCHMARK / "scripts" / "format_write.py"


def _rust_header() -> list[str]:
    """`const CSV_HEADER: &str = "..."` — a Rust literal split over lines with
    `\\` continuations, each of which eats the newline and the indent after it.
    """
    text = COORDINATOR.read_text(encoding="utf-8")
    match = re.search(r'const CSV_HEADER: &str = "(.*?)";', text, re.DOTALL)
    assert match, f"{COORDINATOR}: no `const CSV_HEADER: &str = \"...\"`"
    return re.sub(r"\\\n\s*", "", match.group(1)).split(",")


def _shell_header() -> list[str]:
    text = DUCKDB_SH.read_text(encoding="utf-8")
    match = re.search(r'^CSV_HEADER="([^"]*)"', text, re.MULTILINE)
    assert match, f"{DUCKDB_SH}: no `CSV_HEADER=\"...\"` assignment"
    return match.group(1).split(",")


def _python_header() -> list[str]:
    text = FORMAT_WRITE.read_text(encoding="utf-8")
    match = re.search(r"^HEADER = \[(.*?)\]", text, re.MULTILINE | re.DOTALL)
    assert match, f"{FORMAT_WRITE}: no `HEADER = [...]` assignment"
    return re.findall(r'"([^"]*)"', match.group(1))


def test_every_writer_of_the_read_results_csv_uses_the_same_header():
    authority = _rust_header()
    assert authority[0] == "dataset" and len(authority) == 16, authority
    assert _shell_header() == authority, DUCKDB_SH
    assert _python_header() == authority, FORMAT_WRITE


def test_the_renderer_reads_a_leading_prefix_of_that_header():
    """`prep._check_columns` requires `READ_COLUMNS` as a PREFIX and allows
    appended extras, so a column renamed or reordered inside the prefix is the
    drift it catches — and the prefix itself has to stay the header's own.
    """
    authority = _rust_header()
    assert authority[: len(prep.READ_COLUMNS)] == prep.READ_COLUMNS
