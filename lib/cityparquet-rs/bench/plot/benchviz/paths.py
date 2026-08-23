"""Default input and output locations, all derived from this checkout.

One module, because the three stages (prep, html, figures) and the CLI must
agree on them, and because a second copy of ``parents[2]`` is how the old
paper-repo edition of this package ended up hard-wired to a directory layout
outside itself.

Outputs default *inside* ``bench/`` — a `just plot-pretty` run needs no
arguments and writes nothing outside the repository. A caller with somewhere
else to put them (the paper workspace renders the page into its docs tree and
the figures into ``paper/assets/bench``) passes the paths in.

stdlib only: ``prep`` imports this and must stay runnable from a bare Python.
"""

from __future__ import annotations

from pathlib import Path

# bench/plot/benchviz/paths.py -> bench/ is two levels up.
DEFAULT_BENCH_DIR = Path(__file__).resolve().parents[2]
DEFAULT_OUT_DIR = DEFAULT_BENCH_DIR / "summary"
DEFAULT_DATA_PATH = DEFAULT_OUT_DIR / "bench_data.json"
DEFAULT_HTML_PATH = DEFAULT_OUT_DIR / "bench-summary.html"
DEFAULT_FIGURES_DIR = DEFAULT_OUT_DIR / "figures"


def derive(out_dir: Path) -> tuple[Path, Path, Path]:
    """The (data, html, figures) triple rooted at ``out_dir``."""
    return (
        out_dir / DEFAULT_DATA_PATH.name,
        out_dir / DEFAULT_HTML_PATH.name,
        out_dir / DEFAULT_FIGURES_DIR.name,
    )
