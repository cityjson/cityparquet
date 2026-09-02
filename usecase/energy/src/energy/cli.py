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
    from .errors import EnergyError

    args = build_parser().parse_args(argv)
    try:
        if args.command == "features":
            from .run import run_features

            summary = run_features(
                args.input, args.lod, args.output,
                faces_out=args.faces, validate_out=args.validate,
                flat_tilt_deg=args.flat_tilt_deg,
                ext_dir=args.ext_dir,
            )
            print(f"{summary.n_buildings} buildings, {summary.n_parts} parts "
                  f"({summary.n_null_geometry} null-geometry parts skipped, "
                  f"{summary.n_buildings_missing_geometry} buildings without "
                  f"usable geometry, {summary.n_open_solids} open solids flagged)")
            for path in summary.outputs:
                print(f"wrote {path}")
        else:
            from .screen import load_params, screen_features
            import duckdb

            table = screen_features(args.features, load_params(args.params),
                                    hdd=args.hdd, year_before=args.year_before,
                                    sv_above=args.sv_above, top=args.top)
            con = duckdb.connect()
            con.register("screen_t", table)
            con.execute(
                f"COPY screen_t TO '{args.output}' (FORMAT PARQUET, COMPRESSION ZSTD)"
            )
            print(f"{table.num_rows} buildings ranked")
            print(f"wrote {args.output}")
        return 0
    except EnergyError as err:
        import sys as _sys

        print(f"energy: {err}", file=_sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
