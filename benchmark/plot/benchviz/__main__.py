"""``python -m benchviz [prep|html|figures|all] [paths]``.

Every path is a flag with a default inside this repository (``benchviz/paths.py``),
so the no-argument form charts the committed benchmark CSVs into
``benchmark/summary/`` and a caller elsewhere — the paper workspace, which wants the
page in its docs tree and the figures in ``paper/assets/bench`` — overrides only
what it needs.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import paths, prep


def _resolved(args: argparse.Namespace) -> tuple[Path, Path, Path]:
    """The (data, html, figures) paths this invocation writes.

    ``--out`` moves all three at once; the individual flags win over it, which
    is what lets the paper workspace redirect the page and the figures to two
    unrelated directories while the JSON stays wherever ``--out`` put it.
    """
    data, html, figures = (
        paths.derive(args.out)
        if args.out
        else (paths.DEFAULT_DATA_PATH, paths.DEFAULT_HTML_PATH, paths.DEFAULT_FIGURES_DIR)
    )
    return (args.data or data, args.html or html, args.figures or figures)


def _cmd_prep(args: argparse.Namespace) -> None:
    data, _, _ = _resolved(args)
    prep.main(prep.Inputs(args.bench_dir), out_path=data)


def _cmd_html(args: argparse.Namespace) -> None:
    from . import html  # lazy: only this stage needs the template module

    data, out, _ = _resolved(args)
    html.main(data_path=data, out_path=out)


def _cmd_figures(args: argparse.Namespace) -> None:
    from . import figures  # lazy: the only stage that needs matplotlib

    data, _, out_dir = _resolved(args)
    figures.main(data_path=data, out_dir=out_dir)


def _cmd_all(args: argparse.Namespace) -> None:
    _cmd_prep(args)
    _cmd_html(args)
    _cmd_figures(args)


def build_parser() -> argparse.ArgumentParser:
    # The path flags live on a shared parent parser, so they are written AFTER
    # the subcommand (`benchviz all --out X`) — the order a justfile recipe and
    # a shell reader both expect.
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--bench-dir",
        type=Path,
        default=paths.DEFAULT_BENCH_DIR,
        metavar="DIR",
        help="benchmark results to read (default: this repo's benchmark/formats/)",
    )
    common.add_argument(
        "--out",
        type=Path,
        metavar="DIR",
        help="write all three outputs under DIR (default: benchmark/summary/)",
    )
    common.add_argument("--data", type=Path, metavar="FILE", help="bench_data.json path")
    common.add_argument("--html", type=Path, metavar="FILE", help="summary page path")
    common.add_argument(
        "--figures", type=Path, metavar="DIR", help="static figure directory"
    )

    parser = argparse.ArgumentParser(
        prog="benchviz",
        description=(
            "Build the CityParquet benchmark visualisations from the result CSVs "
            "an earlier `just bench` / `just compression-bench` / `just sizes` run "
            "left in benchmark/formats/. Runs no benchmark of its own."
        ),
    )
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser(
        "prep", parents=[common], help="CSVs -> bench_data.json"
    ).set_defaults(func=_cmd_prep)
    sub.add_parser(
        "html", parents=[common], help="bench_data.json -> bench-summary.html"
    ).set_defaults(func=_cmd_html)
    sub.add_parser(
        "figures", parents=[common], help="bench_data.json -> *.svg + *.png"
    ).set_defaults(func=_cmd_figures)
    sub.add_parser(
        "all", parents=[common], help="prep + html + figures"
    ).set_defaults(func=_cmd_all)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        args.func(args)
    except prep.PrepError as exc:
        # A PrepError is a statement about the INPUT CSVs — a missing results
        # directory, an unexpected column, an unparseable value — and every
        # one of them is phrased for the operator who has to fix it. A
        # traceback buries that sentence under frames from this tool's own
        # internals, none of which are the reader's problem.
        print(f"benchviz: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
