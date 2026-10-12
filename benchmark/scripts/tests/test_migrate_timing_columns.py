"""migrate_timing_columns.py: recompute the timing block from raw samples."""

import csv
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import migrate_timing_columns as m  # noqa: E402

LEGACY = ("dataset,format,scenario,selectivity,result_count,time_s,time_std_s,peak_heap_bytes,"
          "peak_rss_bytes,repeat,notes,bytes_read,http_requests,row_groups_total,bloom_pruned,filter_bytes")


def sample(i, t, tag=""):
    return {"dataset": "d", "format": "cityparquet", "scenario": "bbox-query", "query_tag": tag,
            "sample_index": i, "warmup": i == 0, "time_s": t, "peak_rss_bytes": 1,
            "peak_heap_bytes": 1, "result_count": 3}


class Readbench(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.csv = self.dir / "d.csv"
        times = [9.0, 1.0, 2.0, 3.0, 4.0]  # first is the warm-up
        (self.dir / "d.csv.samples.json").write_text(
            json.dumps([sample(i, t, "bbox-5pct") for i, t in enumerate(times)]))
        self.row = "d,cityparquet,bbox-query,0.5,3,{mean},{std},1,1,4,bbox-5pct;x,,,,,"

    def write(self, mean="2.500000", std="1.118034"):
        self.csv.write_text(LEGACY + "\n" + self.row.format(mean=mean, std=std) + "\n")

    def test_migrate_writes_the_block_and_verify_accepts_it(self):
        self.write()
        self.assertEqual(m.migrate_file(self.csv), 1)
        row = next(csv.DictReader(self.csv.open()))
        self.assertEqual([row[c] for c in m.TIME_BLOCK],
                         ["2.500000", "1.118034", "2.500000", "1.000000", "4.000000",
                          "1.750000", "3.250000"])
        self.assertEqual(row["notes"], "bbox-5pct;x")
        self.assertNotIn("time_s", row)
        self.assertEqual(m.verify_file(self.csv), 1)

    def test_a_mean_that_does_not_reproduce_fails_naming_file_and_row(self):
        self.write(mean="2.500001")
        with self.assertRaisesRegex(m.MigrationError, r"d\.csv: row 2: .*time_mean_s"):
            m.migrate_file(self.csv)

    def test_missing_sidecar_fails(self):
        self.write()
        (self.dir / "d.csv.samples.json").unlink()
        with self.assertRaisesRegex(m.MigrationError, "samples sidecar"):
            m.migrate_file(self.csv)

    def test_verify_rejects_a_tampered_cell(self):
        self.write()
        m.migrate_file(self.csv)
        self.csv.write_text(self.csv.read_text().replace("3.250000", "3.250001"))
        with self.assertRaisesRegex(m.MigrationError, "row 2: time_q3_s"):
            m.verify_file(self.csv)

    def test_verify_accepts_a_row_the_cell_budget_cut_short(self):
        # A budgeted run with a ceiling of 25 stopped this cell after 4 warm
        # samples: `repeat` records the samples taken and `notes` ends in
        # `budget`, so the sidecar's warm-sample count still matches it.
        (self.dir / "d.csv.samples.json").write_text(json.dumps([
            {**sample(i, t, "bbox-5pct"), "cell_budget_s": 30.0, "min_repeat": 3}
            for i, t in enumerate([9.0, 1.0, 2.0, 3.0, 4.0])
        ]))
        self.row = "d,cityparquet,bbox-query,0.5,3,{mean},{std},1,1,4,bbox-5pct;budget,,,,,"
        self.write()
        self.assertEqual(m.migrate_file(self.csv), 1)
        self.assertEqual(m.verify_file(self.csv), 1)


class Databases(unittest.TestCase):
    def test_both_blocks_recomputed_with_crlf_kept(self):
        d = Path(tempfile.mkdtemp())
        path = d / "x.csv"
        header = ["dataset", "time_s", "time_std_s", "server_time_s", "status",
                  "raw_time_samples_s", "raw_server_time_samples_s"]
        with path.open("w", newline="") as fh:
            w = csv.writer(fh)
            w.writerow(header)
            w.writerow(["d", "2.000000", "0.816497", "1.500000", "ok", "[1.0,2.0,3.0]", "[1.0,2.0]"])
            w.writerow(["d", "", "", "", "error", "[]", "[]"])
        m.migrate_file(path)
        self.assertIn("\r\n", path.open(newline="").read())
        rows = list(csv.DictReader(path.open(newline="")))
        self.assertEqual(rows[0]["time_median_s"], "2.000000")
        self.assertEqual(rows[0]["server_time_q3_s"], "1.750000")
        self.assertEqual(rows[1]["server_time_mean_s"], "")
        self.assertEqual(list(rows[0])[1:15], m.TIME_BLOCK + m.SERVER_BLOCK)
        self.assertEqual(m.verify_file(path), 2)


if __name__ == "__main__":
    unittest.main()
