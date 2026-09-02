"""Regenerate tile_slice.parquet from a local 3DBAG-as-CityParquet tile.

Usage: uv run python tests/fixtures/make_fixture.py [SOURCE_PARQUET]
Committed output: deterministic 150-building slice (plus their parts).
"""
import sys
from pathlib import Path

import duckdb

DEFAULT_SOURCE = "/data2/hideba/cityparquet_data/10-756-44/building.parquet"
OUT = Path(__file__).parent / "tile_slice.parquet"


def main() -> None:
    source = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_SOURCE
    con = duckdb.connect()
    con.execute(
        """
        COPY (
          WITH picked AS (
            SELECT id FROM read_parquet($src)
            WHERE object_type = 'Building'
              AND b3_volume_lod22 IS NOT NULL
              AND b3_opp_grond IS NOT NULL
            ORDER BY id
            LIMIT 150
          )
          SELECT t.* FROM read_parquet($src) t
          WHERE t.id IN (SELECT id FROM picked)
             OR t.parents[1] IN (SELECT id FROM picked)
          ORDER BY t.id
        ) TO $out (FORMAT PARQUET, COMPRESSION ZSTD)
        """,
        {"src": source, "out": str(OUT)},
    )
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
