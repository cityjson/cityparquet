//! Conformance checking of a CityParquet package against the specification
//! (`documents/docs/03-specification/`).
//!
//! Every check is derived from a requirement of the specification and cites
//! the page it comes from: a MUST, MUST NOT or REQUIRED is a
//! [`Severity::Error`], a SHOULD or SHOULD NOT a [`Severity::Warning`].
//!
//! Conformance is at the level of **Parquet logical types** (spec 02
//! "Physical encoding and conformance"). The checks read the Parquet schema
//! and the footer key-value metadata, and decode values without the
//! `ARROW:schema` footer entry; they assume nothing about a writer's
//! physical choices — row-group sizing, encodings, compression, row order,
//! bloom filters, or the child names inside a `LIST`/`MAP`. Where the spec
//! states a value-level rule (a required column non-null on every row), the
//! values are checked and a merely nullable declaration is a warning, since a
//! writer may declare a column `OPTIONAL` and never write a null into it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::{Float64Type, Int32Type, Int64Type};
use arrow_array::{Array, ArrayRef, BinaryArray, ListArray, MapArray, RecordBatch, StructArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::arrow_reader::{ArrowReaderOptions, ParquetRecordBatchReaderBuilder};
use parquet::basic::{ConvertedType, LogicalType, Repetition, TimeUnit, Type as PhysicalType};
use parquet::file::metadata::ParquetMetaData;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::schema::types::Type;
use serde_json::{Map, Value};

use cityparquet_schema::metadata::CITYPARQUET_VERSION;
use cityparquet_schema::model::{
    geometry_properties_data_type, material_data_type, texture_data_type,
};
use cityparquet_schema::sidecar_schemas::{materials_schema, textures_schema};
use cityparquet_schema::types::{Lod, ModuleKey, TAXONOMY, module_file};
use cityparquet_schema::{CityParquetError, CityParquetSchema, Result};

use crate::wkb_read::{WkbVisitor, read_point, visit_wkb};

mod footer;
mod layout;
mod stac;
mod tables;
mod types;
mod values;
mod wkb;

use footer::*;
use layout::*;
use stac::*;
use tables::*;
use types::*;
use values::*;
use wkb::*;

// The specification pages a violation cites.
const PACKAGE: &str = "01-dataset-package";
const OBJECT_TABLE: &str = "02-object-table-schema";
const GEOMETRY: &str = "03-geometry-semantics";
const APPEARANCE: &str = "04-appearance-templates";
const METADATA: &str = "05-metadata";
const EXTENSIONS: &str = "06-extensions";

/// How many violations of one code in one file are listed before the rest
/// are summarised as a count.
const MAX_EXAMPLES: usize = 5;

const MATERIALS: &str = "materials.parquet";
const TEXTURES: &str = "textures.parquet";
const IMPLICIT_GEOMETRIES: &str = "implicit_geometries.parquet";

// ISO WKB Z type codes (spec 03 "WKB encoding").
const MULTIPOINT_Z: u32 = 1004;
const MULTIPOLYGON_Z: u32 = 1006;
const GEOMETRYCOLLECTION_Z: u32 = 1007;
const POLYHEDRALSURFACE_Z: u32 = 1015;

/// Whether a violated requirement is a MUST (error) or a SHOULD (warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}

/// One violated requirement.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub severity: Severity,
    /// A stable identifier of the rule, e.g. `footer.version-missing`.
    pub code: &'static str,
    /// The specification page the rule is stated on, e.g. `05-metadata`.
    pub spec: &'static str,
    /// The package file the violation is in, when it is in one.
    pub file: Option<String>,
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.severity, self.code)?;
        if let Some(file) = &self.file {
            write!(f, " {file}:")?;
        }
        write!(f, " {} (spec {})", self.message, self.spec)
    }
}

/// The outcome of [`validate_package`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationReport {
    pub violations: Vec<Violation>,
}

impl ValidationReport {
    pub fn errors(&self) -> impl Iterator<Item = &Violation> {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Violation> {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Warning)
    }

    /// No requirement of the specification is violated (warnings allowed).
    pub fn is_conformant(&self) -> bool {
        self.errors().next().is_none()
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for v in &self.violations {
            writeln!(f, "{v}")?;
        }
        write!(
            f,
            "{} error(s), {} warning(s)",
            self.errors().count(),
            self.warnings().count()
        )
    }
}

/// Collects violations, listing at most [`MAX_EXAMPLES`] of one code per
/// file and counting the rest.
#[derive(Default)]
struct Reporter {
    violations: Vec<Violation>,
    counts: BTreeMap<(Option<String>, &'static str), (Severity, &'static str, usize)>,
}

impl Reporter {
    fn push(
        &mut self,
        severity: Severity,
        code: &'static str,
        spec: &'static str,
        file: Option<&str>,
        message: String,
    ) {
        let entry = self
            .counts
            .entry((file.map(str::to_string), code))
            .or_insert((severity, spec, 0));
        entry.2 += 1;
        if entry.2 <= MAX_EXAMPLES {
            self.violations.push(Violation {
                severity,
                code,
                spec,
                file: file.map(str::to_string),
                message,
            });
        }
    }

    fn error(&mut self, code: &'static str, spec: &'static str, file: Option<&str>, msg: String) {
        self.push(Severity::Error, code, spec, file, msg);
    }

    fn warn(&mut self, code: &'static str, spec: &'static str, file: Option<&str>, msg: String) {
        self.push(Severity::Warning, code, spec, file, msg);
    }

    fn finish(mut self) -> ValidationReport {
        for ((file, code), (severity, spec, n)) in self.counts {
            if n > MAX_EXAMPLES {
                self.violations.push(Violation {
                    severity,
                    code,
                    spec,
                    file,
                    message: format!("{} more `{code}` violation(s) not listed", n - MAX_EXAMPLES),
                });
            }
        }
        ValidationReport {
            violations: self.violations,
        }
    }
}

/// What a `.parquet` file of the package is (spec 01 "File roles").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    ObjectTable,
    Materials,
    Textures,
    ImplicitGeometries,
}

impl FileKind {
    fn of(name: &str) -> Self {
        match name {
            MATERIALS => FileKind::Materials,
            TEXTURES => FileKind::Textures,
            IMPLICIT_GEOMETRIES => FileKind::ImplicitGeometries,
            _ => FileKind::ObjectTable,
        }
    }
}

/// The `city.crs` key's tri-state (spec 05 "CRS rules"), plus a value that
/// is none of the three.
#[derive(Debug, Clone, PartialEq)]
enum CrsKey {
    Absent,
    Null,
    Projjson(Value),
    Invalid,
}

/// The parts of a `city` footer object the checks consult.
struct CityFooter {
    raw: Map<String, Value>,
    crs: CrsKey,
    primary_column: Option<String>,
    columns: Vec<Value>,
    attributes: Vec<String>,
    extensions: BTreeMap<String, Value>,
}

/// One readable `.parquet` file of the package.
struct PackageFile {
    name: String,
    kind: FileKind,
    path: std::path::PathBuf,
    metadata: Arc<ParquetMetaData>,
    city: Option<CityFooter>,
}

impl PackageFile {
    fn key_value(&self, key: &str) -> Option<&str> {
        self.metadata
            .file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.as_deref())
    }

    fn root_fields(&self) -> &[Arc<Type>] {
        self.metadata
            .file_metadata()
            .schema_descr()
            .root_schema()
            .get_fields()
    }

    fn declared_namespaces(&self) -> BTreeSet<&str> {
        self.city
            .as_ref()
            .map(|c| c.extensions.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }
}

/// The `id` values of the sidecars, for resolving references (spec 04
/// "Invariants": every non-null `id` MUST match an `id` in the sidecar).
#[derive(Default)]
struct SidecarIds {
    materials: HashSet<i64>,
    textures: HashSet<i64>,
    implicit_geometries: HashSet<i64>,
}

/// One object row's place in the feature hierarchy, for the `feature_id`
/// rule, which spans files.
struct HierarchyRow {
    first_parent: Option<String>,
    feature_id: Option<String>,
    file: String,
}

/// Check the CityParquet package in `dir` against the specification.
///
/// Errs only when `dir` is not a readable directory; everything the package
/// itself gets wrong is a [`Violation`] in the report.
pub fn validate_package(dir: &Path) -> Result<ValidationReport> {
    if !dir.is_dir() {
        return Err(CityParquetError::io(format!(
            "{} is not a directory",
            dir.display()
        )));
    }
    let mut r = Reporter::default();

    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(|e| CityParquetError::io_source(format!("cannot list {}", dir.display()), e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".parquet"))
        .collect();
    names.sort();

    let mut files = Vec::new();
    for name in &names {
        let path = dir.join(name);
        let metadata = match fs::File::open(&path)
            .map_err(|e| e.to_string())
            .and_then(|f| SerializedFileReader::new(f).map_err(|e| e.to_string()))
        {
            Ok(reader) => Arc::new(reader.metadata().clone()),
            Err(e) => {
                // spec 01 "Directory layout": a package is a directory of
                // Parquet files.
                r.error(
                    "package.unreadable-parquet",
                    PACKAGE,
                    Some(name),
                    format!("not a readable Parquet file: {e}"),
                );
                continue;
            }
        };
        let mut file = PackageFile {
            name: name.clone(),
            kind: FileKind::of(name),
            path,
            metadata,
            city: None,
        };
        file.city = parse_city(&file, &mut r);
        files.push(file);
    }

    check_layout(&files, &mut r);
    check_extension_consistency(&files, &mut r);
    check_stac(dir, &names, &mut r);

    // The relative geometries reference materials and textures, so those
    // two sidecars' ids are collected first.
    let mut ids = SidecarIds::default();
    for kind in [
        FileKind::Materials,
        FileKind::Textures,
        FileKind::ImplicitGeometries,
    ] {
        for file in files.iter().filter(|f| f.kind == kind) {
            check_sidecar(file, &mut ids, &mut r);
        }
    }
    let mut hierarchy: HashMap<String, HierarchyRow> = HashMap::new();
    for file in files.iter().filter(|f| f.kind == FileKind::ObjectTable) {
        check_object_table(file, &ids, &mut hierarchy, &mut r);
    }
    check_feature_ids(&hierarchy, &mut r);

    Ok(r.finish())
}
