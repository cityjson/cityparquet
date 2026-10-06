import csv
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parents[1]))
import measure_sizes


def write_bytes(path: Path, count: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"x" * count)


def prepare_all(prepared: Path, dataset: str) -> dict[str, int]:
    """One artefact per format; the CityParquet package is a directory."""
    sizes = {}
    for index, (fmt, suffix) in enumerate(measure_sizes.FORMATS.items(), start=1):
        if fmt == "cityparquet":
            package = prepared / f"{dataset}{suffix}"
            write_bytes(package / "building.parquet", 700)
            write_bytes(package / "textures.parquet", 50)
            write_bytes(package / "metadata.json", 7)
            sizes[fmt] = 757
        else:
            write_bytes(prepared / f"{dataset}{suffix}", index * 1000)
            sizes[fmt] = index * 1000
    return sizes


def read_csv(path: Path) -> list[list[str]]:
    with path.open(newline="") as stream:
        return list(csv.reader(stream))


class ArtefactBytesTest(unittest.TestCase):
    def test_a_package_directory_sums_every_file_beneath_it(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            package = Path(tmp) / "city.parquet"
            write_bytes(package / "building.parquet", 1000)
            write_bytes(package / "materials.parquet", 20)
            write_bytes(package / "metadata.json", 3)
            write_bytes(package / "nested" / "part.parquet", 400)
            self.assertEqual(measure_sizes.artefact_bytes(package), 1423)

    def test_a_single_file_is_its_own_size(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "city.fcb"
            write_bytes(path, 123)
            self.assertEqual(measure_sizes.artefact_bytes(path), 123)

    def test_a_missing_artefact_raises_instead_of_counting_zero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(FileNotFoundError):
                measure_sizes.artefact_bytes(Path(tmp) / "absent.parquet")


class MeasureTest(unittest.TestCase):
    def test_every_format_is_measured_under_the_given_dataset_name(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            prepared = Path(tmp)
            expected = prepare_all(prepared, "tokyo")
            self.assertEqual(dict(measure_sizes.measure(prepared, "tokyo")), expected)

    def test_a_missing_artefact_fails_loudly_and_names_the_format(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            prepared = Path(tmp)
            prepare_all(prepared, "tokyo")
            (prepared / "tokyo.fcb").unlink()
            with self.assertRaises(SystemExit) as raised:
                measure_sizes.measure(prepared, "tokyo")
            self.assertIn("flatcitybuf", str(raised.exception))


class CsvContractTest(unittest.TestCase):
    def test_header_is_bytes_and_decimal_megabytes_only(self) -> None:
        self.assertEqual(measure_sizes.HEADER, ["dataset", "format", "bytes", "mb_decimal"])

    def test_rows_carry_bytes_and_megabytes_of_one_million_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            prepared = Path(tmp) / "prepared"
            prepare_all(prepared, "tokyo")
            out = Path(tmp) / "out" / "sizes.csv"
            measure_sizes.main(["--dataset", "tokyo", "--prepared", str(prepared), "--out", str(out)])
            rows = read_csv(out)
            self.assertEqual(rows[0], measure_sizes.HEADER)
            self.assertEqual([row[1] for row in rows[1:]], list(measure_sizes.FORMATS))
            parquet = next(row for row in rows[1:] if row[1] == "cityparquet")
            self.assertEqual(parquet, ["tokyo", "cityparquet", "757", "0.000757"])

    def test_a_rerun_replaces_its_dataset_and_keeps_the_others(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            prepared = Path(tmp) / "prepared"
            prepare_all(prepared, "tokyo")
            prepare_all(prepared, "vienna")
            out = Path(tmp) / "sizes.csv"
            for dataset in ("tokyo", "vienna", "tokyo"):
                measure_sizes.main(["--dataset", dataset, "--prepared", str(prepared), "--out", str(out)])
            datasets = [row[0] for row in read_csv(out)[1:]]
            self.assertEqual(datasets.count("tokyo"), len(measure_sizes.FORMATS))
            self.assertEqual(datasets.count("vienna"), len(measure_sizes.FORMATS))

    def test_a_file_with_a_foreign_header_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            prepared = Path(tmp) / "prepared"
            prepare_all(prepared, "tokyo")
            out = Path(tmp) / "sizes.csv"
            out.write_text("dataset,format,bytes,mb,ratio_vs_cityjsonseq\n")
            with self.assertRaises(SystemExit):
                measure_sizes.main(["--dataset", "tokyo", "--prepared", str(prepared), "--out", str(out)])


if __name__ == "__main__":
    unittest.main()
