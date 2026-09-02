"""Command-line entry point for the energy use-case tool."""
from __future__ import annotations

import argparse
import sys


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="energy",
        description="UBEM feature extraction and retrofit screening on CityParquet.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    f = sub.add_parser("features", help="extract per-building geometric features")
    f.add_argument("--input", required=True,
                   help="path, glob or s3:// URL of building.parquet file(s)")
    f.add_argument("--lod", default="2.2", help="LoD to read (default: 2.2)")
    f.add_argument("--output", default="features.parquet")
    f.add_argument("--faces", default=None,
                   help="also write the per-face table (ST_3DFaces prototype)")
    f.add_argument("--validate", default=None,
                   help="write a JSON comparison against 3DBAG b3_* columns")
    f.add_argument("--flat-tilt-deg", type=float, default=5.0,
                   help="roof tilt at or below this counts as flat (default: 5)")
    f.add_argument("--ext-dir", default=None,
                   help="directory holding the .duckdb_extension binaries")

    s = sub.add_parser("screen", help="degree-day heat-loss screen and ranking")
    s.add_argument("--features", required=True, help="features.parquet from `features`")
    s.add_argument("--hdd", type=float, default=2900.0,
                   help="heating degree days, K·d (default: 2900, NL base 18°C)")
    s.add_argument("--params", default=None, help="U-value bands TOML (default: built-in)")
    s.add_argument("--year-before", type=int, default=None,
                   help="keep only buildings built before this year")
    s.add_argument("--sv-above", type=float, default=None,
                   help="keep only buildings with S/V above this")
    s.add_argument("--top", type=int, default=None, help="keep only the top-N ranked")
    s.add_argument("--output", default="screen.parquet")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    print(f"energy {args.command}: not implemented yet", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
