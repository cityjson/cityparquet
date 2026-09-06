# Energy Feature-Factory Use-Case Tool — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `usecase/energy/` — a CLI tool that produces UBEM geometric inputs
(heated volume, semantic envelope split, S/V), a degree-day heat-loss screen and a
retrofit ranking from a CityParquet package, per the approved spec
`ai/design-notes/specs/2026-09-02-usecase-energy-design.md`.

**Architecture:** uv Python project; whole-solid metrics via `duckdb-3d` SQL;
per-face semantic split via a Python `ST_3DFaces` prototype reading WKB +
`face_semantics`/`surfaces` from Arrow; screening as plain computation over the
feature table; validation against 3DBAG's own `b3_*` columns.

**Tech Stack:** Python ≥3.11, uv, `duckdb==1.5.4` (pinned), pyarrow, numpy, pytest.

## Global Constraints

- **Working directory for all commands:** monorepo root (the `cityparquet` repo).
- **`duckdb==1.5.4` exactly** — the built extensions (`lib/*/build/release/extension/…`) are v1.5.4 binaries; any other client version fails to load them.
- **Python ≥3.11** (stdlib `tomllib`).
- **Extension paths** (relative to monorepo root):
  `lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension`
  `lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension`
- **Verified data facts** (from a real 3DBAG tile, 2026-09-02): geometry WKB is ISO
  **PolyhedralSurface Z, type code 1015** (polygons type 1003); `Building` rows carry
  attributes + `b3_*` references, `BuildingPart` rows carry geometry;
  `parents[1]` on a part is its Building id; `face_semantics` is `list<int32>`
  indexing the `surfaces` JSON array (`[{"type":"GroundSurface"},
  {"type":"WallSurface",…},…]`); `ST_3DVolume` agreed with `b3_volume_lod22` to
  0.03% on the smoke-test row.
- **Local test tile:** `/data2/hideba/cityparquet_data/10-756-44/building.parquet`
  (source for the committed fixture; never read directly by tests).
- **LoD flag format:** `--lod 2.2` → column suffix `lod2_2`.
- **British English** in all docs and user-facing strings.
- **Commit style:** `<type>(usecase): message` (matches repo convention, e.g.
  `feat(usecase): …`, `test(usecase): …`, `docs(usecase): …`).
- Tests that need the built extensions use the `requires_extensions` marker and
  **skip** (not fail) when the binaries are absent.

---

### Task 1: Project scaffold + CLI skeleton

**Files:**
- Create: `usecase/energy/pyproject.toml`
- Create: `usecase/energy/src/energy/__init__.py`
- Create: `usecase/energy/src/energy/cli.py`
- Test: `usecase/energy/tests/test_cli.py`

**Interfaces:**
- Produces: console script `energy`; `energy.cli.main(argv: list[str] | None = None) -> int`;
  `build_parser() -> argparse.ArgumentParser`. Subcommands `features` and `screen`
  parse all flags but return exit code 2 with "not implemented yet" until Task 11.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_cli.py
from energy.cli import build_parser, main


def test_features_flags_parse():
    args = build_parser().parse_args(
        ["features", "--input", "in/*.parquet", "--lod", "2.2",
         "--output", "f.parquet", "--faces", "faces.parquet",
         "--validate", "report.json", "--flat-tilt-deg", "7.5"]
    )
    assert args.command == "features"
    assert args.input == "in/*.parquet"
    assert args.lod == "2.2"
    assert args.output == "f.parquet"
    assert args.faces == "faces.parquet"
    assert args.validate == "report.json"
    assert args.flat_tilt_deg == 7.5


def test_features_defaults():
    args = build_parser().parse_args(["features", "--input", "x.parquet"])
    assert args.lod == "2.2"
    assert args.output == "features.parquet"
    assert args.faces is None
    assert args.validate is None
    assert args.flat_tilt_deg == 5.0
    assert args.ext_dir is None


def test_screen_flags_parse():
    args = build_parser().parse_args(
        ["screen", "--features", "f.parquet", "--hdd", "3000",
         "--params", "u.toml", "--year-before", "1975",
         "--sv-above", "0.8", "--top", "100", "--output", "s.parquet"]
    )
    assert args.command == "screen"
    assert args.hdd == 3000.0
    assert args.year_before == 1975
    assert args.sv_above == 0.8
    assert args.top == 100


def test_screen_defaults():
    args = build_parser().parse_args(["screen", "--features", "f.parquet"])
    assert args.hdd == 2900.0
    assert args.params is None
    assert args.year_before is None
    assert args.sv_above is None
    assert args.top is None
    assert args.output == "screen.parquet"


def test_main_not_implemented_yet(capsys):
    rc = main(["features", "--input", "x.parquet"])
    assert rc == 2
    assert "not implemented" in capsys.readouterr().err
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_cli.py -v`
Expected: FAIL (ModuleNotFoundError / ImportError — project not scaffolded yet).
Note: the first `uv run` also needs Step 3's `pyproject.toml`, so at this point the
failure may be uv refusing to run at all; that still counts as the red step.

- [ ] **Step 3: Write minimal implementation**

```toml
# usecase/energy/pyproject.toml
[project]
name = "cityparquet-usecase-energy"
version = "0.1.0"
description = "CityParquet use-case tool: UBEM feature extraction and retrofit screening"
requires-python = ">=3.11"
dependencies = [
    "duckdb==1.5.4",
    "pyarrow>=17",
    "numpy>=1.26",
]

[project.scripts]
energy = "energy.cli:main"

[dependency-groups]
dev = ["pytest>=8"]

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[tool.hatch.build.targets.wheel]
packages = ["src/energy"]

[tool.pytest.ini_options]
testpaths = ["tests"]
markers = ["requires_extensions: needs built duckdb-cityjson/duckdb-3d binaries"]
```

```python
# usecase/energy/src/energy/__init__.py
```

```python
# usecase/energy/src/energy/cli.py
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_cli.py -v`
Expected: 5 PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy
git commit -m "feat(usecase): scaffold the energy tool with its CLI surface"
```

---

### Task 2: Committed fixture sliced from a real tile

**Files:**
- Create: `usecase/energy/tests/fixtures/make_fixture.py`
- Create: `usecase/energy/tests/fixtures/tile_slice.parquet` (generated, committed)
- Create: `usecase/energy/tests/fixtures/README.md`
- Create: `usecase/energy/tests/conftest.py`
- Test: `usecase/energy/tests/test_fixture.py`

**Interfaces:**
- Produces: pytest fixture `fixture_path -> pathlib.Path` (in `conftest.py`) pointing
  at `tile_slice.parquet`; the file contains 150 `Building` rows (with
  `b3_volume_lod22` etc.) plus all their `BuildingPart` children.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/conftest.py
from pathlib import Path

import pytest

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture
def fixture_path() -> Path:
    return FIXTURES / "tile_slice.parquet"
```

```python
# usecase/energy/tests/test_fixture.py
import duckdb


def test_fixture_has_buildings_and_parts(fixture_path):
    con = duckdb.connect()
    n = con.sql(
        "SELECT object_type, count(*) FROM read_parquet(?) GROUP BY 1 ORDER BY 1",
        params=[str(fixture_path)],
    ).fetchall()
    kinds = dict(n)
    assert kinds.get("Building", 0) == 150
    assert kinds.get("BuildingPart", 0) >= 150


def test_every_part_links_to_a_building(fixture_path):
    con = duckdb.connect()
    orphans = con.sql(
        """
        SELECT count(*) FROM read_parquet($p) p
        WHERE p.object_type = 'BuildingPart'
          AND p.parents[1] NOT IN
              (SELECT id FROM read_parquet($p) WHERE object_type = 'Building')
        """,
        params={"p": str(fixture_path)},
    ).fetchone()[0]
    assert orphans == 0


def test_parts_have_geometry_and_buildings_have_references(fixture_path):
    con = duckdb.connect()
    row = con.sql(
        """
        SELECT
          count(*) FILTER (object_type = 'BuildingPart' AND geometry_lod2_2 IS NULL),
          count(*) FILTER (object_type = 'Building' AND b3_volume_lod22 IS NULL)
        FROM read_parquet(?)
        """,
        params=[str(fixture_path)],
    ).fetchone()
    assert row == (0, 0)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_fixture.py -v`
Expected: FAIL (tile_slice.parquet does not exist).

- [ ] **Step 3: Write the fixture generator and run it**

```python
# usecase/energy/tests/fixtures/make_fixture.py
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
```

```markdown
# usecase/energy/tests/fixtures/README.md
`tile_slice.parquet` is a deterministic 150-building slice (plus their
BuildingPart children) of 3DBAG tile 10-756-44 as CityParquet, regenerated with
`make_fixture.py`. Committed so the test suite runs without the 18 GB tile set.
3DBAG data: CC-BY 4.0, © 3DBAG / TU Delft.
```

Run: `cd usecase/energy && uv run python tests/fixtures/make_fixture.py`
Then check the size is committable: `ls -lh tests/fixtures/tile_slice.parquet`
Expected: a few MB at most. If it exceeds ~10 MB, lower LIMIT to 100 and adjust
`test_fixture_has_buildings_and_parts` to match.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_fixture.py -v`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/tests
git commit -m "test(usecase): commit a deterministic 3DBAG fixture slice"
```

---

### Task 3: `db.py` — extension discovery and session

**Files:**
- Create: `usecase/energy/src/energy/errors.py`
- Create: `usecase/energy/src/energy/db.py`
- Modify: `usecase/energy/tests/conftest.py` (add `requires_extensions` helper)
- Test: `usecase/energy/tests/test_db.py`

**Interfaces:**
- Produces:
  - `energy.errors.EnergyError(Exception)`, `ExtensionsNotFound(EnergyError)`,
    `MissingLoD(EnergyError)` (MissingLoD used from Task 7).
  - `energy.db.REPO_ROOT: Path` — the monorepo root.
  - `energy.db.find_extensions(ext_dir: Path | None = None) -> dict[str, Path]`
    with keys `"cityjson"`, `"three_d"`; raises `ExtensionsNotFound` naming every
    path it tried.
  - `energy.db.connect(ext_dir: Path | None = None, need_httpfs: bool = False)
    -> duckdb.DuckDBPyConnection` — unsigned-extensions session with both loaded.
  - `energy.db.extensions_available() -> bool`.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_db.py
from pathlib import Path

import pytest

from energy.db import find_extensions
from energy.errors import ExtensionsNotFound


def test_find_extensions_in_explicit_dir(tmp_path):
    (tmp_path / "cityjson.duckdb_extension").touch()
    (tmp_path / "three_d.duckdb_extension").touch()
    found = find_extensions(ext_dir=tmp_path)
    assert found["cityjson"] == tmp_path / "cityjson.duckdb_extension"
    assert found["three_d"] == tmp_path / "three_d.duckdb_extension"


def test_missing_extensions_error_names_paths(tmp_path):
    with pytest.raises(ExtensionsNotFound) as exc:
        find_extensions(ext_dir=tmp_path)
    msg = str(exc.value)
    assert str(tmp_path / "cityjson.duckdb_extension") in msg
    assert str(tmp_path / "three_d.duckdb_extension") in msg
```

Append to `usecase/energy/tests/conftest.py`:

```python
import pytest as _pytest

from energy.db import extensions_available

requires_extensions = _pytest.mark.skipif(
    not extensions_available(),
    reason="duckdb-cityjson / duckdb-3d not built in this checkout",
)
```

And a first integration test at the end of `test_db.py`:

```python
from .conftest import requires_extensions


@requires_extensions
def test_connect_loads_both_extensions():
    from energy.db import connect

    con = connect()
    fns = {r[0] for r in con.sql(
        "SELECT function_name FROM duckdb_functions() "
        "WHERE function_name IN ('ST_3DVolume', 'cityjson_metadata')"
    ).fetchall()}
    assert fns == {"ST_3DVolume", "cityjson_metadata"}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_db.py -v`
Expected: FAIL with ImportError (no `energy.db`).

- [ ] **Step 3: Write minimal implementation**

```python
# usecase/energy/src/energy/errors.py
class EnergyError(Exception):
    """Base for user-facing failures; the CLI prints these without a traceback."""


class ExtensionsNotFound(EnergyError):
    pass


class MissingLoD(EnergyError):
    pass
```

```python
# usecase/energy/src/energy/db.py
"""DuckDB session setup: locate and load the ecosystem extensions."""
from __future__ import annotations

from pathlib import Path

import duckdb

from .errors import ExtensionsNotFound

# …/usecase/energy/src/energy/db.py → parents[4] = monorepo root
REPO_ROOT = Path(__file__).resolve().parents[4]

_BUILD_PATHS = {
    "cityjson": REPO_ROOT
    / "lib/duckdb-cityjson/build/release/extension/cityjson/cityjson.duckdb_extension",
    "three_d": REPO_ROOT
    / "lib/duckdb-3d/build/release/extension/three_d/three_d.duckdb_extension",
}


def find_extensions(ext_dir: Path | None = None) -> dict[str, Path]:
    if ext_dir is not None:
        candidates = {name: Path(ext_dir) / f"{name}.duckdb_extension"
                      for name in _BUILD_PATHS}
    else:
        candidates = dict(_BUILD_PATHS)
    missing = [p for p in candidates.values() if not p.is_file()]
    if missing:
        tried = "\n  ".join(str(p) for p in candidates.values())
        raise ExtensionsNotFound(
            "could not find the duckdb extensions; tried:\n  " + tried
            + "\nbuild them (see lib/duckdb-3d/README.md) or pass --ext-dir"
        )
    return candidates


def extensions_available(ext_dir: Path | None = None) -> bool:
    try:
        find_extensions(ext_dir)
        return True
    except ExtensionsNotFound:
        return False


def connect(ext_dir: Path | None = None,
            need_httpfs: bool = False) -> duckdb.DuckDBPyConnection:
    paths = find_extensions(ext_dir)
    con = duckdb.connect(config={"allow_unsigned_extensions": True})
    for path in paths.values():
        con.load_extension(str(path))
    if need_httpfs:
        con.install_extension("httpfs")
        con.load_extension("httpfs")
    return con
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_db.py -v`
Expected: 3 PASS (the integration test runs — this checkout has built extensions).

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/errors.py usecase/energy/src/energy/db.py \
        usecase/energy/tests/test_db.py usecase/energy/tests/conftest.py
git commit -m "feat(usecase): load the ecosystem extensions from the build tree"
```

---

### Task 4: `faces.py` — WKB PolyhedralSurface parser

**Files:**
- Create: `usecase/energy/src/energy/faces.py`
- Test: `usecase/energy/tests/test_faces_wkb.py`

**Interfaces:**
- Produces:
  - `energy.faces.parse_wkb_polyhedral(wkb: bytes) -> list[list[np.ndarray]]` —
    outer list: faces in storage order (this order is what `face_semantics` indexes);
    inner list: rings (exterior first); each ring an `(N, 3)` float64 array with the
    WKB closing point removed.
  - Test helper `make_polyhedral_wkb(polygons)` (defined in the test file; Tasks 5–6
    import it from there via `from .test_faces_wkb import make_polyhedral_wkb`).

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_faces_wkb.py
import struct

import numpy as np
import pytest

from energy.faces import parse_wkb_polyhedral


def make_polyhedral_wkb(polygons, type_code=1015, polygon_code=1003):
    """polygons: list of faces; each face: list of rings; ring: list of (x, y, z)."""
    out = bytearray()
    out += struct.pack("<BII", 1, type_code, len(polygons))
    for rings in polygons:
        out += struct.pack("<BII", 1, polygon_code, len(rings))
        for ring in rings:
            pts = list(ring) + [ring[0]]  # WKB rings close explicitly
            out += struct.pack("<I", len(pts))
            for x, y, z in pts:
                out += struct.pack("<ddd", x, y, z)
    return bytes(out)


UNIT_CUBE = [
    [[(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)]],  # bottom, normal -z
    [[(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)]],  # top, normal +z
    [[(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)]],  # front y=0, normal -y
    [[(1, 1, 0), (0, 1, 0), (0, 1, 1), (1, 1, 1)]],  # back y=1, normal +y
    [[(0, 0, 0), (0, 0, 1), (0, 1, 1), (0, 1, 0)]],  # left x=0, normal -x
    [[(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)]],  # right x=1, normal +x
]


def test_parse_unit_cube():
    faces = parse_wkb_polyhedral(make_polyhedral_wkb(UNIT_CUBE))
    assert len(faces) == 6
    for face in faces:
        assert len(face) == 1               # exterior ring only
        assert face[0].shape == (4, 3)      # closing point removed
    np.testing.assert_allclose(faces[0][0][0], [0.0, 0.0, 0.0])
    np.testing.assert_allclose(faces[5][0][2], [1.0, 1.0, 1.0])


def test_parse_accepts_multipolygon_z():
    faces = parse_wkb_polyhedral(
        make_polyhedral_wkb(UNIT_CUBE, type_code=1006, polygon_code=1003)
    )
    assert len(faces) == 6


def test_parse_rejects_unknown_type():
    bad = struct.pack("<BII", 1, 1001, 0)  # PointZ
    with pytest.raises(ValueError, match="unsupported WKB type"):
        parse_wkb_polyhedral(bad)


def test_parse_keeps_interior_rings():
    face_with_hole = [[
        (0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)],
        [(1, 1, 0), (1, 2, 0), (2, 2, 0), (2, 1, 0)],
    ]
    faces = parse_wkb_polyhedral(make_polyhedral_wkb([face_with_hole]))
    assert len(faces) == 1
    assert len(faces[0]) == 2
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_faces_wkb.py -v`
Expected: FAIL with ImportError (no `energy.faces`).

- [ ] **Step 3: Write minimal implementation**

```python
# usecase/energy/src/energy/faces.py
"""ST_3DFaces prototype: per-face metrics from CityParquet WKB + semantics.

This module deliberately reimplements, in Python, what the planned
`ST_3DFaces` table function should do inside duckdb-3d. Its output schema is
the working contract for that primitive; when it lands, this module reduces
to a SQL query.
"""
from __future__ import annotations

import struct

import numpy as np

_Z_FLAG = 0x80000000
_SURFACE_TYPES = {15, 6}   # PolyhedralSurface, MultiPolygon
_POLYGON_TYPES = {3}


def _base_type(raw: int) -> int:
    if raw & _Z_FLAG:                      # EWKB Z flag
        return raw & 0x0FFFFFFF
    return raw % 1000 if raw >= 1000 else raw   # ISO: 1015 → 15


class _Reader:
    def __init__(self, buf: bytes) -> None:
        self.buf = buf
        self.pos = 0

    def _take(self, fmt: str):
        vals = struct.unpack_from(fmt, self.buf, self.pos)
        self.pos += struct.calcsize(fmt)
        return vals

    def header(self) -> int:
        (bo,) = self._take("<B")
        (raw,) = self._take("<I" if bo == 1 else ">I")
        self.order = "<" if bo == 1 else ">"
        return _base_type(raw)

    def u32(self) -> int:
        return self._take(f"{self.order}I")[0]

    def ring(self) -> np.ndarray:
        n = self.u32()
        flat = self._take(f"{self.order}{3 * n}d")
        pts = np.asarray(flat, dtype=np.float64).reshape(n, 3)
        if n > 1 and np.array_equal(pts[0], pts[-1]):
            pts = pts[:-1]
        return pts


def parse_wkb_polyhedral(wkb: bytes) -> list[list[np.ndarray]]:
    reader = _Reader(wkb)
    outer = reader.header()
    if outer not in _SURFACE_TYPES:
        raise ValueError(f"unsupported WKB type {outer}; expected PolyhedralSurface/MultiPolygon")
    faces: list[list[np.ndarray]] = []
    for _ in range(reader.u32()):
        inner = reader.header()
        if inner not in _POLYGON_TYPES:
            raise ValueError(f"unsupported WKB type {inner}; expected Polygon")
        faces.append([reader.ring() for _ in range(reader.u32())])
    return faces
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_faces_wkb.py -v`
Expected: 4 PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/faces.py usecase/energy/tests/test_faces_wkb.py
git commit -m "feat(usecase): parse PolyhedralSurface WKB into per-face rings"
```

---

### Task 5: `faces.py` — Newell metrics per face

**Files:**
- Modify: `usecase/energy/src/energy/faces.py` (append)
- Test: `usecase/energy/tests/test_faces_metrics.py`

**Interfaces:**
- Produces:
  - `energy.faces.FaceMetrics` dataclass: `normal: np.ndarray` (unit, 3), `area: float`,
    `tilt_deg: float` (angle normal↔+Z: 0 flat-up, 90 vertical, 180 flat-down),
    `azimuth_deg: float | None` (compass from north, clockwise; None when the
    normal is within 1e-9 of vertical), `centroid: np.ndarray` (3).
  - `energy.faces.face_metrics(rings: list[np.ndarray]) -> FaceMetrics` —
    exterior-ring Newell normal; area = exterior − Σ interior ring areas (≥ 0).

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_faces_metrics.py
import math

import numpy as np

from energy.faces import face_metrics, parse_wkb_polyhedral
from .test_faces_wkb import UNIT_CUBE, make_polyhedral_wkb


def _cube_faces():
    return parse_wkb_polyhedral(make_polyhedral_wkb(UNIT_CUBE))


def test_cube_top_face():
    m = face_metrics(_cube_faces()[1])
    assert math.isclose(m.area, 1.0, rel_tol=1e-12)
    assert math.isclose(m.tilt_deg, 0.0, abs_tol=1e-9)
    assert m.azimuth_deg is None
    np.testing.assert_allclose(m.normal, [0, 0, 1], atol=1e-12)
    np.testing.assert_allclose(m.centroid, [0.5, 0.5, 1.0], atol=1e-12)


def test_cube_bottom_face_points_down():
    m = face_metrics(_cube_faces()[0])
    assert math.isclose(m.tilt_deg, 180.0, abs_tol=1e-9)
    np.testing.assert_allclose(m.normal, [0, 0, -1], atol=1e-12)


def test_cube_wall_azimuths():
    faces = _cube_faces()
    # back face (+y normal) → azimuth 0 (north); right face (+x) → 90 (east)
    assert math.isclose(face_metrics(faces[3]).azimuth_deg, 0.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[5]).azimuth_deg, 90.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[2]).azimuth_deg, 180.0, abs_tol=1e-9)
    assert math.isclose(face_metrics(faces[4]).azimuth_deg, 270.0, abs_tol=1e-9)
    for i in (2, 3, 4, 5):
        assert math.isclose(face_metrics(faces[i]).tilt_deg, 90.0, abs_tol=1e-9)


def test_pitched_roof_45_degrees():
    ring = np.array([(0, 0, 0), (1, 0, 0), (1, 1, 1), (0, 1, 1)], dtype=float)
    m = face_metrics([ring])
    assert math.isclose(m.tilt_deg, 45.0, abs_tol=1e-9)
    assert math.isclose(m.area, math.sqrt(2.0), rel_tol=1e-12)


def test_hole_subtracts_area():
    exterior = np.array([(0, 0, 0), (4, 0, 0), (4, 4, 0), (0, 4, 0)], dtype=float)
    hole = np.array([(1, 1, 0), (1, 2, 0), (2, 2, 0), (2, 1, 0)], dtype=float)
    m = face_metrics([exterior, hole])
    assert math.isclose(m.area, 16.0 - 1.0, rel_tol=1e-12)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_faces_metrics.py -v`
Expected: FAIL with ImportError (`face_metrics` not defined).

- [ ] **Step 3: Write minimal implementation** (append to `faces.py`)

```python
from dataclasses import dataclass


@dataclass
class FaceMetrics:
    normal: np.ndarray
    area: float
    tilt_deg: float
    azimuth_deg: float | None
    centroid: np.ndarray


def _newell(ring: np.ndarray) -> np.ndarray:
    nxt = np.roll(ring, -1, axis=0)
    return np.array([
        np.sum((ring[:, 1] - nxt[:, 1]) * (ring[:, 2] + nxt[:, 2])),
        np.sum((ring[:, 2] - nxt[:, 2]) * (ring[:, 0] + nxt[:, 0])),
        np.sum((ring[:, 0] - nxt[:, 0]) * (ring[:, 1] + nxt[:, 1])),
    ])


def face_metrics(rings: list[np.ndarray]) -> FaceMetrics:
    n = _newell(rings[0])
    exterior_area = float(np.linalg.norm(n)) / 2.0
    holes = sum(float(np.linalg.norm(_newell(r))) / 2.0 for r in rings[1:])
    area = max(exterior_area - holes, 0.0)

    norm = np.linalg.norm(n)
    unit = n / norm if norm > 0 else np.array([0.0, 0.0, 0.0])
    tilt = float(np.degrees(np.arccos(np.clip(unit[2], -1.0, 1.0))))
    horiz = math.hypot(unit[0], unit[1])
    azimuth = None if horiz < 1e-9 else float(np.degrees(np.arctan2(unit[0], unit[1])) % 360.0)
    return FaceMetrics(
        normal=unit,
        area=area,
        tilt_deg=tilt,
        azimuth_deg=azimuth,
        centroid=rings[0].mean(axis=0),
    )
```

Also add `import math` at the top of `faces.py`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_faces_metrics.py -v`
Expected: 5 PASS.
If `test_cube_wall_azimuths` fails with swapped directions, the cube winding in
`UNIT_CUBE` and the Newell sign convention disagree — fix the test's expected
*labels* only after confirming with a hand computation of one face; do not silently
flip signs in `_newell`.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/faces.py usecase/energy/tests/test_faces_metrics.py
git commit -m "feat(usecase): per-face Newell normal, area, tilt and azimuth"
```

---

### Task 6: `faces.py` — semantics, per-part records, class aggregation

**Files:**
- Modify: `usecase/energy/src/energy/faces.py` (append)
- Test: `usecase/energy/tests/test_faces_semantics.py`

**Interfaces:**
- Produces:
  - `energy.faces.resolve_semantics(face_semantics: list[int] | None,
    surfaces_json: str | None, n_faces: int) -> list[str]` — per-face labels;
    `"Unknown"` on None / out-of-range / length mismatch padding.
  - `energy.faces.FaceRecord` dataclass: `building_id: str`, `part_id: str`,
    `face_idx: int`, `semantic: str`, `nx, ny, nz: float`, `tilt_deg: float`,
    `azimuth_deg: float | None`, `area_m2: float`, `cx, cy, cz: float`.
  - `energy.faces.faces_for_part(building_id: str, part_id: str, wkb: bytes,
    face_semantics, surfaces_json) -> list[FaceRecord]`.
  - `energy.faces.class_areas(records: list[FaceRecord], flat_tilt_deg: float = 5.0)
    -> dict[str, float]` with keys `a_roof_flat_m2`, `a_roof_pitched_m2`,
    `a_wall_m2`, `a_ground_m2`, `a_other_m2`.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_faces_semantics.py
import json
import math

from energy.faces import class_areas, faces_for_part, resolve_semantics
from .test_faces_wkb import UNIT_CUBE, make_polyhedral_wkb

CUBE_SURFACES = json.dumps([
    {"type": "GroundSurface"},
    {"type": "WallSurface", "on_footprint_edge": True},
    {"type": "RoofSurface", "b3_azimut": 120.0},
])
CUBE_SEMANTICS = [0, 2, 1, 1, 1, 1]  # bottom, top, 4 walls


def test_resolve_semantics_maps_indices():
    labels = resolve_semantics(CUBE_SEMANTICS, CUBE_SURFACES, 6)
    assert labels == ["GroundSurface", "RoofSurface", "WallSurface",
                      "WallSurface", "WallSurface", "WallSurface"]


def test_resolve_semantics_handles_missing():
    assert resolve_semantics(None, CUBE_SURFACES, 2) == ["Unknown", "Unknown"]
    assert resolve_semantics([0, 99], CUBE_SURFACES, 2) == ["GroundSurface", "Unknown"]
    assert resolve_semantics([0], CUBE_SURFACES, 3) == ["GroundSurface", "Unknown", "Unknown"]


def test_faces_for_part_full_cube():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE),
                             CUBE_SEMANTICS, CUBE_SURFACES)
    assert len(records) == 6
    assert records[0].semantic == "GroundSurface"
    assert records[1].semantic == "RoofSurface"
    assert all(r.building_id == "B1" and r.part_id == "P1" for r in records)
    assert [r.face_idx for r in records] == list(range(6))
    assert math.isclose(sum(r.area_m2 for r in records), 6.0, rel_tol=1e-12)


def test_class_areas_cube():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE),
                             CUBE_SEMANTICS, CUBE_SURFACES)
    areas = class_areas(records, flat_tilt_deg=5.0)
    assert math.isclose(areas["a_roof_flat_m2"], 1.0, rel_tol=1e-12)  # flat top
    assert math.isclose(areas["a_roof_pitched_m2"], 0.0, abs_tol=1e-12)
    assert math.isclose(areas["a_wall_m2"], 4.0, rel_tol=1e-12)
    assert math.isclose(areas["a_ground_m2"], 1.0, rel_tol=1e-12)
    assert math.isclose(areas["a_other_m2"], 0.0, abs_tol=1e-12)


def test_class_areas_unknown_goes_to_other():
    records = faces_for_part("B1", "P1", make_polyhedral_wkb(UNIT_CUBE), None, None)
    areas = class_areas(records)
    assert math.isclose(areas["a_other_m2"], 6.0, rel_tol=1e-12)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_faces_semantics.py -v`
Expected: FAIL with ImportError.

- [ ] **Step 3: Write minimal implementation** (append to `faces.py`)

```python
import json


def resolve_semantics(face_semantics, surfaces_json, n_faces: int) -> list[str]:
    labels = ["Unknown"] * n_faces
    if face_semantics is None or surfaces_json is None:
        return labels
    try:
        surfaces = json.loads(surfaces_json)
    except (TypeError, ValueError):
        return labels
    for i, idx in enumerate(face_semantics[:n_faces]):
        if idx is not None and 0 <= idx < len(surfaces):
            labels[i] = surfaces[idx].get("type", "Unknown")
    return labels


@dataclass
class FaceRecord:
    building_id: str
    part_id: str
    face_idx: int
    semantic: str
    nx: float
    ny: float
    nz: float
    tilt_deg: float
    azimuth_deg: float | None
    area_m2: float
    cx: float
    cy: float
    cz: float


def faces_for_part(building_id, part_id, wkb, face_semantics, surfaces_json):
    faces = parse_wkb_polyhedral(wkb)
    labels = resolve_semantics(face_semantics, surfaces_json, len(faces))
    records = []
    for idx, (rings, label) in enumerate(zip(faces, labels)):
        m = face_metrics(rings)
        records.append(FaceRecord(
            building_id=building_id, part_id=part_id, face_idx=idx, semantic=label,
            nx=float(m.normal[0]), ny=float(m.normal[1]), nz=float(m.normal[2]),
            tilt_deg=m.tilt_deg, azimuth_deg=m.azimuth_deg, area_m2=m.area,
            cx=float(m.centroid[0]), cy=float(m.centroid[1]), cz=float(m.centroid[2]),
        ))
    return records


def class_areas(records, flat_tilt_deg: float = 5.0) -> dict[str, float]:
    out = {"a_roof_flat_m2": 0.0, "a_roof_pitched_m2": 0.0,
           "a_wall_m2": 0.0, "a_ground_m2": 0.0, "a_other_m2": 0.0}
    for r in records:
        if r.semantic == "RoofSurface":
            key = "a_roof_flat_m2" if r.tilt_deg <= flat_tilt_deg else "a_roof_pitched_m2"
        elif r.semantic == "WallSurface":
            key = "a_wall_m2"
        elif r.semantic == "GroundSurface":
            key = "a_ground_m2"
        else:
            key = "a_other_m2"
        out[key] += r.area_m2
    return out
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_faces_semantics.py -v`
Expected: 5 PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/faces.py usecase/energy/tests/test_faces_semantics.py
git commit -m "feat(usecase): resolve face semantics and aggregate class areas"
```

---

### Task 7: `features.py` — SQL whole-solid metrics

**Files:**
- Create: `usecase/energy/src/energy/features.py`
- Test: `usecase/energy/tests/test_features.py`

**Interfaces:**
- Consumes: `energy.db.connect`, `energy.errors.MissingLoD`.
- Produces:
  - `energy.features.lod_to_suffix(lod: str) -> str` — `"2.2" → "lod2_2"`; raises
    `MissingLoD` on malformed input (not `r"^\d\.\d$"`).
  - `energy.features.available_lods(con, input_glob: str) -> list[str]` — e.g.
    `["1.2", "1.3", "2.2"]`, from the `geometry_lod*` column names.
  - `energy.features.build_features(con, input_glob: str, lod: str) -> pyarrow.Table`
    — one row per Building with columns: `building_id: str`, `year: int64`,
    `n_parts: int64`, `volume_m3, envelope_m2, sv_ratio, footprint_m2, height_m:
    float64`, `is_closed: bool`, and reference columns `b3_volume_lod22,
    b3_opp_dak_plat, b3_opp_dak_schuin, b3_opp_grond, b3_opp_buitenmuur,
    b3_opp_scheidingsmuur: float64`. Raises `MissingLoD` naming available LoDs
    when the geometry column is absent.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_features.py
import duckdb
import pytest

from energy.errors import MissingLoD
from energy.features import available_lods, build_features, lod_to_suffix
from .conftest import requires_extensions


def test_lod_to_suffix():
    assert lod_to_suffix("2.2") == "lod2_2"
    assert lod_to_suffix("1.3") == "lod1_3"
    with pytest.raises(MissingLoD):
        lod_to_suffix("22")


def test_available_lods(fixture_path):
    con = duckdb.connect()
    assert available_lods(con, str(fixture_path)) == ["0.0", "1.2", "1.3", "2.2"]


def test_missing_lod_lists_available(fixture_path):
    con = duckdb.connect()
    with pytest.raises(MissingLoD, match="2.2"):
        build_features(con, str(fixture_path), "9.9")


@requires_extensions
def test_build_features_volumes_match_3dbag(fixture_path):
    from energy.db import connect

    table = build_features(connect(), str(fixture_path), "2.2")
    assert table.num_rows == 150
    rows = table.to_pylist()
    rel_errs = sorted(
        abs(r["volume_m3"] - r["b3_volume_lod22"]) / r["b3_volume_lod22"]
        for r in rows if r["b3_volume_lod22"]
    )
    median = rel_errs[len(rel_errs) // 2]
    assert median < 0.01, f"median volume error {median:.4f}"
    assert all(r["height_m"] > 0 for r in rows)
    assert all(r["sv_ratio"] and r["sv_ratio"] > 0 for r in rows)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_features.py -v`
Expected: FAIL with ImportError.

- [ ] **Step 3: Write minimal implementation**

```python
# usecase/energy/src/energy/features.py
"""Whole-solid metrics in SQL via duckdb-3d, one row per Building."""
from __future__ import annotations

import re

import duckdb
import pyarrow as pa

from .errors import MissingLoD

_REFS = ("b3_volume_lod22", "b3_opp_dak_plat", "b3_opp_dak_schuin",
         "b3_opp_grond", "b3_opp_buitenmuur", "b3_opp_scheidingsmuur")


def lod_to_suffix(lod: str) -> str:
    if not re.fullmatch(r"\d\.\d", lod):
        raise MissingLoD(f"malformed LoD {lod!r}; expected e.g. '2.2'")
    return "lod" + lod.replace(".", "_")


def available_lods(con: duckdb.DuckDBPyConnection, input_glob: str) -> list[str]:
    cols = [d[0] for d in con.sql(
        "SELECT * FROM read_parquet(?) LIMIT 0", params=[input_glob]
    ).description]
    lods = sorted(m.group(1) + "." + m.group(2)
                  for c in cols
                  if (m := re.fullmatch(r"geometry_lod(\d)_(\d)", c)))
    return lods


def build_features(con: duckdb.DuckDBPyConnection,
                   input_glob: str, lod: str) -> pa.Table:
    suffix = lod_to_suffix(lod)
    lods = available_lods(con, input_glob)
    if lod not in lods:
        raise MissingLoD(
            f"LoD {lod} not in this package; available: {', '.join(lods)}"
        )
    geom = f"geometry_{suffix}"
    refs = ", ".join(f"any_value(b.{r}) AS {r}" for r in _REFS)
    query = f"""
        WITH parts AS (
          SELECT p.parents[1] AS building_id,
                 ST_3DTryFromWKB(p.{geom}) AS solid
          FROM read_parquet($input) p
          WHERE p.object_type = 'BuildingPart' AND p.{geom} IS NOT NULL
        ),
        metrics AS (
          SELECT building_id,
                 ST_3DVolume(solid) AS v,
                 ST_3DSurfaceArea(solid) AS a,
                 ST_3DFootprintArea(solid) AS fp,
                 ST_3DZMin(solid) AS zmin,
                 ST_3DZMax(solid) AS zmax,
                 ST_3DIsClosed(solid) AS closed
          FROM parts
          WHERE solid IS NOT NULL
        )
        SELECT m.building_id,
               any_value(b.oorspronkelijkbouwjaar) AS year,
               count(*) AS n_parts,
               sum(m.v) AS volume_m3,
               sum(m.a) AS envelope_m2,
               sum(m.a) / nullif(sum(m.v), 0) AS sv_ratio,
               sum(m.fp) AS footprint_m2,
               max(m.zmax) - min(m.zmin) AS height_m,
               bool_and(m.closed) AS is_closed,
               {refs}
        FROM metrics m
        JOIN read_parquet($input) b
          ON b.id = m.building_id AND b.object_type = 'Building'
        GROUP BY m.building_id
        ORDER BY m.building_id
    """
    return con.sql(query, params={"input": input_glob}).arrow()
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_features.py -v`
Expected: 4 PASS. If the fixture happens to include `geometry_lod0_0` as NULL-typed
or the LoD list differs, adjust `test_available_lods` to the actual list the fixture
shows — the assertion documents reality, the helper must simply report the columns.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/features.py usecase/energy/tests/test_features.py
git commit -m "feat(usecase): whole-solid building metrics via duckdb-3d SQL"
```

---

### Task 8: `run.py` — assemble and write `features.parquet`

**Files:**
- Create: `usecase/energy/src/energy/run.py`
- Modify: `usecase/energy/src/energy/faces.py` (append `compute_face_tables`)
- Test: `usecase/energy/tests/test_run_features.py`

**Interfaces:**
- Consumes: `db.connect`, `features.build_features`, `faces.faces_for_part`,
  `faces.class_areas`, `faces.FaceRecord`.
- Produces:
  - `energy.faces.compute_face_tables(input_glob: str, lod_suffix: str,
    flat_tilt_deg: float) -> tuple[pa.Table, pa.Table]` — `(faces, classes)`.
    `faces` columns = FaceRecord fields; `classes` columns = `building_id` +
    the five `a_*_m2` keys (summed over all parts of the building).
    Uses a plain `duckdb.connect()` (no extensions needed).
  - `energy.run.RunSummary` dataclass: `n_buildings: int`, `n_parts: int`,
    `n_null_geometry: int`, `n_open_solids: int`, `outputs: list[str]`.
  - `energy.run.run_features(input_glob: str, lod: str, output: str,
    faces_out: str | None, validate_out: str | None, flat_tilt_deg: float,
    ext_dir=None) -> RunSummary` — writes `output` (zstd Parquet) with the spec's
    feature columns (b3 refs excluded unless `validate_out` is set; validation
    wiring itself lands in Task 9), optionally `faces_out`.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_run_features.py
import duckdb
import pytest

from energy.faces import compute_face_tables
from .conftest import requires_extensions

EXPECTED_COLS = {
    "building_id", "year", "n_parts", "volume_m3", "envelope_m2", "sv_ratio",
    "footprint_m2", "height_m", "is_closed",
    "a_roof_flat_m2", "a_roof_pitched_m2", "a_wall_m2", "a_ground_m2", "a_other_m2",
}


def test_compute_face_tables_shapes(fixture_path):
    faces, classes = compute_face_tables(str(fixture_path), "lod2_2", 5.0)
    assert classes.num_rows == 150
    assert faces.num_rows > classes.num_rows          # many faces per building
    assert set(classes.column_names) == {
        "building_id", "a_roof_flat_m2", "a_roof_pitched_m2",
        "a_wall_m2", "a_ground_m2", "a_other_m2",
    }


def test_class_areas_match_3dbag_references(fixture_path):
    _, classes = compute_face_tables(str(fixture_path), "lod2_2", 5.0)
    con = duckdb.connect()
    con.register("classes", classes)
    median_err = con.sql(
        """
        SELECT median(abs((c.a_ground_m2 - b.b3_opp_grond) / nullif(b.b3_opp_grond, 0)))
        FROM classes c
        JOIN read_parquet(?) b ON b.id = c.building_id
        WHERE b.object_type = 'Building'
        """,
        params=[str(fixture_path)],
    ).fetchone()[0]
    assert median_err < 0.05, f"median ground-area error {median_err:.4f}"


@requires_extensions
def test_run_features_end_to_end(fixture_path, tmp_path):
    from energy.run import run_features

    out = tmp_path / "features.parquet"
    summary = run_features(str(fixture_path), "2.2", str(out),
                           faces_out=None, validate_out=None, flat_tilt_deg=5.0)
    assert summary.n_buildings == 150
    assert summary.n_parts >= 150
    table = duckdb.sql(f"FROM '{out}'").arrow()
    assert set(table.column_names) == EXPECTED_COLS
    assert table.num_rows == 150


@requires_extensions
def test_run_features_writes_faces_table(fixture_path, tmp_path):
    from energy.run import run_features

    out = tmp_path / "features.parquet"
    faces_out = tmp_path / "faces.parquet"
    run_features(str(fixture_path), "2.2", str(out),
                 faces_out=str(faces_out), validate_out=None, flat_tilt_deg=5.0)
    faces = duckdb.sql(f"FROM '{faces_out}'").arrow()
    assert {"building_id", "part_id", "face_idx", "semantic",
            "tilt_deg", "azimuth_deg", "area_m2"} <= set(faces.column_names)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_run_features.py -v`
Expected: FAIL with ImportError (`compute_face_tables`, `energy.run`).

- [ ] **Step 3: Write minimal implementation**

Append to `faces.py`:

```python
import duckdb
import pyarrow as pa
from dataclasses import asdict, fields


def compute_face_tables(input_glob: str, lod_suffix: str,
                        flat_tilt_deg: float) -> tuple[pa.Table, pa.Table]:
    con = duckdb.connect()
    rows = con.sql(
        f"""
        SELECT p.parents[1] AS building_id, p.id AS part_id,
               p.geometry_{lod_suffix} AS wkb,
               p.geometry_properties_{lod_suffix}.face_semantics AS fs,
               CAST(p.geometry_properties_{lod_suffix}.surfaces AS VARCHAR) AS sj
        FROM read_parquet($input) p
        WHERE p.object_type = 'BuildingPart' AND p.geometry_{lod_suffix} IS NOT NULL
        ORDER BY building_id, part_id
        """,
        params={"input": input_glob},
    ).fetchall()

    all_records = []
    per_building: dict[str, dict[str, float]] = {}
    for building_id, part_id, wkb, fs, sj in rows:
        records = faces_for_part(building_id, part_id, bytes(wkb), fs, sj)
        all_records.extend(records)
        areas = class_areas(records, flat_tilt_deg)
        acc = per_building.setdefault(building_id, dict.fromkeys(areas, 0.0))
        for key, val in areas.items():
            acc[key] += val

    face_cols = [f.name for f in fields(FaceRecord)]
    faces_table = pa.table(
        {c: [getattr(r, c) for r in all_records] for c in face_cols}
    )
    classes_table = pa.table({
        "building_id": list(per_building),
        **{k: [v[k] for v in per_building.values()]
           for k in ("a_roof_flat_m2", "a_roof_pitched_m2",
                      "a_wall_m2", "a_ground_m2", "a_other_m2")},
    })
    return faces_table, classes_table
```

```python
# usecase/energy/src/energy/run.py
"""Orchestrate the `features` subcommand: SQL metrics + face split → Parquet."""
from __future__ import annotations

from dataclasses import dataclass, field

from . import db
from .faces import compute_face_tables
from .features import build_features, lod_to_suffix

_REF_COLS = ("b3_volume_lod22", "b3_opp_dak_plat", "b3_opp_dak_schuin",
             "b3_opp_grond", "b3_opp_buitenmuur", "b3_opp_scheidingsmuur")


@dataclass
class RunSummary:
    n_buildings: int
    n_parts: int
    n_null_geometry: int
    n_open_solids: int
    outputs: list[str] = field(default_factory=list)


def run_features(input_glob: str, lod: str, output: str,
                 faces_out: str | None, validate_out: str | None,
                 flat_tilt_deg: float, ext_dir=None) -> RunSummary:
    con = db.connect(ext_dir, need_httpfs=input_glob.startswith("s3://"))
    features = build_features(con, input_glob, lod)
    faces, classes = compute_face_tables(input_glob, lod_to_suffix(lod), flat_tilt_deg)
    con.register("features_t", features)
    con.register("classes_t", classes)

    drop_refs = "" if validate_out else \
        "EXCLUDE (" + ", ".join(_REF_COLS) + ")"
    con.execute(
        f"""
        COPY (
          SELECT f.* {drop_refs},
                 coalesce(c.a_roof_flat_m2, 0)    AS a_roof_flat_m2,
                 coalesce(c.a_roof_pitched_m2, 0) AS a_roof_pitched_m2,
                 coalesce(c.a_wall_m2, 0)         AS a_wall_m2,
                 coalesce(c.a_ground_m2, 0)       AS a_ground_m2,
                 coalesce(c.a_other_m2, 0)        AS a_other_m2
          FROM features_t f LEFT JOIN classes_t c USING (building_id)
          ORDER BY f.building_id
        ) TO '{output}' (FORMAT PARQUET, COMPRESSION ZSTD)
        """
    )
    outputs = [output]

    if faces_out:
        con.execute(
            f"COPY (SELECT * FROM faces_t ORDER BY building_id, part_id, face_idx)"
            f" TO '{faces_out}' (FORMAT PARQUET, COMPRESSION ZSTD)",
        ) if con.register("faces_t", faces) is None else None
        outputs.append(faces_out)

    geom = f"geometry_{lod_to_suffix(lod)}"
    n_null, n_parts = con.sql(
        f"""
        SELECT count(*) FILTER ({geom} IS NULL),
               count(*) FILTER ({geom} IS NOT NULL)
        FROM read_parquet($input) WHERE object_type = 'BuildingPart'
        """,
        params={"input": input_glob},
    ).fetchone()
    n_open = con.sql(
        "SELECT count(*) FROM features_t WHERE NOT is_closed"
    ).fetchone()[0]
    return RunSummary(
        n_buildings=features.num_rows, n_parts=n_parts,
        n_null_geometry=n_null, n_open_solids=n_open, outputs=outputs,
    )
```

Note on the `faces_out` block: `con.register` returns the connection, so write it as
two plain statements — register first, then COPY. (The one-liner above is a reminder
that both must happen; implement it as two statements.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_run_features.py -v`
Expected: 4 PASS. The 5% ground-area tolerance is a starting bound; if the fixture
shows a larger honest discrepancy (e.g. party-wall effects don't touch ground), 
investigate before loosening — ground areas should match closely.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/run.py usecase/energy/src/energy/faces.py \
        usecase/energy/tests/test_run_features.py
git commit -m "feat(usecase): assemble and write the per-building feature table"
```

---

### Task 9: `validate.py` — comparison report

**Files:**
- Create: `usecase/energy/src/energy/validate.py`
- Modify: `usecase/energy/src/energy/run.py` (wire `validate_out`)
- Test: `usecase/energy/tests/test_validate.py`

**Interfaces:**
- Consumes: the features table with reference columns (from `build_features` +
  class areas, as assembled by `run_features` when `validate_out` is set).
- Produces:
  - `energy.validate.validate(table: pa.Table) -> dict` — keys `volume`,
    `roof_flat`, `roof_pitched`, `ground`, `wall`; each
    `{"n": int, "mae": float, "median_rel_err_pct": float, "worst":
    [{"building_id", "computed", "reference", "rel_err_pct"} × ≤5]}`.
    The `wall` reference is `b3_opp_buitenmuur + b3_opp_scheidingsmuur`.
  - `energy.validate.write_report(report: dict, path: str) -> None` (JSON, indent 2).

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_validate.py
import json

import pyarrow as pa

from energy.validate import validate, write_report


def _table():
    return pa.table({
        "building_id": ["a", "b"],
        "volume_m3": [100.0, 210.0],
        "a_roof_flat_m2": [10.0, 0.0],
        "a_roof_pitched_m2": [0.0, 50.0],
        "a_ground_m2": [40.0, 60.0],
        "a_wall_m2": [80.0, 120.0],
        "b3_volume_lod22": [100.0, 200.0],
        "b3_opp_dak_plat": [10.0, 0.0],
        "b3_opp_dak_schuin": [0.0, 50.0],
        "b3_opp_grond": [40.0, 60.0],
        "b3_opp_buitenmuur": [50.0, 70.0],
        "b3_opp_scheidingsmuur": [30.0, 50.0],
    })


def test_validate_metrics():
    report = validate(_table())
    assert set(report) == {"volume", "roof_flat", "roof_pitched", "ground", "wall"}
    vol = report["volume"]
    assert vol["n"] == 2
    assert abs(vol["mae"] - 5.0) < 1e-9                 # (0 + 10) / 2
    assert abs(vol["median_rel_err_pct"] - 2.5) < 1e-9  # median of 0%, 5%
    assert vol["worst"][0]["building_id"] == "b"
    assert report["wall"]["mae"] == 0.0                 # 80 = 50+30, 120 = 70+50


def test_write_report(tmp_path):
    path = tmp_path / "report.json"
    write_report(validate(_table()), str(path))
    loaded = json.loads(path.read_text())
    assert loaded["volume"]["n"] == 2
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_validate.py -v`
Expected: FAIL with ImportError.

- [ ] **Step 3: Write minimal implementation**

```python
# usecase/energy/src/energy/validate.py
"""Self-contained validation against 3DBAG's own b3_* reference columns."""
from __future__ import annotations

import json

import pyarrow as pa

_PAIRS = {
    "volume": ("volume_m3", ("b3_volume_lod22",)),
    "roof_flat": ("a_roof_flat_m2", ("b3_opp_dak_plat",)),
    "roof_pitched": ("a_roof_pitched_m2", ("b3_opp_dak_schuin",)),
    "ground": ("a_ground_m2", ("b3_opp_grond",)),
    "wall": ("a_wall_m2", ("b3_opp_buitenmuur", "b3_opp_scheidingsmuur")),
}


def validate(table: pa.Table) -> dict:
    rows = table.to_pylist()
    report: dict = {}
    for name, (computed_col, ref_cols) in _PAIRS.items():
        entries = []
        for r in rows:
            refs = [r.get(c) for c in ref_cols]
            if any(v is None for v in refs) or r.get(computed_col) is None:
                continue
            reference = sum(refs)
            computed = r[computed_col]
            err = abs(computed - reference)
            rel = err / reference * 100.0 if reference else 0.0
            entries.append((r["building_id"], computed, reference, err, rel))
        entries.sort(key=lambda e: e[4], reverse=True)
        rels = sorted(e[4] for e in entries)
        n = len(entries)
        report[name] = {
            "n": n,
            "mae": sum(e[3] for e in entries) / n if n else None,
            "median_rel_err_pct": rels[n // 2] if n else None,
            "worst": [
                {"building_id": b, "computed": c, "reference": ref, "rel_err_pct": rel}
                for b, c, ref, _, rel in entries[:5]
            ],
        }
    return report


def write_report(report: dict, path: str) -> None:
    with open(path, "w") as fh:
        json.dump(report, fh, indent=2)
```

Wire into `run.py` — inside `run_features`, after the COPY of the features output,
add:

```python
    if validate_out:
        from .validate import validate as _validate, write_report

        full = con.sql(
            """
            SELECT f.*, coalesce(c.a_roof_flat_m2, 0) AS a_roof_flat_m2,
                   coalesce(c.a_roof_pitched_m2, 0) AS a_roof_pitched_m2,
                   coalesce(c.a_wall_m2, 0) AS a_wall_m2,
                   coalesce(c.a_ground_m2, 0) AS a_ground_m2,
                   coalesce(c.a_other_m2, 0) AS a_other_m2
            FROM features_t f LEFT JOIN classes_t c USING (building_id)
            """
        ).arrow()
        write_report(_validate(full), validate_out)
        outputs.append(validate_out)
```

Add an integration assertion at the end of `test_run_features.py`:

```python
@requires_extensions
def test_run_features_validate_report(fixture_path, tmp_path):
    import json

    from energy.run import run_features

    report_path = tmp_path / "report.json"
    run_features(str(fixture_path), "2.2", str(tmp_path / "f.parquet"),
                 faces_out=None, validate_out=str(report_path), flat_tilt_deg=5.0)
    report = json.loads(report_path.read_text())
    assert report["volume"]["median_rel_err_pct"] < 1.0
    assert report["ground"]["median_rel_err_pct"] < 5.0
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd usecase/energy && uv run pytest tests/test_validate.py tests/test_run_features.py -v`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/validate.py usecase/energy/src/energy/run.py \
        usecase/energy/tests/test_validate.py usecase/energy/tests/test_run_features.py
git commit -m "feat(usecase): validate computed features against 3DBAG references"
```

---

### Task 10: `screen.py` + U-value parameters

**Files:**
- Create: `usecase/energy/src/energy/params/u_values.toml`
- Create: `usecase/energy/src/energy/screen.py`
- Test: `usecase/energy/tests/test_screen.py`

**Interfaces:**
- Produces:
  - `energy.screen.Band` dataclass: `name: str`, `max_year: int | None`,
    `u_roof: float`, `u_wall: float`, `u_ground: float`.
  - `energy.screen.load_params(path: str | None = None) -> list[Band]` — sorted by
    `max_year` ascending, the open-ended band (`max_year = None`) last; default file
    is the packaged `params/u_values.toml`.
  - `energy.screen.band_for_year(bands: list[Band], year: int | None) -> Band` —
    unknown year → first (oldest) band.
  - `energy.screen.screen_features(features_path: str, bands: list[Band],
    hdd: float, year_before: int | None, sv_above: float | None,
    top: int | None) -> pa.Table` — features columns + `u_roof, u_wall, u_ground,
    h_t_w_per_k, annual_kwh: float64`, `rank: int64` (1 = highest annual_kwh),
    filtered and truncated per the arguments.
    `h_t_w_per_k = u_roof·(a_roof_flat+a_roof_pitched) + u_wall·a_wall +
    u_ground·a_ground`; `annual_kwh = h_t_w_per_k · hdd · 24 / 1000`.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_screen.py
import duckdb
import pyarrow as pa
import pytest

from energy.screen import band_for_year, load_params, screen_features


def _features(tmp_path):
    table = pa.table({
        "building_id": ["old", "new"],
        "year": [1930, 2015],
        "sv_ratio": [0.9, 0.4],
        "a_roof_flat_m2": [50.0, 50.0],
        "a_roof_pitched_m2": [50.0, 0.0],
        "a_wall_m2": [200.0, 100.0],
        "a_ground_m2": [80.0, 60.0],
    })
    path = tmp_path / "features.parquet"
    duckdb.connect().register("t", table).execute(
        f"COPY t TO '{path}' (FORMAT PARQUET)"
    )
    return str(path)


def test_default_params_load_and_cover_all_years():
    bands = load_params()
    assert len(bands) >= 4
    assert bands[-1].max_year is None
    assert band_for_year(bands, None) == bands[0]      # unknown → oldest
    assert band_for_year(bands, 1900) == bands[0]
    assert band_for_year(bands, 2030) == bands[-1]


def test_screen_computes_ht_and_ranks(tmp_path):
    bands = load_params()
    out = screen_features(_features(tmp_path), bands, hdd=2900.0,
                          year_before=None, sv_above=None, top=None)
    rows = {r["building_id"]: r for r in out.to_pylist()}
    old, new = rows["old"], rows["new"]
    expected_ht = (old["u_roof"] * 100.0 + old["u_wall"] * 200.0
                   + old["u_ground"] * 80.0)
    assert old["h_t_w_per_k"] == pytest.approx(expected_ht)
    assert old["annual_kwh"] == pytest.approx(expected_ht * 2900.0 * 24 / 1000)
    assert old["annual_kwh"] > new["annual_kwh"]
    assert old["rank"] == 1 and new["rank"] == 2


def test_screen_filters(tmp_path):
    bands = load_params()
    only_old = screen_features(_features(tmp_path), bands, hdd=2900.0,
                               year_before=1975, sv_above=None, top=None)
    assert only_old.to_pylist()[0]["building_id"] == "old"
    assert only_old.num_rows == 1
    top1 = screen_features(_features(tmp_path), bands, hdd=2900.0,
                           year_before=None, sv_above=None, top=1)
    assert top1.num_rows == 1
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_screen.py -v`
Expected: FAIL with ImportError.

- [ ] **Step 3: Write minimal implementation**

```toml
# usecase/energy/src/energy/params/u_values.toml
# SCREENING-GRADE defaults, W/(m^2.K), by Dutch construction-year band.
# These are order-of-magnitude values for ranking only — NOT calibrated.
# Before any publication-grade run, replace with TABULA NL age-band values
# (episcope.eu) and cite them here.  Bands are matched as year <= max_year.

[[bands]]
name = "pre1946"
max_year = 1945
u_roof = 2.0
u_wall = 2.0
u_ground = 1.7

[[bands]]
name = "1946-1974"
max_year = 1974
u_roof = 1.5
u_wall = 1.6
u_ground = 1.7

[[bands]]
name = "1975-1991"
max_year = 1991
u_roof = 0.9
u_wall = 0.9
u_ground = 0.9

[[bands]]
name = "1992-2005"
max_year = 2005
u_roof = 0.4
u_wall = 0.5
u_ground = 0.6

[[bands]]
name = "post2005"
u_roof = 0.3
u_wall = 0.35
u_ground = 0.4
```

```python
# usecase/energy/src/energy/screen.py
"""Degree-day heat-loss screen and retrofit ranking over a feature table."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass
from pathlib import Path

import duckdb
import pyarrow as pa

_DEFAULT_PARAMS = Path(__file__).parent / "params" / "u_values.toml"


@dataclass
class Band:
    name: str
    max_year: int | None
    u_roof: float
    u_wall: float
    u_ground: float


def load_params(path: str | None = None) -> list[Band]:
    with open(path or _DEFAULT_PARAMS, "rb") as fh:
        raw = tomllib.load(fh)
    bands = [Band(b["name"], b.get("max_year"), b["u_roof"], b["u_wall"], b["u_ground"])
             for b in raw["bands"]]
    bands.sort(key=lambda b: (b.max_year is None, b.max_year))
    return bands


def band_for_year(bands: list[Band], year: int | None) -> Band:
    if year is None:
        return bands[0]
    for band in bands:
        if band.max_year is not None and year <= band.max_year:
            return band
    return bands[-1]


def screen_features(features_path: str, bands: list[Band], hdd: float,
                    year_before: int | None, sv_above: float | None,
                    top: int | None) -> pa.Table:
    con = duckdb.connect()
    rows = con.sql("SELECT * FROM read_parquet(?)",
                   params=[features_path]).arrow().to_pylist()
    out = []
    for r in rows:
        if year_before is not None and not (r["year"] and r["year"] < year_before):
            continue
        if sv_above is not None and not (r["sv_ratio"] and r["sv_ratio"] > sv_above):
            continue
        band = band_for_year(bands, r["year"])
        h_t = (band.u_roof * (r["a_roof_flat_m2"] + r["a_roof_pitched_m2"])
               + band.u_wall * r["a_wall_m2"]
               + band.u_ground * r["a_ground_m2"])
        out.append({**r, "u_roof": band.u_roof, "u_wall": band.u_wall,
                    "u_ground": band.u_ground, "h_t_w_per_k": h_t,
                    "annual_kwh": h_t * hdd * 24.0 / 1000.0})
    out.sort(key=lambda r: r["annual_kwh"], reverse=True)
    for i, r in enumerate(out):
        r["rank"] = i + 1
    if top is not None:
        out = out[:top]
    return pa.Table.from_pylist(out)
```

Note: `pyproject.toml`'s hatch config already packages `src/energy`, which includes
`params/u_values.toml`; confirm with `uv run python -c "from energy.screen import
load_params; print(load_params()[0])"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd usecase/energy && uv run pytest tests/test_screen.py -v`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/screen.py usecase/energy/src/energy/params \
        usecase/energy/tests/test_screen.py
git commit -m "feat(usecase): degree-day screen and retrofit ranking"
```

---

### Task 11: Wire the CLI

**Files:**
- Modify: `usecase/energy/src/energy/cli.py`
- Test: `usecase/energy/tests/test_cli_integration.py`

**Interfaces:**
- Consumes: `run.run_features`, `screen.load_params/screen_features`,
  `errors.EnergyError`.
- Produces: working `energy features` / `energy screen` commands; exit 0 on success
  with a summary on stdout; exit 1 with the message on stderr for any `EnergyError`.

- [ ] **Step 1: Write the failing test**

```python
# usecase/energy/tests/test_cli_integration.py
import duckdb

from energy.cli import main
from .conftest import requires_extensions


@requires_extensions
def test_features_then_screen_end_to_end(fixture_path, tmp_path, capsys):
    features = tmp_path / "features.parquet"
    rc = main(["features", "--input", str(fixture_path),
               "--output", str(features)])
    assert rc == 0
    assert "150 buildings" in capsys.readouterr().out

    screen = tmp_path / "screen.parquet"
    rc = main(["screen", "--features", str(features),
               "--year-before", "2100", "--top", "10",
               "--output", str(screen)])
    assert rc == 0
    table = duckdb.sql(f"FROM '{screen}'").arrow()
    assert table.num_rows == 10
    assert "annual_kwh" in table.column_names


def test_features_bad_lod_is_a_clean_error(fixture_path, capsys):
    rc = main(["features", "--input", str(fixture_path), "--lod", "9.9",
               "--output", "/dev/null"])
    assert rc == 1
    assert "available" in capsys.readouterr().err
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd usecase/energy && uv run pytest tests/test_cli_integration.py -v`
Expected: FAIL (`main` still returns 2 "not implemented").

- [ ] **Step 3: Replace `main` in `cli.py`**

```python
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
                  f"{summary.n_open_solids} open solids flagged)")
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
```

(The `not implemented` import of `sys` at module top level stays — it is still used
by the `__main__` guard.)

- [ ] **Step 4: Run the full suite**

Run: `cd usecase/energy && uv run pytest -v`
Expected: everything PASSES (Tasks 1–11 tests, integration included).

- [ ] **Step 5: Commit**

```bash
git add usecase/energy/src/energy/cli.py usecase/energy/tests/test_cli_integration.py
git commit -m "feat(usecase): wire the features and screen subcommands"
```

---

### Task 12: Documentation + justfile recipes

**Files:**
- Create: `usecase/README.md`
- Create: `usecase/energy/README.md`
- Modify: `justfile` (monorepo root — add a `# ── usecase ──` section at the end)

**Interfaces:**
- Consumes: the spec `ai/design-notes/specs/2026-09-02-usecase-energy-design.md`
  (methodology section and citations are adapted from it, not rewritten from
  scratch).

- [ ] **Step 1: Write `usecase/README.md`**

Content requirements (write in full, British English):
- What `usecase/` is: literature-anchored, individually executable CLI tools, one
  directory per use case, demonstrating the CityParquet + DuckDB stack on real
  urban-analysis problems. Each tool: uv project, headless, CityParquet path in,
  Parquet out.
- Table of current tools (one row: `energy/`).
- Pointer: candidate rationale lives in the paper repository's use-case candidates
  note; design specs under `ai/design-notes/specs/`.

- [ ] **Step 2: Write `usecase/energy/README.md`**

Section order is fixed (methodology first — this ordering was an explicit
requirement):
1. **Methodology** — adapt the spec's "Methodology from the literature" section:
   UBEM envelope inputs (Nouvel et al. 2015; Agugiaro et al. 2018,
   doi:10.1186/s40965-018-0042-y; León-Sánchez et al. 2021), degree-day screening
   `H_T = Σ Uᵢ·Aᵢ`, `annual kWh ≈ H_T·HDD·24/1000` (Rode et al. 2014,
   doi:10.1068/b39065), retrofit prioritisation (Evans et al. 2017,
   doi:10.1177/0265813516652898; Steadman et al. 2020, doi:10.5334/bc.52;
   Chen et al. 2017).
2. **How it maps onto the stack** — SQL core functions used; the face module as the
   `ST_3DFaces` prototype with its output schema table; Building/BuildingPart join;
   no-load-step + columnar pruning.
3. **Usage** — both commands with the exact flags from `cli.py`, plus an example
   against a local tile and the s3:// note (httpfs).
4. **Validation & limitations** — the `--validate` report and the b3_* pairs table;
   party walls counted in `a_wall_m2` (reference columns expose 3DBAG's own split);
   screening-grade U-values pending TABULA NL confirmation; migration path when
   `ST_3DFaces` lands.

- [ ] **Step 3: Add justfile recipes** (append to the monorepo root `justfile`)

```just
# ── usecase ──────────────────────────────────────────────────────────

# run the energy tool's test suite
usecase-energy-test:
    cd usecase/energy && uv run pytest

# extract features from a CityParquet package: just usecase-energy-features IN OUT
usecase-energy-features input output="features.parquet":
    cd usecase/energy && uv run energy features --input '{{input}}' --output '{{output}}'
```

- [ ] **Step 4: Verify docs build nothing is broken**

Run: `just usecase-energy-test`
Expected: full suite passes from the repo root.
Run: `just usecase-energy-features /data2/hideba/cityparquet_data/10-756-44/building.parquet /tmp/features.parquet`
Expected: exits 0, prints the building count for the full tile.

- [ ] **Step 5: Commit**

```bash
git add usecase/README.md usecase/energy/README.md justfile
git commit -m "docs(usecase): document the energy tool, methodology first"
```

---

### Task 13: Full-tile validation run (verification, no new code)

- [ ] **Step 1: Run the tool on the complete local tile with validation**

```bash
cd usecase/energy && uv run energy features \
  --input /data2/hideba/cityparquet_data/10-756-44/building.parquet \
  --output /tmp/tile-features.parquet \
  --faces /tmp/tile-faces.parquet \
  --validate /tmp/tile-report.json
```

- [ ] **Step 2: Inspect the report**

```bash
cd usecase/energy && uv run python -c "
import json
r = json.load(open('/tmp/tile-report.json'))
for k, v in r.items():
    print(k, 'n=', v['n'], 'median_rel_err_pct=', v['median_rel_err_pct'])
"
```

Expected: volume median error well under 1%; ground close; roof/wall larger but
explainable (party walls, semantic edge cases). Record the numbers in the final
summary to the user. If any metric is wildly off (>25% median), stop and debug
before declaring done — that is a correctness bug, not a tolerance issue.

- [ ] **Step 3: Run the screen end-to-end**

```bash
cd usecase/energy && uv run energy screen \
  --features /tmp/tile-features.parquet --year-before 1975 --top 25 \
  --output /tmp/tile-screen.parquet
```

Expected: exits 0; 25 rows ranked.

- [ ] **Step 4: Full suite one last time**

Run: `cd usecase/energy && uv run pytest`
Expected: all pass, extension-dependent tests included.

No commit — this task produces evidence, not code.

---

## Self-Review Notes

- **Spec coverage:** methodology doc (Task 12), SQL core (Task 7), face prototype
  (Tasks 4–6, 8), screening + params (Task 10), validation (Task 9), CLI + errors
  (Tasks 1, 11), fixture + skip markers (Tasks 2–3), justfile (Task 12), full-tile
  evidence (Task 13). S3/httpfs is wired in `db.connect`/`run_features` and
  documented; a live S3 test is out of scope (no bucket in CI).
- **Known judgement points for the implementer:** fixture LoD list assertion
  (Task 7 Step 4 note), cube winding vs azimuth labels (Task 5 Step 4 note),
  ground-area tolerance (Task 8 Step 4 note), `faces_out` two-statement note
  (Task 8 Step 3).
- **Type consistency check:** `build_features` and `compute_face_tables` both key on
  `building_id: str`; `run_features` joins on it; `screen_features` consumes the
  written feature columns by exact name; CLI flag names match `argparse` dests used
  in `main`.
