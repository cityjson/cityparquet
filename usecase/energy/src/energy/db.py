"""DuckDB session setup: load the ecosystem extensions.

By default the signed community builds, `INSTALL cityjson FROM community`
and `INSTALL three_d FROM community`. `--ext-dir` names a directory of local
builds instead (`cityjson.duckdb_extension`, `three_d.duckdb_extension`),
which are unsigned and built for one DuckDB version.
"""
from __future__ import annotations

from pathlib import Path

import duckdb

from .errors import ExtensionsNotFound

EXTENSIONS = ("cityjson", "three_d")


def find_extensions(ext_dir: Path) -> dict[str, Path]:
    """The local builds in `ext_dir`, one per extension."""
    candidates = {name: Path(ext_dir) / f"{name}.duckdb_extension"
                  for name in EXTENSIONS}
    missing = [p for p in candidates.values() if not p.is_file()]
    if missing:
        tried = "\n  ".join(str(p) for p in candidates.values())
        raise ExtensionsNotFound(
            "could not find the duckdb extensions; tried:\n  " + tried
            + "\nbuild them (see lib/duckdb-3d/README.md), or leave out "
            "--ext-dir to install the community builds"
        )
    return candidates


def extensions_available(ext_dir: Path | None = None) -> bool:
    try:
        connect(ext_dir).close()
        return True
    except (ExtensionsNotFound, duckdb.Error):
        return False


def connect(ext_dir: Path | None = None,
            need_httpfs: bool = False) -> duckdb.DuckDBPyConnection:
    if ext_dir is None:
        con = duckdb.connect()
        for name in EXTENSIONS:
            con.install_extension(name, repository="community")
            con.load_extension(name)
    else:
        paths = find_extensions(ext_dir)
        con = duckdb.connect(config={"allow_unsigned_extensions": True})
        for path in paths.values():
            con.load_extension(str(path))
    if need_httpfs:
        con.install_extension("httpfs")
        con.load_extension("httpfs")
    return con
