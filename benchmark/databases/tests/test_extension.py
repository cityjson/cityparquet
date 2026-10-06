"""Which build of the DuckDB CityJSON extension the write tier loads.

A bare `LOAD cityjson` answers with whatever community build happens to be
installed under `~/.duckdb/extensions/`, which is how an old build refused
`append-object`. The harness therefore loads an explicitly chosen file.
"""
from __future__ import annotations

from pathlib import Path

import pytest

from citybench import extension


def test_explicit_path_wins_over_env_and_default(tmp_path, monkeypatch):
    explicit = tmp_path / "a.duckdb_extension"
    explicit.write_bytes(b"x")
    env = tmp_path / "b.duckdb_extension"
    env.write_bytes(b"y")
    monkeypatch.setenv(extension.ENV_VAR, str(env))
    assert extension.resolve(explicit) == explicit


def test_env_wins_over_default(tmp_path, monkeypatch):
    env = tmp_path / "b.duckdb_extension"
    env.write_bytes(b"y")
    monkeypatch.setenv(extension.ENV_VAR, str(env))
    assert extension.resolve() == env


def test_default_is_the_submodules_local_release_build(tmp_path, monkeypatch):
    monkeypatch.delenv(extension.ENV_VAR, raising=False)
    build = tmp_path / "cityjson.duckdb_extension"
    build.write_bytes(b"z")
    monkeypatch.setattr(extension, "DEFAULT_BUILD", build)
    assert extension.resolve() == build
    assert extension.DEFAULT_BUILD.name == "cityjson.duckdb_extension"


def test_default_build_path_points_into_the_submodule():
    parts = extension.SUBMODULE.parts
    assert parts[-2:] == ("lib", "duckdb-cityjson")
    assert str(extension.DEFAULT_BUILD).startswith(str(extension.SUBMODULE))


def test_no_build_anywhere_refuses_rather_than_falling_back_to_load(tmp_path, monkeypatch):
    monkeypatch.delenv(extension.ENV_VAR, raising=False)
    monkeypatch.setattr(extension, "DEFAULT_BUILD", tmp_path / "missing")
    with pytest.raises(extension.ExtensionNotFound, match=extension.ENV_VAR):
        extension.resolve()


def test_a_named_file_that_does_not_exist_is_refused(tmp_path, monkeypatch):
    monkeypatch.setenv(extension.ENV_VAR, str(tmp_path / "nope"))
    with pytest.raises(extension.ExtensionNotFound, match="nope"):
        extension.resolve()


def test_load_statement_names_the_file_never_the_bare_extension(tmp_path):
    executed = []

    class Conn:
        def execute(self, sql, *args):
            executed.append(sql)
            return self

    path = tmp_path / "it's.duckdb_extension"
    extension.load(Conn(), path)
    assert executed == [f"LOAD '{str(path).replace(chr(39), chr(39) * 2)}'"]


def test_connect_allows_unsigned_extensions(monkeypatch):
    seen = {}
    monkeypatch.setattr(extension.duckdb, "connect",
                        lambda **kw: seen.update(kw) or "conn")
    assert extension.connect() == "conn"
    assert seen["config"]["allow_unsigned_extensions"] == "true"


def test_provenance_records_path_sha_and_duckdb_version(tmp_path):
    build = tmp_path / "cityjson.duckdb_extension"
    build.write_bytes(b"abc")
    record = extension.provenance(build, reported="6937c06")
    assert record["duckdb-cityjson-path"] == str(build)
    assert record["duckdb-cityjson-sha256"].startswith("ba7816bf")
    assert record["duckdb-cityjson"] == "6937c06"
    assert record["duckdb"]
    # Outside the submodule there is no commit to state.
    assert record["duckdb-cityjson-commit"] == "unknown: not inside lib/duckdb-cityjson"
