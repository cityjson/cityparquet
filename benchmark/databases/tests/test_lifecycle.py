from pathlib import Path

import pytest

from citybench import engine, lifecycle




def test_rejects_a_data_root_outside_the_benchmark_root():
    with pytest.raises(ValueError):
        with lifecycle.isolated_databases(Path('/tmp/not-bench'), 7415):
            pass


def test_context_uses_unique_run_directories_and_cleans_created_containers(tmp_path, monkeypatch):
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    calls = []
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: calls.append(args) or ('127.0.0.1:40123\n' if args[1] == 'port' else ''))
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: calls.append(tuple(args)))
    with lifecycle.isolated_databases(tmp_path, 7415) as first:
        assert first['ports'] == {'cjdb': 40123, '3dcitydb': 40123}
        first_root = first['run_root']
        first_temp = first['temp_root']
        assert first_temp.parent == tmp_path / "tmp"
        assert first_temp.stat().st_mode & 0o7777 == 0o1777
        assert set(first['temp_dirs']) == {"cjdb", "3dcitydb", "citydb-tool", "duckdb"}
        assert len(set(first['temp_dirs'].values())) == 4
        assert all(directory.stat().st_mode & 0o7777 == 0o1777 for directory in first['temp_dirs'].values())
        starts = [call for call in calls if len(call) > 2 and call[1] == 'run']
        assert f"{first['temp_dirs']['cjdb']}:/tmp" in starts[0]
        assert f"{first['temp_dirs']['3dcitydb']}:/tmp" in starts[1]
        assert "127.0.0.1:40123:5432" in starts[0]
    assert not first_temp.exists()
    with lifecycle.isolated_databases(tmp_path, 7415) as second:
        assert second['run_root'] != first_root
    starts = [call for call in calls if len(call) > 2 and call[1] == 'run']
    stops = [call for call in calls if len(call) > 1 and call[1] == 'stop']
    assert len(starts) == 4
    assert len(stops) == 4


def test_benchmark_temp_directory_defaults_to_user_owned_location(monkeypatch):
    monkeypatch.delenv("CITYBENCH_TEMP_DIR", raising=False)
    monkeypatch.delenv("TMPDIR", raising=False)
    assert lifecycle.benchmark_temp_directory() == Path("/data2/hideba/tmp")


def test_container_args_are_spliced_into_every_postgresql_container(tmp_path, monkeypatch):
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    calls = []
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: calls.append(args) or ('127.0.0.1:40123\n' if args[1] == 'port' else ''))
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: calls.append(tuple(args)))
    flags = ["--cpuset-cpus=32-63", "--cpuset-mems=1"]
    with lifecycle.isolated_databases(tmp_path, 7415, container_args=flags):
        pass
    starts = [call for call in calls if len(call) > 2 and call[1] == 'run']
    assert len(starts) == 2
    assert all(set(flags) <= set(start) for start in starts)


def test_rejected_cpuset_flags_restart_the_containers_without_them(tmp_path, monkeypatch):
    import subprocess
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    calls = []

    def fake_run(*args, **kwargs):
        calls.append(args)
        if args[1] == 'run' and "--cpuset-mems=1" in args:
            raise subprocess.CalledProcessError(125, args)
        return '127.0.0.1:40123\n' if args[1] == 'port' else ''

    monkeypatch.setattr(lifecycle, '_run', fake_run)
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: calls.append(tuple(args)))
    flags = ["--cpuset-cpus=32-63", "--cpuset-mems=1"]
    with lifecycle.isolated_databases(tmp_path, 7415, container_args=flags) as context:
        assert context["container_args_applied"] is False
    started = [c for c in calls if c[1] == 'run' and "--cpuset-mems=1" not in c]
    assert len(started) == 2


def test_container_args_applied_is_reported(tmp_path, monkeypatch):
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: '127.0.0.1:40123\n' if args[1] == 'port' else '')
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: None)
    with lifecycle.isolated_databases(tmp_path, 7415, container_args=["--cpuset-mems=0"]) as context:
        assert context["container_args_applied"] is True


def test_runs_root_override_admits_a_data_root_outside_the_repository(tmp_path, monkeypatch):
    """CITYBENCH_RUNS_ROOT moves the permitted root, so a scratch run never
    writes under the repository's own benchmark/runs/."""
    monkeypatch.setenv("CITYBENCH_RUNS_ROOT", str(tmp_path))
    assert lifecycle.runs_root() == tmp_path.resolve()
    monkeypatch.delenv("CITYBENCH_RUNS_ROOT")
    assert lifecycle.runs_root() == lifecycle.ROOT


def test_database_containers_name_the_amd64_platform(tmp_path, monkeypatch):
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    calls = []
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: calls.append(args) or '')
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: None)
    with lifecycle.isolated_databases(tmp_path, 7415):
        pass
    runs = [c for c in calls if 'run' in c[:2]]
    assert len(runs) == 2
    for argv in runs:
        assert argv[argv.index('--platform') + 1] == lifecycle.DB_PLATFORM == 'linux/amd64'


def test_apple_container_keeps_pgdata_inside_the_container(tmp_path, monkeypatch):
    from citybench import engine as eng
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    monkeypatch.setenv("TMPDIR", str(tmp_path / "tmp"))
    calls = []
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: calls.append(args) or '')
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: None)
    eng.set_active(eng.Engine(name="container", binary="container", version="1.0.0", run_flags=frozenset(), platform="darwin"))
    try:
        with lifecycle.isolated_databases(tmp_path, 7415):
            pass
    finally:
        eng.set_active(None)
    for argv in [c for c in calls if 'run' in c[:2]]:
        assert f"PGDATA={lifecycle.CONTAINER_PGDATA}" in argv
        assert not any(a.endswith(":/var/lib/postgresql/data") for a in argv)
