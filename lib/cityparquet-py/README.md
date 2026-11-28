# cityparquet (Python)

Python bindings for the [CityParquet](https://cityjson.github.io/cityparquet/)
reference implementation. They convert 3D city models (CityJSON, CityJSONSeq,
CityGML) into CityParquet packages, validate packages against the
specification, and export them again.

A package is plain Parquet, so **reading one needs no bindings**: pyarrow,
pandas, DuckDB and GeoPandas open it as it is, locally or over HTTPS. The
bindings are for the part those tools cannot do, which is writing a package
that conforms.

| Function                              | Does                                                                |
| ------------------------------------- | ------------------------------------------------------------------- |
| `cityparquet.convert(inputs, output)` | CityJSON, CityJSONSeq or CityGML 2.0 / 3.0 → a package              |
| `cityparquet.validate(package)`       | The package's violations of the specification, as a list of strings |
| `cityparquet.export(package, output)` | A package → CityJSONSeq, CityJSON or CityGML 2.0                    |
| `cityparquet.CityParquetError`        | Raised by all three, with the library's message                     |

## Install

```bash
pip install cityparquet
```

PyPI carries wheels for Linux (x86_64, aarch64), macOS (arm64, x86_64) and
Windows (x86_64), each serving CPython 3.11 and later, and the same five for
free-threaded CPython 3.14t. Elsewhere pip builds the sdist, which needs
[Rust](https://rustup.rs/).

The bindings have no Python dependencies. The reading examples below use the
tools you choose to read with:

```bash
pip install pyarrow pandas duckdb geopandas
```

To install the development version from the repository instead (this compiles
the library, so it needs Rust and takes a few minutes):

```bash
pip install "git+https://github.com/cityjson/cityparquet.git#subdirectory=lib/cityparquet-py"
```

## Convert, validate, export

Download a small CityJSONSeq file to try it on, 1,115 buildings in Delft from
[3DBAG](https://3dbag.nl):

```bash
curl -fLO https://cityjson.open3d.city/cityjsonseq/delft.city.jsonl
```

```python
import cityparquet

cityparquet.convert(["delft.city.jsonl"], "delft")
print(cityparquet.validate("delft"))  # [] when the package conforms
cityparquet.export("delft", "delft_export.city.jsonl")
```

`delft/` now holds `building.parquet` and `metadata.json`, the
[package layout](https://cityjson.github.io/cityparquet/specification/dataset-package)
the specification defines.

### `convert(inputs, output, *, overwrite=False, row_group_size=None, datetime=None)`

- `inputs` is a **list** of paths, also for one file. Each entry is a file, a
  directory (its `.json`, `.jsonl` and `.gml` files, not recursive) or a glob
  such as `"tiles/*.city.jsonl"`. Several inputs are merged into one package.
  The format is detected from the content: CityJSON, CityJSONSeq, or CityGML
  2.0 or 3.0.
- `output` is the package directory. An existing, non-empty directory is
  refused unless `overwrite=True`.
- `row_group_size` is the number of rows per Parquet row group (the library's
  default of 65,536 when `None`). Smaller groups prune more precisely on a spatial
  filter, at some cost in file size.
- `datetime` is the STAC `datetime` written to `metadata.json`, as an RFC 3339
  string such as `"2026-01-01T00:00:00Z"`. Without it the source's reference
  date is used, else the conversion time.

### `validate(package)`

Returns the package's violations of the specification, one string each, and an
empty list when it conforms.

### `export(package, output)`

The output's extension selects the format: `.city.jsonl` writes CityJSONSeq,
`.city.json` one CityJSON document, `.gml` CityGML 2.0.

### Errors

A failure raises `cityparquet.CityParquetError` with the library's own message:

```python
try:
    cityparquet.convert(["delft.city.jsonl"], "delft")
except cityparquet.CityParquetError as error:
    print(error)
# schema error: output directory delft already exists and is not empty (pass overwrite to replace it)
```

All three functions take `str` or `pathlib.Path`, work on local paths only, and
release the GIL while they run.

## Read a package

Nothing here uses the bindings. An object table is one row per city object,
with its attributes as ordinary columns and each level of detail in its own
`geometry_lod*` column.

**pyarrow and pandas**, reading only the columns you name:

```python
import pyarrow.parquet as pq

table = pq.read_table(
    "delft/building.parquet",
    columns=["id", "object_type", "b3_h_dak_max", "b3_h_maaiveld"],
)
frame = table.to_pandas()
```

**DuckDB**, locally or straight from object storage. This counts the city
objects of a Tokyo ward without downloading its file:

```python
import duckdb

URL = "https://cityparquet.open3d.city/data/plateau/chuo-ku/building.parquet"
print(duckdb.sql(f"""
    SELECT object_type, count(*) AS objects
    FROM read_parquet('{URL}')
    GROUP BY object_type ORDER BY objects DESC
""").df())
#             object_type  objects
# 0              Building    37025
# 1  BuildingInstallation    14579
```

`.df()` gives a pandas DataFrame and `.to_arrow_table()` an Arrow table.

**GeoPandas**, for the LoD0 footprints, which are conformant GeoParquet:

```python
import geopandas as gpd

footprints = gpd.read_parquet("delft/building.parquet")
print(footprints.geometry.name, footprints.crs.to_string())
# geometry_lod0_0 EPSG:7415
```

Solids at LoD1 and above are WKB `PolyhedralSurface Z`, which Shapely and
GeoPandas do not decode. Measure them in DuckDB with the
[`three_d` extension](https://cityjson.github.io/duckdb-3d/#/), which runs from
Python like any other DuckDB SQL.

## What the bindings do not cover

The bindings are three functions with the options listed above. The
[`cityparquet` command-line tool](https://cityjson.github.io/cityparquet/tutorials/cli-conversion)
does more, and until the bindings catch up these need the CLI:

- **FlatCityBuf input.** The published wheels are built without the FlatCityBuf
  reader, so `convert` refuses a `.fcb` file and says so.
- **Conversion options**: declaring a missing CRS (`--crs`), synthesising LoD0
  footprints (`--lod0`), partitioning into several packages, compression and
  row-ordering choices, and converting from a 3DCityDB database.
- **CityGML 3.0 export** (`--citygml-version 3.0`). The bindings write 2.0.
- **Reports**: the CLI prints what a conversion or export left out or dropped;
  the bindings return `None`.
- **`compare`**, the semantic comparison of two models that checks a round
  trip, and **`collection`**, the STAC Collection over several packages.

There is no Python reader object either: reading is the job of the Parquet
tools above.

## Develop

With Rust and [uv](https://docs.astral.sh/uv/), from a clone of the repository:

```bash
cd lib/cityparquet-py
uv sync --extra dev     # builds the extension into .venv
just py-test            # the test suite; run from the repository root
```

## Release

The **Publish Python** workflow (`.github/workflows/publish-python.yml`), run
by hand from `main` with a `patch` / `minor` / `major` bump (or `none` for the
version already in `Cargo.toml`), sets the version, builds and tests every
wheel and the sdist, uploads them to PyPI and tags `py-vX.Y.Z`. Its dry run, the
default, uploads nothing.
