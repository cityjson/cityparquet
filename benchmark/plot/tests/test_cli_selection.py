import json
from pathlib import Path

from benchviz import __main__ as cli


def test_selection_filters_real_payload(tmp_path: Path, monkeypatch):
    def fake_prep(inputs, out_path, statistic="median"):
        payload = {
            "datasets": [{"id": "vienna_102081"}, {"id": "other"}],
            "read": [{"dataset": "vienna_102081", "scenario_key": "write"}],
            "sizes": [{"dataset": "vienna_102081", "bytes": 12}],
            "bloom": {"records": [], "sizes": []},
            "databases": {
                "dataset": "3dbag_n1000",
                "records": [{"scenario": "count"}],
                "sizes": [{"format": "cjdb"}],
            },
        }
        out_path.write_text(json.dumps(payload))

    monkeypatch.setattr(cli.prep, "main", fake_prep)
    for family in ("sizes", "formats", "bloom"):
        args = cli.build_parser().parse_args(
            [
                "prep",
                "--out",
                str(tmp_path),
                "--families",
                family,
                "--datasets",
                "vienna",
            ]
        )
        cli._cmd_prep(args)
        payload = json.loads((tmp_path / "bench_data.json").read_text())
        assert payload["datasets"] == [{"id": "vienna_102081"}]
        assert bool(payload["read"]) == (family == "formats")
        assert bool(payload["sizes"]) == (family == "sizes")
        assert payload["databases"]["records"] == []
        assert payload["databases"]["sizes"] == []
