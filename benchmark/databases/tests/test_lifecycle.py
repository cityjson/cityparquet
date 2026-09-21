from pathlib import Path

import pytest

from citybench import lifecycle


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
