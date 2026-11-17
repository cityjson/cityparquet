"""The build of the DuckDB CityJSON extension the harness loads.

The write tier runs through the extension's package model
(`PRAGMA cityparquet_read`, `PRAGMA insert_cityjsonseq`,
`cityparquet_write`), so which build answered is part of the result. The
harness loads one explicitly chosen build:

1. the path passed to `resolve` (the CLI's `--duckdb-cityjson-extension`);
2. else the path in ``CITYBENCH_DUCKDB_CITYJSON_EXTENSION``;
3. else the signed community build, `INSTALL cityjson FROM community`.

The community build is accepted only when it reports `COMMUNITY_VERSION`.
DuckDB's community repository serves one build per DuckDB version, the
latest the extension's `description.yml` names, and `INSTALL` keeps
whatever is already under `~/.duckdb/extensions/`; either can be a build
other than the one the submodule pins, which is how an old build once
refused `append-object`. A different version refuses the run rather than
measuring it.

A path names a local build. That is unsigned, so a connection that loads
one is opened with ``allow_unsigned_extensions``. A C++ DuckDB extension
loads only into the exact DuckDB version it was built for; `pyproject.toml`
pins the Python package to the version the submodule builds against.
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

#: The release the community repository builds for DuckDB 2.0.0, and the
#: commit its `description.yml` builds it from
#: (`duckdb/community-extensions`, `extensions/cityjson/description.yml`,
#: `repo.ref`). The commit is the one `lib/duckdb-cityjson` pins; a test
#: holds the two together, so moving the submodule means moving these.
COMMUNITY_VERSION = "v0.5.0"
COMMUNITY_COMMIT = "df6f0e1c07642191a05b1e576b44926597a7eec8"


class Community:
    """The signed build from DuckDB's community extension repository."""

    def __repr__(self) -> str:
        return "community"

    def __str__(self) -> str:
        return f"community {COMMUNITY_VERSION}"


#: The one community build value; compare with ``is``.
COMMUNITY = Community()

#: What the harness loads: a local build by path, or the community build.
Build = Path | Community


class ExtensionNotFound(RuntimeError):
    """An explicitly named extension build does not exist."""


class WrongVersion(RuntimeError):
    """The community build that loaded is not `COMMUNITY_VERSION`."""


def resolve(explicit: Path | str | None = None) -> Build:
    """The build to load: explicit path, else env path, else the community build."""
    chosen = explicit or os.environ.get(ENV_VAR)
    if not chosen:
        return COMMUNITY
    path = Path(chosen)
    if not path.is_file():
        raise ExtensionNotFound(f"DuckDB CityJSON extension build not found: {path}")
    return path


def connect(build: Build | None = None, **config: str) -> duckdb.DuckDBPyConnection:
    """A connection for `build`, allowed to load unsigned code only for a local build."""
    build = resolve() if build is None else build
    if build is COMMUNITY:
        return duckdb.connect(config=dict(config))
    return duckdb.connect(config={"allow_unsigned_extensions": "true", **config})


def load(conn, build: Build) -> None:
    """`LOAD` a local build by file path, or install and check the community build."""
    if build is COMMUNITY:
        conn.execute("INSTALL cityjson FROM community")
        conn.execute("LOAD cityjson")
        reported = reported_version(conn)
        if reported != COMMUNITY_VERSION:
            raise WrongVersion(
                f"the community DuckDB CityJSON extension reports {reported}, "
                f"not {COMMUNITY_VERSION}. An older installed build is replaced "
                "by `FORCE INSTALL cityjson FROM community`; if the community "
                "repository has moved past it, pass --duckdb-cityjson-extension "
                f"(or {ENV_VAR}) a local build of lib/duckdb-cityjson at "
                f"{COMMUNITY_COMMIT[:7]}"
            )
        return
    conn.execute("LOAD '{}'".format(str(build).replace("'", "''")))


def reported_version(conn) -> str | None:
    """The version string the loaded extension reports about itself."""
    row = conn.execute(
        "SELECT extension_version FROM duckdb_extensions() "
        "WHERE extension_name = 'cityjson' AND loaded"
    ).fetchone()
    return None if row is None else str(row[0])


def installed_path(conn) -> Path | None:
    """The installed file of the loaded extension.

    `duckdb_extensions()` reports `install_path` from the catalogue of
    installed extensions, not from the file that was loaded, so this names
    the loaded file only for a build loaded by name: the community build.
    """
    row = conn.execute(
        "SELECT install_path FROM duckdb_extensions() "
        "WHERE extension_name = 'cityjson' AND loaded"
    ).fetchone()
    return None if row is None or not row[0] else Path(row[0])


def _git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(SUBMODULE), *args],
        capture_output=True, text=True, check=True,
    ).stdout.strip()


def provenance(build: Build, *, reported: str | None,
               file: Path | None = None) -> dict[str, str]:
    """Flat manifest entries naming the build that answered.

    For a local build, the commit is the submodule's checked-out commit,
    with ``+dirty`` when its working tree differs, and is stated only for a
    build inside the submodule; the extension's self-reported version is
    recorded beside it because the two can differ (a build made before its
    commit reports the parent commit). For the community build, the commit
    is the one the community repository builds `COMMUNITY_VERSION` from,
    and `file` is the installed file. Either way the file's SHA-256 is what
    identifies the build exactly.
    """
    if build is COMMUNITY:
        path = file
        commit = f"{COMMUNITY_COMMIT} (community build of {COMMUNITY_VERSION})"
    else:
        path = build
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
    digest = (hashlib.sha256(Path(path).read_bytes()).hexdigest()
              if path is not None and Path(path).is_file() else "unknown")
    return {
        "duckdb": duckdb.__version__,
        "duckdb-cityjson": reported or "unknown",
        "duckdb-cityjson-source": "community" if build is COMMUNITY else "local",
        "duckdb-cityjson-path": str(path) if path is not None else "unknown",
        "duckdb-cityjson-commit": commit,
        "duckdb-cityjson-sha256": digest,
    }
