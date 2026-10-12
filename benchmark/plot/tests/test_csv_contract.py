"""The read-results CSV header is written in one place and read in another.

`benchmark/readbench/src/coordinator.rs`'s `CSV_HEADER` is the single
authority: the coordinator writes the file, and `benchviz.prep.READ_COLUMNS`
is what the renderer reads back by name. Nothing at run time compares the two,
and a drifted copy shifts every column after it silently — which is what this
file exists to stop.

It lives in the plotting project because the renderer is the reading side
(`just plot-test`); it reads the header out of the coordinator's own source
rather than restating it, so it cannot go stale in the way it is checking for.
"""

import ast
import re
from pathlib import Path

from benchviz import prep

BENCHMARK = Path(__file__).parents[2]
COORDINATOR = BENCHMARK / "readbench" / "src" / "coordinator.rs"
REPORT = BENCHMARK / "databases" / "src" / "citybench" / "report.py"


def _rust_header() -> list[str]:
    """`const CSV_HEADER: &str = "..."` — a Rust literal split over lines with
    `\\` continuations, each of which eats the newline and the indent after it.
    """
    text = COORDINATOR.read_text(encoding="utf-8")
    match = re.search(r'const CSV_HEADER: &str = "(.*?)";', text, re.DOTALL)
    assert match, f'{COORDINATOR}: no `const CSV_HEADER: &str = "..."`'
    return re.sub(r"\\\n\s*", "", match.group(1)).split(",")


def test_the_coordinator_header_is_the_documented_one():
    authority = _rust_header()
    assert authority[0] == "dataset" and len(authority) == 22, authority
    start = authority.index("time_mean_s")
    assert authority[start : start + len(prep.TIMING_BLOCK)] == prep.TIMING_BLOCK, authority


def test_the_renderer_reads_a_leading_prefix_of_that_header():
    """`prep._check_columns` requires `READ_COLUMNS` as a PREFIX and allows
    appended extras, so a column renamed or reordered inside the prefix is the
    drift it catches — and the prefix itself has to stay the header's own.
    """
    authority = _rust_header()
    assert authority[: len(prep.READ_COLUMNS)] == prep.READ_COLUMNS


def _database_columns() -> list[str]:
    """citybench `report.COLUMNS`, evaluated from the module's own top-level
    assignments (importing it would pull in the database project's
    dependencies, which the plotting project does not have).
    """
    tree = ast.parse(REPORT.read_text(encoding="utf-8"))
    assignments = [n for n in tree.body if isinstance(n, (ast.Assign, ast.AnnAssign))]
    namespace: dict = {}
    exec(compile(ast.Module(body=assignments, type_ignores=[]), str(REPORT), "exec"), namespace)
    return list(namespace["COLUMNS"])


def test_the_database_csv_carries_the_same_timing_blocks():
    """Both families write the seven-column timing block in the coordinator's
    order; the database CSV adds the same block for server time, `server_`-prefixed.
    """
    authority = _rust_header()
    start = authority.index("time_mean_s")
    block = authority[start : start + 7]
    columns = _database_columns()
    at = columns.index(block[0])
    assert columns[at : at + 7] == block, columns
    server = [f"server_{c}" for c in block]
    at = columns.index(server[0])
    assert columns[at : at + 7] == server, columns
    assert "time_s" not in columns and "server_time_s" not in columns


def test_the_renderer_reads_the_database_memory_column_by_its_current_name():
    """The loader's memory column is the one citybench writes, in its
    position after `peak_heap_bytes`."""
    columns = _database_columns()
    assert prep.DB_MEMORY_COLUMN == "peak_working_mem_bytes"
    assert "peak_rss_bytes" not in columns
    assert columns[columns.index("peak_heap_bytes") + 1] == prep.DB_MEMORY_COLUMN
    for column in ("size_bytes", "size_bytes_no_index", "status", "notes", "result_count"):
        assert column in columns, column


def test_the_database_fixture_carries_the_current_header():
    fixture = Path(__file__).parent / "fixtures" / "benchviz-databases" / "results"
    header = (fixture / "3dbag_n10000.csv").read_text(encoding="utf-8").splitlines()[0]
    assert header.split(",") == _database_columns()
