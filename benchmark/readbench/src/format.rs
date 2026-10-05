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
/// models actually ship as today (CityGML → CityJSON → CityJSONSeq), then
/// the indexed/columnar ones (FlatCityBuf → CityParquet → Hilbert-ordered
/// CityParquet). See [`Format::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// CityGML 2.0 XML — the format most national datasets are published in.
    CityGml,
    /// Plain (non-sequential) CityJSON: one JSON document for the whole
    /// dataset.
    CityJson,
    /// CityJSONSeq: one JSON object per line.
    CityJsonSeq,
    /// FlatCityBuf: the indexed FlatBuffers encoding.
    FlatCityBuf,
    /// A CityParquet package in source order.
    CityParquet,
    /// A CityParquet package written in Hilbert-curve order. Read by the
    /// SAME runner as [`Format::CityParquet`] (a Hilbert-ordered package is
    /// still a plain CityParquet package on disk); only the artefact path
    /// differs — see [`Format::artefact`].
    CityParquetHilbert,
}

impl Format {
    /// Every variant, in the benchmark's canonical order: the formats data
    /// ships as, then the indexed/columnar ones — so a chart reads
    /// left-to-right from "what you have" to "what we propose".
    pub const ALL: [Format; 6] = [
        Format::CityGml,
        Format::CityJson,
        Format::CityJsonSeq,
        Format::FlatCityBuf,
        Format::CityParquet,
        Format::CityParquetHilbert,
    ];

    /// The FORMAT-COMPARISON set: what a run with no `--formats` measures.
    ///
    /// One tag per format family, so the CSV answers exactly one question —
    /// *how do the formats a city model can ship as compare?* CityParquet is
    /// represented by [`Format::CityParquetHilbert`], the configuration we
    /// would actually ship, so the comparison is not handicapped by an
    /// ordering choice no other format here faces; the ordering choice itself
    /// is a separate question, asked by [`Format::ORDERING_SET`].
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
    /// set rather than extra members of [`Format::DEFAULT_SET`].
    pub const ORDERING_SET: [Format; 2] = [Format::CityParquet, Format::CityParquetHilbert];

    /// The canonical kebab-case CLI/CSV spelling (round-trips through
    /// [`FromStr`]).
    /// The format's runner counts FEATURES (a top-level object with its
    /// descendants) for `count`, `full-read` and `bbox-query`; the others
    /// count CityObjects (READ_BENCHMARK.md, Caveat 1).
    pub fn counts_features(self) -> bool {
        matches!(
            self,
            Format::CityGml | Format::CityJsonSeq | Format::FlatCityBuf
        )
    }

    /// The artefact stores coordinates as GeoParquet does — `x` longitude,
    /// `y` latitude, whatever the CRS declares. Every other artefact keeps
    /// the source's own axis order, so a latitude-first dataset's query
    /// window reaches it with `x` and `y` swapped.
    pub fn stores_longitude_first(self) -> bool {
        matches!(self, Format::CityParquet | Format::CityParquetHilbert)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Format::CityGml => "citygml",
            Format::CityJson => "cityjson",
            Format::CityJsonSeq => "cityjsonseq",
            Format::FlatCityBuf => "flatcitybuf",
            Format::CityParquet => "cityparquet",
            Format::CityParquetHilbert => "cityparquet-hilbert",
        }
    }

    /// The file or directory, relative to the coordinator's `prepared_dir`,
    /// that holds this format's artefact.
    ///
    /// EVERY measured format reads an artefact
    /// `benchmark/scripts/readbench_prepare.sh` built inside `prepared_dir` —
    /// no format reads the original `--input`. These names are the
    /// coordinator's HALF of a contract with that script, which writes
    /// exactly them; `scripts/tests/readbench_prepare_test.sh` reads both
    /// sides out of their own sources and fails if they disagree.
    pub fn artefact(self, base: &str) -> String {
        match self {
            Format::CityGml => format!("{base}.gml"),
            Format::CityJson => format!("{base}.city.json"),
            // NEVER the `--input` itself: a `.gml`/`.city.json` input would
            // then be measured, and published, as CityJSONSeq.
            Format::CityJsonSeq => format!("{base}.city.jsonl"),
            Format::FlatCityBuf => format!("{base}.fcb"),
            Format::CityParquet => format!("{base}.parquet"),
            Format::CityParquetHilbert => format!("{base}-hilbert.parquet"),
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
            "flatcitybuf" => Ok(Format::FlatCityBuf),
            "cityparquet" => Ok(Format::CityParquet),
            "cityparquet-hilbert" => Ok(Format::CityParquetHilbert),
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
        assert_eq!(Format::CityParquet.artefact("delft"), "delft.parquet");
        assert_eq!(
            Format::CityParquetHilbert.artefact("delft"),
            "delft-hilbert.parquet"
        );
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
            "plateau_chuo_fld.city.jsonl"
        );
    }
}
