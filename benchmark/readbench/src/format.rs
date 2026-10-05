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
/// the indexed/columnar ones (FlatCityBuf → CityParquet). See
/// [`Format::ALL`].
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
    /// A CityParquet package, its rows written in Hilbert-curve order
    /// (`cityparquet convert --ordering hilbert`): the configuration
    /// CityParquet would ship with, and the benchmark's only one.
    CityParquet,
}

impl Format {
    /// Every variant, in the benchmark's canonical order: the formats data
    /// ships as, then the indexed/columnar ones — so a chart reads
    /// left-to-right from "what you have" to "what we propose". It is also
    /// what a run with no `--formats` measures: one tag per format family.
    pub const ALL: [Format; 5] = [
        Format::CityGml,
        Format::CityJson,
        Format::CityJsonSeq,
        Format::FlatCityBuf,
        Format::CityParquet,
    ];

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
        matches!(self, Format::CityParquet)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Format::CityGml => "citygml",
            Format::CityJson => "cityjson",
            Format::CityJsonSeq => "cityjsonseq",
            Format::FlatCityBuf => "flatcitybuf",
            Format::CityParquet => "cityparquet",
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
            "CityParquet".parse::<Format>().unwrap(),
            Format::CityParquet
        );
    }

    /// The one CityParquet package lives at `<base>.parquet`.
    #[test]
    fn cityparquet_reads_the_one_package() {
        assert_eq!(Format::CityParquet.artefact("delft"), "delft.parquet");
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
