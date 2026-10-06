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

import re
from pathlib import Path

from benchviz import prep

BENCHMARK = Path(__file__).parents[2]
COORDINATOR = BENCHMARK / "readbench" / "src" / "coordinator.rs"


def _rust_header() -> list[str]:
    """`const CSV_HEADER: &str = "..."` — a Rust literal split over lines with
    `\\` continuations, each of which eats the newline and the indent after it.
    """
    text = COORDINATOR.read_text(encoding="utf-8")
    match = re.search(r'const CSV_HEADER: &str = "(.*?)";', text, re.DOTALL)
    assert match, f"{COORDINATOR}: no `const CSV_HEADER: &str = \"...\"`"
    return re.sub(r"\\\n\s*", "", match.group(1)).split(",")


def test_the_coordinator_header_is_the_documented_one():
    authority = _rust_header()
    assert authority[0] == "dataset" and len(authority) == 21, authority
    start = authority.index("time_mean_s")
    assert authority[start : start + len(prep.TIMING_BLOCK)] == prep.TIMING_BLOCK, authority


def test_the_renderer_reads_a_leading_prefix_of_that_header():
    """`prep._check_columns` requires `READ_COLUMNS` as a PREFIX and allows
    appended extras, so a column renamed or reordered inside the prefix is the
    drift it catches — and the prefix itself has to stay the header's own.
    """
    authority = _rust_header()
    assert authority[: len(prep.READ_COLUMNS)] == prep.READ_COLUMNS
