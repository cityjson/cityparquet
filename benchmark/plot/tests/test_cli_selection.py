import json
from pathlib import Path

from benchviz import __main__ as cli


def test_selection_filters_real_payload(tmp_path: Path, monkeypatch):
    def fake_prep(inputs, out_path):
        payload = {
            "datasets": [{"id": "rotterdam_delfshaven"}, {"id": "other"}],
            "read": [{"dataset": "rotterdam_delfshaven", "scenario_key": "write"}],
            "sizes": [{"dataset": "rotterdam_delfshaven", "bytes": 12}],
            "scaling": {
                "codec": {"records": [], "sizes": []},
                "rowgroup": {"records": [], "sizes": []},
                "datasets": [],
            },
            "databases": {
                "dataset": "3dbag_n1000",
                "records": [{"scenario": "count"}],
                "sizes": [{"format": "cjdb"}],
            },
        }
        out_path.write_text(json.dumps(payload))

    monkeypatch.setattr(cli.prep, "main", fake_prep)
    for family in ("sizes", "formats", "codec"):
        args = cli.build_parser().parse_args(
            [
                "prep",
                "--out",
                str(tmp_path),
                "--families",
                family,
                "--datasets",
                "rotterdam",
            ]
        )
        cli._cmd_prep(args)
        payload = json.loads((tmp_path / "bench_data.json").read_text())
        assert payload["datasets"] == [{"id": "rotterdam_delfshaven"}]
        assert bool(payload["read"]) == (family == "formats")
        assert bool(payload["sizes"]) == (family == "sizes")
        assert payload["databases"]["records"] == []
        assert payload["databases"]["sizes"] == []
