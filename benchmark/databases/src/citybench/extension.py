"""The build of the DuckDB CityJSON extension the harness loads.

The write tier runs through the extension's package model
(`PRAGMA cityparquet_read`, `PRAGMA insert_cityjsonseq`,
`cityparquet_write`), so which build answered is part of the result. A bare
`LOAD cityjson` answers with whatever community build is installed under
`~/.duckdb/extensions/`, which need not carry the submodule's fixes. The
harness therefore loads one explicitly chosen file, by path:

1. the path passed to `resolve` (the CLI's `--duckdb-cityjson-extension`);
2. else the path in ``CITYBENCH_DUCKDB_CITYJSON_EXTENSION``;
3. else the submodule's local release build, `DEFAULT_BUILD`.

When none of them names an existing file the harness refuses to start the
write tier rather than fall back to `LOAD cityjson`.

A locally built extension is unsigned, so every connection that loads it is
opened with ``allow_unsigned_extensions``. A C++ DuckDB extension loads only
into the exact DuckDB version it was built for; `pyproject.toml` pins the
Python package to the version the submodule builds against.
"""
from __future__ import annotations

import hashlib
import os
import subprocess
from pathlib import Path

import duckdb

ENV_VAR = "CITYBENCH_DUCKDB_CITYJSON_EXTENSION"

#: `lib/duckdb-cityjson` in the monorepo.
SUBMODULE = Path(__file__).resolve().parents[4] / "lib" / "duckdb-cityjson"

#: What `just -f lib/duckdb-cityjson/justfile build` produces.
DEFAULT_BUILD = (
    SUBMODULE / "build" / "release" / "extension" / "cityjson"
    / "cityjson.duckdb_extension"
)


class ExtensionNotFound(RuntimeError):
    """No explicitly chosen extension build exists."""


def resolve(explicit: Path | str | None = None) -> Path:
    """The extension file to load: explicit path, else env, else the local build."""
    chosen = explicit or os.environ.get(ENV_VAR)
    if chosen:
        path = Path(chosen)
        if not path.is_file():
            raise ExtensionNotFound(f"DuckDB CityJSON extension build not found: {path}")
        return path
    if DEFAULT_BUILD.is_file():
        return DEFAULT_BUILD
    raise ExtensionNotFound(
        f"no DuckDB CityJSON extension build at {DEFAULT_BUILD}; build it "
        f"(`just -f lib/duckdb-cityjson/justfile build`) or set {ENV_VAR} "
        "to a build for this DuckDB version"
    )


def connect(**config: str) -> duckdb.DuckDBPyConnection:
    """A connection allowed to load the unsigned local build."""
    return duckdb.connect(config={"allow_unsigned_extensions": "true", **config})


def load(conn, path: Path) -> None:
    """`LOAD` the extension by file path, never by name."""
    conn.execute("LOAD '{}'".format(str(path).replace("'", "''")))


def reported_version(conn) -> str | None:
    """The version string the loaded extension reports about itself.

    Only the version: `duckdb_extensions()` reports `install_path` from the
    catalogue of installed extensions, not from the file that was loaded.
    """
    row = conn.execute(
        "SELECT extension_version FROM duckdb_extensions() "
        "WHERE extension_name = 'cityjson' AND loaded"
    ).fetchone()
    return None if row is None else str(row[0])


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(SUBMODULE), *args],
        capture_output=True, text=True, check=True,
    ).stdout.strip()


def provenance(path: Path, *, reported: str | None) -> dict[str, str]:
    """Flat manifest entries naming the build that answered.

    The commit is the submodule's checked-out commit, with ``+dirty`` when
    its working tree differs, and is stated only for a build inside the
    submodule. The extension's self-reported version is recorded beside it
    because the two can differ (a build made before its commit reports the
    parent commit); the file's SHA-256 is what identifies it exactly.
    """
    digest = hashlib.sha256(Path(path).read_bytes()).hexdigest()
    try:
        inside = Path(path).resolve().is_relative_to(SUBMODULE.resolve())
    except OSError:
        inside = False
    if inside:
        try:
            commit = _git("rev-parse", "HEAD")
            if _git("status", "--porcelain"):
                commit += "+dirty"
        except (OSError, subprocess.CalledProcessError) as exc:
            commit = f"unknown: {exc}"
    else:
        commit = "unknown: not inside lib/duckdb-cityjson"
    return {
        "duckdb": duckdb.__version__,
        "duckdb-cityjson": reported or "unknown",
        "duckdb-cityjson-path": str(path),
        "duckdb-cityjson-commit": commit,
        "duckdb-cityjson-sha256": digest,
    }
