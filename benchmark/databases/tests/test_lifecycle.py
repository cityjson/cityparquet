from pathlib import Path

import pytest

from citybench import lifecycle


def test_rejects_a_data_root_outside_the_benchmark_root():
    with pytest.raises(ValueError):
        with lifecycle.isolated_databases(Path('/tmp/not-bench'), 7415):
            pass


def test_context_uses_unique_run_directories_and_cleans_created_containers(tmp_path, monkeypatch):
    monkeypatch.setattr(lifecycle, 'ROOT', tmp_path)
    calls = []
    monkeypatch.setattr(lifecycle, '_run', lambda *args, **kwargs: calls.append(args) or ('127.0.0.1:40123\n' if args[1] == 'port' else ''))
    monkeypatch.setattr(lifecycle, '_wait', lambda *args, **kwargs: None)
    monkeypatch.setattr(lifecycle.subprocess, 'run', lambda args, **kwargs: calls.append(tuple(args)))
    with lifecycle.isolated_databases(tmp_path, 7415) as first:
        assert first['ports'] == {'cjdb': 40123, '3dcitydb': 40123}
        first_root = first['run_root']
    with lifecycle.isolated_databases(tmp_path, 7415) as second:
        assert second['run_root'] != first_root
    starts = [call for call in calls if len(call) > 2 and call[1] == 'run']
    stops = [call for call in calls if len(call) > 1 and call[1] == 'stop']
    assert len(starts) == 4
    assert len(stops) == 4
