"""`python -m readbench_plot RESULTS_DIR` entrypoint."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from readbench_plot.plot import run


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="readbench_plot",
        description=(
            "Render grouped-bar charts (median time_s and peak_heap_bytes per "
            "scenario x format) from cityparquet-readbench result CSVs."
        ),
    )
    parser.add_argument(
        "results_dir",
        type=Path,
        help="directory containing read-benchmark result CSVs (e.g. bench/read_results)",
    )
    args = parser.parse_args()
    sys.exit(run(args.results_dir))


if __name__ == "__main__":
    main()
