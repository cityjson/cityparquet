"""Size accounting of a CityParquet package with and without its indexes."""

import json

import pyarrow as pa
import pyarrow.parquet as pq

from citybench.parquet_sizes import package_sizes, parquet_file_sizes


def _write(path, *, page_index: bool):
    table = pa.table({"id": [f"obj-{i}" for i in range(5000)],
                      "h": [float(i) for i in range(5000)]})
    pq.write_table(table, path, write_page_index=page_index, row_group_size=1000)


def test_file_without_page_index_has_no_index_bytes(tmp_path):
    f = tmp_path / "a.parquet"
    _write(f, page_index=False)
    s = parquet_file_sizes(f)
    assert s["page_index_bytes"] == 0 and s["bloom_filter_bytes"] == 0
    assert s["footer_bytes"] > 0
    assert s["total_bytes"] == f.stat().st_size


def test_page_index_bytes_are_everything_but_data_footer_and_magic(tmp_path):
    with_idx, without = tmp_path / "a.parquet", tmp_path / "b.parquet"
    _write(with_idx, page_index=True)
    _write(without, page_index=False)
    s = parquet_file_sizes(with_idx)
    assert s["page_index_bytes"] > 0
    assert parquet_file_sizes(without)["page_index_bytes"] == 0
    # Every byte is accounted for: magic x2, footer length, footer, data,
    # Bloom filters, page indexes.
    assert s["total_bytes"] == (8 + 4 + s["footer_bytes"] + s["data_bytes"]
                                + s["bloom_filter_bytes"] + s["page_index_bytes"])


def test_package_no_index_excludes_bloom_and_page_index_only(tmp_path):
    pkg = tmp_path / "pkg.parquet"
    pkg.mkdir()
    _write(pkg / "building.parquet", page_index=True)
    (pkg / "metadata.json").write_text(json.dumps({"a": 1}))
    report = package_sizes(pkg)
    total = sum(p.stat().st_size for p in pkg.iterdir())
    assert report.size_bytes == total
    detail = report.detail
    assert report.size_bytes_no_index == (
        total - detail["page_index_bytes"] - detail["bloom_filter_bytes"]
    )
    assert detail["footer_bytes"] > 0
