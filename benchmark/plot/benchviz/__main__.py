"""``python -m benchviz [prep|html|figures|all] [paths]``.

Every path is a flag with a default inside this repository (``benchviz/paths.py``),
so the no-argument form charts the committed benchmark CSVs into
``benchmark/summary/`` and a caller elsewhere — the paper workspace, which wants the
page in its docs tree and the figures in ``paper/assets/bench`` — overrides only
what it needs.
"""

from __future__ import annotations

import argparse
import json
import sys
import tomllib
from pathlib import Path

from . import paths, prep


def _resolved(args: argparse.Namespace) -> tuple[Path, Path, Path]:
    """The (data, html, figures) paths this invocation writes.

    ``--out`` moves all three at once; the individual flags win over it, which
    is what lets the paper workspace redirect the page and the figures to two
    unrelated directories while the JSON stays wherever ``--out`` put it.
    """
    data, html, figures = (
        paths.derive(args.out or (args.data_root / "summary"))
        if args.out or args.data_root
        else (paths.DEFAULT_DATA_PATH, paths.DEFAULT_HTML_PATH, paths.DEFAULT_FIGURES_DIR)
    )
    return (args.data or data, args.html or html, args.figures or figures)


def _bench_dir(args: argparse.Namespace) -> Path:
    return (
        args.bench_dir
        if args.bench_dir
        else (args.data_root / "formats" if args.data_root else paths.DEFAULT_BENCH_DIR)
    )


def _cmd_prep(args: argparse.Namespace) -> None:
    data, _, _ = _resolved(args)
    prep.main(prep.Inputs(_bench_dir(args)), out_path=data)
    payload = json.loads(data.read_text(encoding="utf-8"))
    families = set(args.families.split(",")) if args.families else None
    datasets = set(args.datasets.split(",")) if args.datasets else None
    if datasets:
        manifest_path = paths.BENCHMARK_ROOT / "manifest.toml"
        if manifest_path.exists():
            manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
            aliases = {
                key: item.get("source", "").removesuffix(".city.jsonl").removesuffix(".city.json")
                for key, item in manifest.get("datasets", {}).items()
            }
            datasets |= {aliases[name] for name in list(datasets) if name in aliases}
        payload["datasets"] = [d for d in payload.get("datasets", []) if d.get("id") in datasets]
        for field in ("read", "sizes", "ordering"):
            payload[field] = [r for r in payload.get(field, []) if r.get("dataset") in datasets]
        for axis in payload.get("scaling", {}).values():
            if isinstance(axis, dict):
                axis["records"] = [
                    r for r in axis.get("records", []) if r.get("dataset") in datasets
                ]
                axis["sizes"] = [r for r in axis.get("sizes", []) if r.get("dataset") in datasets]
        if payload["databases"].get("dataset") not in datasets:
            payload["databases"] = {"baseline": "3dcitydb", "records": [], "sizes": []}
    if families:
        if "formats" not in families:
            payload["read"] = []
        if "sizes" not in families:
            payload["sizes"] = []
        for name in ("codec", "rowgroup"):
            if name not in families:
                payload["scaling"][name] = {"records": [], "sizes": [], "gaps": [], "variants": []}
        if "databases" not in families:
            payload["databases"] = {"baseline": "3dcitydb", "records": [], "sizes": []}
    data.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    coverage = {
        "families": {
            "formats": {
                "present": bool(payload["read"]),
                "metrics": sorted({r.get("scenario_key") for r in payload["read"]}),
            },
            "sizes": {
                "present": bool(payload["sizes"]),
                "metrics": ["bytes"] if payload["sizes"] else [],
            },
            "codec": {
                "present": bool(payload["scaling"]["codec"]["records"]),
                "metrics": sorted(
                    {r.get("measure") for r in payload["scaling"]["codec"]["records"]}
                ),
            },
            "rowgroup": {
                "present": bool(payload["scaling"]["rowgroup"]["records"]),
                "metrics": sorted(
                    {r.get("measure") for r in payload["scaling"]["rowgroup"]["records"]}
                ),
            },
            "databases": {
                "present": bool(payload["databases"]["records"] or payload["databases"]["sizes"]),
                "metrics": sorted({r.get("scenario") for r in payload["databases"]["records"]}),
            },
        }
    }
    (data.parent / "completeness.json").write_text(
        json.dumps(coverage, indent=2) + "\n", encoding="utf-8"
    )


def _cmd_html(args: argparse.Namespace) -> None:
    from . import html  # lazy: only this stage needs the template module

    data, out, figures = _resolved(args)
    html.main(data_path=data, out_path=out, figures_dir=figures)


def _cmd_figures(args: argparse.Namespace) -> None:
    from . import figures  # lazy: the only stage that needs matplotlib

    data, _, out_dir = _resolved(args)
    figures.main(data_path=data, out_dir=out_dir)


def _cmd_all(args: argparse.Namespace) -> None:
    _cmd_prep(args)

    _cmd_figures(args)

    _cmd_html(args)


def _cmd_summary(args: argparse.Namespace) -> None:
    _cmd_all(args)


def build_parser() -> argparse.ArgumentParser:
    # The path flags live on a shared parent parser, so they are written AFTER
    # the subcommand (`benchviz all --out X`) — the order a justfile recipe and
    # a shell reader both expect.
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument(
        "--data-root",
        type=Path,
        metavar="DIR",
        help="external benchmark root with formats/, databases/ and summary/",
    )

    common.add_argument(
        "--bench-dir",
        type=Path,
        default=None,
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
    common.add_argument(
        "--families",
        metavar="CSV",
        help="selected families: sizes,formats,codec,rowgroup,databases",
    )
    common.add_argument("--datasets", metavar="CSV", help="selected dataset IDs")

    common.add_argument("--html", type=Path, metavar="FILE", help="summary page path")

    common.add_argument("--figures", type=Path, metavar="DIR", help="static figure directory")

    parser = argparse.ArgumentParser(
        prog="benchviz",
        description=(
            "Build the CityParquet benchmark visualisations from the result CSVs "
            "an earlier `just bench` / `just codec-bench` / `just rowgroup-bench` / "
            "`just sizes` run left in benchmark/formats/. Runs no benchmark of its own."
        ),
    )
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("prep", parents=[common], help="CSVs -> bench_data.json").set_defaults(
        func=_cmd_prep
    )
    sub.add_parser(
        "html", parents=[common], help="bench_data.json -> bench-summary.html"
    ).set_defaults(func=_cmd_html)
    sub.add_parser(
        "figures", parents=[common], help="bench_data.json -> *.svg + *.png"
    ).set_defaults(func=_cmd_figures)
    sub.add_parser("all", parents=[common], help="prep + html + figures").set_defaults(
        func=_cmd_all
    )
    sub.add_parser(
        "summary", parents=[common], help="prep + figures + self-contained HTML"
    ).set_defaults(func=_cmd_summary)
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
