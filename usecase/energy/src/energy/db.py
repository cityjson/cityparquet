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
