//! The [`Format`] enum — the vocabulary the whole read-benchmark harness
//! shares.
//!
//! A format is named in four places that must agree exactly: the `--child`
//! dispatch (`formats::resolve`), the coordinator's artefact naming
//! (`coordinator::resolve_format_artefact`), the results CSV's `format`
//! column, and the plotter's ordering. It was previously a bare `&str`
//! matched in three unrelated places, plus two hand-maintained doc-comment
//! lists and a hand-maintained test list, with no compiler help at all — so
//! adding a format was six edits and a hope, and a typo in `--formats` was
//! silently skipped rather than rejected.
//!
//! This deliberately mirrors its sibling `Scenario` (`src/scenario.rs`):
//! [`Format::ALL`] in one canonical order, [`Format::as_str`]
//! as the single spelling authority, a [`Display`](std::fmt::Display) that
//! delegates to it, and a [`FromStr`] whose error enumerates every variant.

use std::str::FromStr;

/// One format the read benchmark measures.
///
/// Variants are ordered as the benchmark presents them: the formats city
/// models actually ship as today (CityGML → CityJSON → CityJSONSeq →
/// gzipped CityJSONSeq), then the indexed/columnar ones (FlatCityBuf →
/// CityParquet → Hilbert-ordered CityParquet), then the SQL-engine
/// baseline. See [`Format::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// CityGML 2.0 XML — the format most national datasets are published in.
    CityGml,
    /// Plain (non-sequential) CityJSON: one JSON document for the whole
    /// dataset.
    CityJson,
    /// CityJSONSeq: one JSON object per line.
    CityJsonSeq,
    /// gzipped CityJSONSeq (`.jsonl.gz`).
    CityJsonSeqGz,
    /// FlatCityBuf: the indexed FlatBuffers encoding.
    FlatCityBuf,
    /// A CityParquet package in source order.
    CityParquet,
    /// A CityParquet package written in Hilbert-curve order. Read by the
    /// SAME runner as [`Format::CityParquet`] (a Hilbert-ordered package is
    /// still a plain CityParquet package on disk); only the artefact path
    /// differs — see [`Format::artefact`].
    CityParquetHilbert,
    /// DuckDB reading the CityParquet package through SQL — a SQL-engine
    /// baseline driven entirely by `scripts/readbench_duckdb.sh`, never by
    /// this binary's `--child` path or its coordinator.
    DuckDbParquet,
}

/// Where a [`Format`]'s artefact lives, relative to the coordinator's
/// `prepared_dir`.
///
/// EVERY measured format reads an artefact `scripts/readbench_prepare.sh`
/// built inside `prepared_dir` — no format reads the original `--input`.
/// There used to be a third case, `TheInputItself`, for
/// [`Format::CityJsonSeq`]: it was correct only while `--input` was itself a
/// `.city.jsonl`, and on the catalogue corpus (whose inputs are `.gml` and
/// `.city.json`) it silently made the `cityjsonseq` row measure the input's
/// OWN format under this format's name. The prepare script now always
/// materialises `<base>.city.jsonl`, and this enum no longer has a way to
/// say otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artefact {
    /// A file or directory the prepare script builds inside `prepared_dir`.
    Prepared(String),
    /// Not this coordinator's business at all.
    NotCoordinated,
}

impl Format {
    /// Every variant, in the benchmark's canonical order: the formats data
    /// ships as, then the indexed/columnar ones, then the engine baseline —
    /// so a chart reads left-to-right from "what you have" to "what we
    /// propose".
    pub const ALL: [Format; 8] = [
        Format::CityGml,
        Format::CityJson,
        Format::CityJsonSeq,
        Format::CityJsonSeqGz,
        Format::FlatCityBuf,
        Format::CityParquet,
        Format::CityParquetHilbert,
        Format::DuckDbParquet,
    ];

    /// The FORMAT-COMPARISON set: what a run with no `--formats` measures.
    ///
    /// One tag per format family, so the CSV answers exactly one question —
    /// *how do the formats a city model can ship as compare?* CityParquet is
    /// represented by [`Format::CityParquetHilbert`], the configuration we
    /// would actually ship, so the comparison is not handicapped by an
    /// ordering choice no other format here faces; the ordering choice itself
    /// is a separate question, asked by [`Format::ORDERING_SET`].
    ///
    /// [`Format::CityJsonSeqGz`] (a compression variant of a format already
    /// in the set) and [`Format::DuckDbParquet`] (an SQL-engine baseline, and
    /// not driven by this coordinator at all) are opt-in: neither is a
    /// format, so neither belongs on a format axis.
    pub const DEFAULT_SET: [Format; 5] = [
        Format::CityGml,
        Format::CityJson,
        Format::CityJsonSeq,
        Format::FlatCityBuf,
        Format::CityParquetHilbert,
    ];

    /// The ORDERING-COMPARISON set — the answer to *does Hilbert-curve
    /// ordering pay for itself?*, and nothing else.
    ///
    /// Both members are the same writer, the same reader and the same
    /// scenarios; the ONLY difference is the row order the package was
    /// written in (see [`Format::artefact`]). Running this set alongside
    /// other formats would confound the two axes, which is why it is its own
    /// set rather than extra members of [`Format::DEFAULT_SET`] — the
    /// justfile's `ordering-bench` recipe passes exactly these two tags.
    pub const ORDERING_SET: [Format; 2] = [Format::CityParquet, Format::CityParquetHilbert];

    /// The canonical kebab-case CLI/CSV spelling (round-trips through
    /// [`FromStr`]).
    pub fn as_str(self) -> &'static str {
        match self {
            Format::CityGml => "citygml",
            Format::CityJson => "cityjson",
            Format::CityJsonSeq => "cityjsonseq",
            Format::CityJsonSeqGz => "cityjsonseq-gz",
            Format::FlatCityBuf => "flatcitybuf",
            Format::CityParquet => "cityparquet",
            Format::CityParquetHilbert => "cityparquet-hilbert",
            Format::DuckDbParquet => "duckdb-parquet",
        }
    }

    /// Where this format's artefact lives, relative to `prepared_dir`.
    ///
    /// Two cases:
    /// - `Prepared(name)`: a file or directory the prepare script builds.
    ///   These names are the coordinator's HALF of a contract with
    ///   `scripts/readbench_prepare.sh`, which writes exactly them;
    ///   `scripts/tests/readbench_prepare_test.sh` reads both sides out of
    ///   their own sources and fails if they disagree.
    /// - `NotCoordinated`: `DuckDbParquet` is an SQL-engine baseline driven
    ///   by `scripts/readbench_duckdb.sh`, never by this coordinator.
    pub fn artefact(self, base: &str) -> Artefact {
        match self {
            Format::CityGml => Artefact::Prepared(format!("{base}.gml")),
            Format::CityJson => Artefact::Prepared(format!("{base}.city.json")),
            // NEVER the `--input` itself: a `.gml`/`.city.json` input would
            // then be measured, and published, as CityJSONSeq.
            Format::CityJsonSeq => Artefact::Prepared(format!("{base}.city.jsonl")),
            Format::CityJsonSeqGz => Artefact::Prepared(format!("{base}.jsonl.gz")),
            Format::FlatCityBuf => Artefact::Prepared(format!("{base}.fcb")),
            Format::CityParquet => Artefact::Prepared(format!("{base}.parquet")),
            Format::CityParquetHilbert => Artefact::Prepared(format!("{base}-hilbert.parquet")),
            Format::DuckDbParquet => Artefact::NotCoordinated,
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Format {
    type Err = String;

    /// Accepts the canonical kebab-case spelling case-insensitively. The
    /// error lists every valid name, so this type — not a hand-maintained
    /// string in `formats::resolve` — is the single place that enumerates
    /// them.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "citygml" => Ok(Format::CityGml),
            "cityjson" => Ok(Format::CityJson),
            "cityjsonseq" => Ok(Format::CityJsonSeq),
            "cityjsonseq-gz" => Ok(Format::CityJsonSeqGz),
            "flatcitybuf" => Ok(Format::FlatCityBuf),
            "cityparquet" => Ok(Format::CityParquet),
            "cityparquet-hilbert" => Ok(Format::CityParquetHilbert),
            "duckdb-parquet" => Ok(Format::DuckDbParquet),
            other => Err(format!(
                "unknown format '{other}'; expected one of: {}",
                Format::ALL
                    .iter()
                    .map(|f| f.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_is_case_insensitive() {
        assert_eq!(
            "CityParquet-Hilbert".parse::<Format>().unwrap(),
            Format::CityParquetHilbert
        );
    }

    /// The two CityParquet variants share a runner but never a path: the
    /// only difference between them IS which artefact resolves.
    #[test]
    fn the_two_cityparquet_orderings_resolve_to_different_artefacts() {
        assert_eq!(
            Format::CityParquet.artefact("delft"),
            Artefact::Prepared("delft.parquet".to_string())
        );
        assert_eq!(
            Format::CityParquetHilbert.artefact("delft"),
            Artefact::Prepared("delft-hilbert.parquet".to_string())
        );
    }

    /// `duckdb-parquet` is the ONLY format with no artefact of this
    /// coordinator's own (see [`Artefact`]'s own doc comment).
    #[test]
    fn only_the_sql_engine_baseline_is_uncoordinated() {
        for format in Format::ALL {
            match format {
                Format::DuckDbParquet => assert_eq!(
                    format.artefact("delft"),
                    Artefact::NotCoordinated,
                    "{format} is driven by scripts/readbench_duckdb.sh"
                ),
                other => assert!(
                    matches!(other.artefact("delft"), Artefact::Prepared(_)),
                    "{other} must read an artefact from --prepared-dir"
                ),
            }
        }
    }

    /// CityJSONSeq reads a PREPARED `<base>.city.jsonl`, never the original
    /// `--input`. While it read the input itself, a `.gml`/`.city.json`
    /// input made the `cityjsonseq` row measure the input's own format —
    /// CityGML parsing, appearance pre-pass included — and publish it as
    /// CityJSONSeq.
    #[test]
    fn cityjsonseq_reads_a_prepared_seq_artefact() {
        assert_eq!(
            Format::CityJsonSeq.artefact("plateau_chuo_fld"),
            Artefact::Prepared("plateau_chuo_fld.city.jsonl".to_string())
        );
    }
}
