//! Which columns of a CityParquet table carry a Bloom filter, and why.
//!
//! A column carries a filter when one of its column chunks records a
//! bloom-filter offset in the Parquet footer. For every text column the
//! non-null and distinct counts are computed EXACTLY, from a single-column
//! projected scan of the data (each non-null value inserted into a hash set),
//! not from the writer's distinct-count estimate, so the listing shows how the
//! writer's rule ([`BLOOM_DISTINCT_RATIO`]) played out on the stored values.

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cityparquet::scan::BLOOM_DISTINCT_RATIO;
use parquet::basic::{LogicalType, Type as PhysicalType};
use parquet::file::reader::{FileReader, SerializedFileReader};

use crate::params::{projected_reader, utf8_values};

/// The writer's threshold: a TEXT attribute gets a filter when its distinct
/// count is at least this share of its non-null count.
pub const WRITER_RATIO: f64 = BLOOM_DISTINCT_RATIO;

/// One top-level column of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSurvey {
    pub name: String,
    /// The Parquet physical type, with the logical type when there is one
    /// (e.g. `BYTE_ARRAY (String)`).
    pub parquet_type: String,
    pub text: bool,
    /// Exact counts, `None` for a non-text column (not scanned).
    pub non_null: Option<u64>,
    pub distinct: Option<u64>,
    /// Summed `bloom_filter_length` over the column's chunks.
    pub filter_bytes: u64,
    /// Column chunks that carry a filter.
    pub chunks_with_filter: usize,
}

impl ColumnSurvey {
    pub fn has_filter(&self) -> bool {
        self.chunks_with_filter > 0
    }

    /// `distinct / non_null`; `None` for a non-text or all-null column.
    pub fn ratio(&self) -> Option<f64> {
        match (self.distinct, self.non_null) {
            (Some(d), Some(n)) if n > 0 => Some(d as f64 / n as f64),
            _ => None,
        }
    }
}

/// The survey of one table.
#[derive(Debug, Clone)]
pub struct BloomSurvey {
    pub table: PathBuf,
    pub rows: u64,
    pub row_groups: usize,
    /// Every top-level column, in the file's order.
    pub columns: Vec<ColumnSurvey>,
}

impl BloomSurvey {
    pub fn with_filter(&self) -> impl Iterator<Item = &ColumnSurvey> {
        self.columns.iter().filter(|c| c.has_filter())
    }

    pub fn text_without_filter(&self) -> impl Iterator<Item = &ColumnSurvey> {
        self.columns.iter().filter(|c| c.text && !c.has_filter())
    }

    pub fn column(&self, name: &str) -> Option<&ColumnSurvey> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// The main table of a package directory (single-table packages only), or
/// `path` itself when it is a Parquet file.
pub fn main_table(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    let tables = cityparquet::stac::properties::PackageTables::open(path)
        .with_context(|| format!("reading the package manifest at {}", path.display()))?;
    match tables.tables.as_slice() {
        [only] => Ok(only.clone()),
        many => bail!(
            "package at {} has {} tables; only single-table packages are supported",
            path.display(),
            many.len()
        ),
    }
}

/// Surveys `table`'s top-level columns (nested leaves such as geometry
/// coordinates are not attribute columns and are skipped).
pub fn survey(table: &Path) -> Result<BloomSurvey> {
    let file = File::open(table).with_context(|| format!("opening {}", table.display()))?;
    let reader = SerializedFileReader::new(file)
        .with_context(|| format!("reading the footer of {}", table.display()))?;
    let meta = reader.metadata();
    let schema = meta.file_metadata().schema_descr();
    let mut columns = Vec::new();
    for (index, leaf) in schema.columns().iter().enumerate() {
        if leaf.path().parts().len() != 1 {
            continue;
        }
        let physical = leaf.physical_type();
        let logical = leaf.logical_type_ref().cloned();
        let text = physical == PhysicalType::BYTE_ARRAY && logical == Some(LogicalType::String);
        let parquet_type = match &logical {
            Some(l) => format!("{physical} ({l:?})"),
            None => physical.to_string(),
        };
        let (mut filter_bytes, mut chunks_with_filter) = (0u64, 0usize);
        for rg in meta.row_groups() {
            let chunk = rg.column(index);
            if chunk.bloom_filter_offset().is_some() {
                chunks_with_filter += 1;
                filter_bytes += chunk.bloom_filter_length().unwrap_or(0).max(0) as u64;
            }
        }
        columns.push(ColumnSurvey {
            name: leaf.name().to_string(),
            parquet_type,
            text,
            non_null: None,
            distinct: None,
            filter_bytes,
            chunks_with_filter,
        });
    }
    for column in columns.iter_mut().filter(|c| c.text) {
        let (non_null, distinct) = exact_counts(table, &column.name)?;
        column.non_null = Some(non_null);
        column.distinct = Some(distinct);
    }
    Ok(BloomSurvey {
        table: table.to_path_buf(),
        rows: meta.file_metadata().num_rows().max(0) as u64,
        row_groups: meta.num_row_groups(),
        columns,
    })
}

fn exact_counts(table: &Path, column: &str) -> Result<(u64, u64)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut non_null = 0u64;
    for batch in projected_reader(table, &[column])? {
        let batch = batch.with_context(|| format!("reading a batch of {}", table.display()))?;
        for cell in utf8_values(batch.column(0).as_ref())?.into_iter().flatten() {
            non_null += 1;
            seen.insert(cell);
        }
    }
    Ok((non_null, seen.len() as u64))
}

/// Fails, naming `column`, unless it exists in `survey`, is a text column and
/// carries a Bloom filter — the precondition of an attribute probe on the
/// bloom axis, which would otherwise measure nothing.
pub fn require_filtered_text(survey: &BloomSurvey, column: &str) -> Result<()> {
    let Some(c) = survey.column(column) else {
        bail!(
            "bloom attribute '{column}' is not a column of {}",
            survey.table.display()
        );
    };
    if !c.text {
        bail!(
            "bloom attribute '{column}' is not a text column ({}) in {}",
            c.parquet_type,
            survey.table.display()
        );
    }
    if !c.has_filter() {
        bail!(
            "bloom attribute '{column}' carries no Bloom filter in {} (distinct/non-null \
             ratio {:.3}, the writer's threshold is {WRITER_RATIO}); choose a column \
             `just bloom-columns` lists as filtered",
            survey.table.display(),
            c.ratio().unwrap_or(0.0)
        );
    }
    Ok(())
}

/// The listing `cityparquet-readbench bloom-columns` prints.
pub fn render(survey: &BloomSurvey) -> String {
    let mut out = format!(
        "table: {}\nrows: {}  row groups: {}\nwriter threshold: distinct >= {WRITER_RATIO} x \
         non-null (TEXT attributes; id and feature_id always)\ncounts: exact, from a \
         single-column projected scan of each text column\n\ncolumns WITH a Bloom filter\n",
        survey.table.display(),
        survey.rows,
        survey.row_groups
    );
    let line = |c: &ColumnSurvey| {
        let opt = |v: Option<u64>| v.map_or("-".to_string(), |v| v.to_string());
        format!(
            "  {:<28} {:<22} non-null {:>9}  distinct {:>9}  ratio {:>6}  filter bytes {:>9}  \
             row groups {}/{}\n",
            c.name,
            c.parquet_type,
            opt(c.non_null),
            opt(c.distinct),
            c.ratio().map_or("-".to_string(), |r| format!("{r:.3}")),
            c.filter_bytes,
            c.chunks_with_filter,
            survey.row_groups
        )
    };
    survey.with_filter().for_each(|c| out.push_str(&line(c)));
    out.push_str("\ntext columns WITHOUT a Bloom filter\n");
    survey
        .text_without_filter()
        .for_each(|c| out.push_str(&line(c)));
    out
}
