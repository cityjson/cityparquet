"""Which build of the DuckDB CityJSON extension the write tier loads.

A bare `LOAD cityjson` answers with whatever build happens to be installed
under `~/.duckdb/extensions/`, which is how an old build refused
`append-object`. The harness therefore loads a named local build by path,
or the community build only after checking the version it reports.
"""
from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

from citybench import extension


class Conn:
    """Records statements; answers `duckdb_extensions()` with `version`."""

    def __init__(self, version=extension.COMMUNITY_VERSION, path="/ext/cityjson.duckdb_extension"):
        self.executed = []
        self._row = None
        self._version = version
        self._path = path

    def execute(self, sql, *args):
        self.executed.append(sql)
        if "extension_version" in sql:
            self._row = (self._version,)
        elif "install_path" in sql:
            self._row = (self._path,)
        return self

    def fetchone(self):
        return self._row


def test_explicit_path_wins_over_env_and_community(tmp_path, monkeypatch):
    explicit = tmp_path / "a.duckdb_extension"
    explicit.write_bytes(b"x")
    env = tmp_path / "b.duckdb_extension"
    env.write_bytes(b"y")
    monkeypatch.setenv(extension.ENV_VAR, str(env))
    assert extension.resolve(explicit) == explicit


def test_env_wins_over_community(tmp_path, monkeypatch):
    env = tmp_path / "b.duckdb_extension"
    env.write_bytes(b"y")
    monkeypatch.setenv(extension.ENV_VAR, str(env))
    assert extension.resolve() == env


def test_default_is_the_community_build(monkeypatch):
    monkeypatch.delenv(extension.ENV_VAR, raising=False)
    assert extension.resolve() is extension.COMMUNITY


def test_a_named_file_that_does_not_exist_is_refused(tmp_path, monkeypatch):
    monkeypatch.setenv(extension.ENV_VAR, str(tmp_path / "nope"))
    with pytest.raises(extension.ExtensionNotFound, match="nope"):
        extension.resolve()


def test_a_local_build_is_loaded_by_its_path_never_by_name(tmp_path):
    conn = Conn()
    path = tmp_path / "it's.duckdb_extension"
    extension.load(conn, path)
    assert conn.executed == [f"LOAD '{str(path).replace(chr(39), chr(39) * 2)}'"]


def test_the_community_build_is_installed_loaded_and_checked():
    conn = Conn()
    extension.load(conn, extension.COMMUNITY)
    assert conn.executed[:2] == ["INSTALL cityjson FROM community", "LOAD cityjson"]
    assert "extension_version" in conn.executed[2]


def test_a_community_build_of_another_version_is_refused():
    with pytest.raises(extension.WrongVersion, match="FORCE INSTALL cityjson FROM community"):
        extension.load(Conn(version="v0.4.0"), extension.COMMUNITY)


def test_only_a_local_build_allows_unsigned_extensions(tmp_path, monkeypatch):
    seen = {}
    monkeypatch.setattr(extension.duckdb, "connect",
                        lambda **kw: seen.update(kw) or "conn")
    build = tmp_path / "cityjson.duckdb_extension"
    assert extension.connect(build) == "conn"
    assert seen["config"]["allow_unsigned_extensions"] == "true"
    extension.connect(extension.COMMUNITY, threads="2")
    assert seen["config"] == {"threads": "2"}


def test_provenance_of_a_local_build_records_path_sha_and_duckdb_version(tmp_path):
    build = tmp_path / "cityjson.duckdb_extension"
    build.write_bytes(b"abc")
    record = extension.provenance(build, reported="6937c06")
    assert record["duckdb-cityjson-source"] == "local"
    assert record["duckdb-cityjson-path"] == str(build)
    assert record["duckdb-cityjson-sha256"].startswith("ba7816bf")
    assert record["duckdb-cityjson"] == "6937c06"
    assert record["duckdb"]
    # Outside the submodule there is no commit to state.
    assert record["duckdb-cityjson-commit"] == "unknown: not inside lib/duckdb-cityjson"


def test_provenance_of_the_community_build_names_its_file_and_commit(tmp_path):
    installed = tmp_path / "cityjson.duckdb_extension"
    installed.write_bytes(b"abc")
    record = extension.provenance(extension.COMMUNITY,
                                  reported=extension.COMMUNITY_VERSION, file=installed)
    assert record["duckdb-cityjson-source"] == "community"
    assert record["duckdb-cityjson-path"] == str(installed)
    assert record["duckdb-cityjson-sha256"].startswith("ba7816bf")
    assert record["duckdb-cityjson-commit"].startswith(extension.COMMUNITY_COMMIT)


def test_installed_path_reads_the_catalogue():
    assert extension.installed_path(Conn(path="/x/cityjson.duckdb_extension")) == Path(
        "/x/cityjson.duckdb_extension")


@pytest.mark.skipif(shutil.which("git") is None, reason="needs git")
def test_the_community_commit_is_the_one_the_submodule_pins():
    """The checked community release is the measured source, or this fails.

    Moving `lib/duckdb-cityjson` without moving `COMMUNITY_VERSION` and
    `COMMUNITY_COMMIT` would measure a community build of other code.
    """
    root = extension.SUBMODULE.parents[1]
    tree = subprocess.run(
        ["git", "-C", str(root), "ls-tree", "HEAD", "lib/duckdb-cityjson"],
        capture_output=True, text=True,
    )
    if tree.returncode != 0 or not tree.stdout:
        pytest.skip("not inside the cityparquet repository")
    assert tree.stdout.split()[2] == extension.COMMUNITY_COMMIT
