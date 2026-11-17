# cityparquet (Python)

Python bindings for [`cityparquet`](../cityparquet-rs), the CityParquet
reference implementation: the CLI's `convert`, `export` and `validate`, each a
function over paths. A package is plain Parquet, so reading one needs no
bindings; pyarrow, DuckDB and GeoPandas read it as it is.

## Install

```bash
pip install cityparquet
```

PyPI carries wheels for Linux (x86_64, aarch64), macOS (arm64, x86_64) and
Windows (x86_64), each serving CPython 3.11 and later. Elsewhere pip builds the
sdist, which needs Rust. To build from this repository, with Rust and uv:

```bash
cd lib/cityparquet-py
uv sync --extra dev
```

## Use

```python
import cityparquet

cityparquet.convert(["delft.city.jsonl"], "delft/", row_group_size=2048)
cityparquet.validate("delft/")  # [] when the package conforms
cityparquet.export("delft/", "back.city.jsonl")
```

`convert` takes files, directories or globs and merges several into one
package. `overwrite`, `row_group_size` and `datetime` are the CLI's
`--overwrite`, `--row-group-size` and `--datetime`. `export` writes CityJSONSeq,
CityJSON or CityGML 2.0, by the output's extension. A failure raises
`cityparquet.CityParquetError` with the library's message.

## Test

```bash
just py-test   # from the repository root
```

## Release

The **Publish Python** workflow (`.github/workflows/publish-python.yml`), run
by hand from `main` with a `patch` / `minor` / `major` bump (or `none` for the
version already in `Cargo.toml`), sets the version, builds and tests every
wheel and the sdist, uploads them to PyPI and tags `py-vX.Y.Z`. Its dry run, the
default, uploads nothing.
