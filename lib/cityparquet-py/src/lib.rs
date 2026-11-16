//! Python bindings for CityParquet: the CLI's `convert`, `export` and
//! `validate`, each a function over paths. Every call is the library's own;
//! nothing here interprets CityJSON or Parquet.

use std::path::PathBuf;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

use cityparquet::export::{ExportOptions, export as export_package};
use cityparquet::inputs::resolve_inputs;
use cityparquet::merge::merge_sources;
use cityparquet::package::{ConvertOptions, convert_source};
use cityparquet::source::Source;
use cityparquet::validate::validate_package;

create_exception!(
    cityparquet,
    CityParquetError,
    PyException,
    "An error from the CityParquet library, with its own message."
);

fn raise(e: cityparquet::CityParquetError) -> PyErr {
    CityParquetError::new_err(e.to_string())
}

/// Converts `inputs` (CityJSON or CityJSONSeq files, directories or globs)
/// into one package at `output`, as `cityparquet convert` does: several
/// inputs are merged first. `row_group_size` and `datetime` are its
/// `--row-group-size` and `--datetime`.
#[pyfunction]
#[pyo3(signature = (inputs, output, *, overwrite = false, row_group_size = None, datetime = None))]
fn convert(
    py: Python<'_>,
    inputs: Vec<PathBuf>,
    output: PathBuf,
    overwrite: bool,
    row_group_size: Option<usize>,
    datetime: Option<String>,
) -> PyResult<()> {
    py.detach(|| {
        // Never empty: `resolve_inputs` errs when nothing resolves.
        let files = resolve_inputs(&inputs)?.files;
        let first = files[0].clone();
        let mut sources = files
            .iter()
            .map(|p| Source::open(p))
            .collect::<cityparquet::Result<Vec<_>>>()?;
        let source = if sources.len() == 1 {
            sources.remove(0)
        } else {
            merge_sources(sources)?
        };
        let mut opts = ConvertOptions::new(first, output);
        opts.overwrite = overwrite;
        opts.datetime = datetime;
        if let Some(size) = row_group_size {
            opts.recipe.row_group_size = size;
        }
        convert_source(&source, &opts).map(|_| ())
    })
    .map_err(raise)
}

/// Writes the package at `package` to `output`: CityJSONSeq for
/// `.city.jsonl`, CityJSON for `.city.json`, CityGML 2.0 for `.gml`, as
/// `cityparquet export` does.
#[pyfunction]
fn export(py: Python<'_>, package: PathBuf, output: PathBuf) -> PyResult<()> {
    py.detach(|| export_package(&ExportOptions::new(package, output)).map(|_| ()))
        .map_err(raise)
}

/// The package's violations of the specification, each as
/// `cityparquet validate` prints it; empty when the package conforms.
#[pyfunction]
fn validate(py: Python<'_>, package: PathBuf) -> PyResult<Vec<String>> {
    let report = py.detach(|| validate_package(&package)).map_err(raise)?;
    Ok(report.violations.iter().map(ToString::to_string).collect())
}

#[pymodule]
#[pyo3(name = "cityparquet")]
fn bindings(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(export, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add("CityParquetError", m.py().get_type::<CityParquetError>())?;
    Ok(())
}
