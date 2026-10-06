//! Derived rows for the whole-file formats in the `network` family.
//!
//! Over HTTP the three text formats (`citygml`, `cityjson`, `cityjsonseq`)
//! answer every query with ONE whole-object GET, so their bytes read and
//! request count are those of read all whatever the query. The `network`
//! family therefore measures them only on the scenarios its
//! [`WholeFileScenarios`] setting names (by default read all and the
//! identifier lookups) and emits every other scenario as a DERIVED row: bytes
//! and requests copied from the measured read all, every time field empty,
//! `notes` tagged [`DERIVED_TAG`] and [`DERIVED_STATUS`].
//!
//! A derived row is written only after the run has proven the premise on the
//! format's measured cells ([`premise`]): every sample, warm-ups included,
//! issued exactly one request and received exactly the artefact's size. When
//! the premise fails nothing is derived for that format, and the run says so.

use std::str::FromStr;

use crate::formats::IoStats;
use crate::scenario::Scenario;
use cityparquet_readbench::format::Format;

/// The `notes` tag naming where a derived row's transfer comes from.
pub const DERIVED_TAG: &str = "derived-from=full-read";
/// The `notes` tag marking a row as not measured; with the empty timing
/// block it keeps any loader from reading the row as a measurement.
pub const DERIVED_STATUS: &str = "status=derived";

/// Which scenarios the whole-file formats are measured on in a `network`
/// run. Read all is always measured: every derived row copies from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WholeFileScenarios {
    /// Every scenario measured (no derived rows).
    All,
    /// Read all and the identifier lookups measured; the rest derived.
    #[default]
    FullReadAndIdLookup,
    /// Only read all measured; the identifier lookups derived too.
    FullRead,
}

impl WholeFileScenarios {
    pub fn as_str(self) -> &'static str {
        match self {
            WholeFileScenarios::All => "all",
            WholeFileScenarios::FullReadAndIdLookup => "full-read,id-lookup",
            WholeFileScenarios::FullRead => "full-read",
        }
    }

    /// Whether `format` is measured on `scenario` under this setting. The
    /// indexed formats are always measured.
    pub fn measures(self, format: Format, scenario: Scenario) -> bool {
        if !is_whole_file(format) {
            return true;
        }
        match self {
            WholeFileScenarios::All => true,
            WholeFileScenarios::FullReadAndIdLookup => {
                matches!(scenario, Scenario::FullRead | Scenario::IdLookup)
            }
            WholeFileScenarios::FullRead => scenario == Scenario::FullRead,
        }
    }
}

impl FromStr for WholeFileScenarios {
    type Err = String;

    /// `all`, or a comma list of scenarios that must contain `full-read` and
    /// may add `id-lookup` (in any order, case-insensitive).
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim().to_ascii_lowercase();
        if value == "all" {
            return Ok(WholeFileScenarios::All);
        }
        let mut full_read = false;
        let mut id_lookup = false;
        for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part {
                "full-read" => full_read = true,
                "id-lookup" => id_lookup = true,
                other => {
                    return Err(format!(
                        "whole-file scenarios: {other:?} cannot be measured alone; accepted: \
                         all, full-read,id-lookup, full-read"
                    ));
                }
            }
        }
        match (full_read, id_lookup) {
            (true, true) => Ok(WholeFileScenarios::FullReadAndIdLookup),
            (true, false) => Ok(WholeFileScenarios::FullRead),
            _ => Err(format!(
                "whole-file scenarios {value:?} must include full-read (every derived row \
                 copies it); accepted: all, full-read,id-lookup, full-read"
            )),
        }
    }
}

/// The formats whose HTTP client issues one whole-object GET per query.
pub fn is_whole_file(format: Format) -> bool {
    matches!(
        format,
        Format::CityGml | Format::CityJson | Format::CityJsonSeq
    )
}

/// Proves the derivation premise for one format: every sample of its measured
/// cells (`(bytes_read, http_requests)`, warm-ups included) issued exactly one
/// request and received exactly `artefact_size` bytes. Returns the transfer
/// to copy, or why the premise does not hold.
pub fn premise(
    samples: &[(Option<u64>, Option<u64>)],
    artefact_size: Option<u64>,
) -> Result<IoStats, String> {
    let Some(size) = artefact_size else {
        return Err("the artefact's size is unknown (HEAD gave no Content-Length)".to_string());
    };
    if samples.is_empty() {
        return Err("no measured sample to prove it on".to_string());
    }
    for (index, &(bytes, requests)) in samples.iter().enumerate() {
        if requests != Some(1) || bytes != Some(size) {
            return Err(format!(
                "sample {index} read {} bytes in {} request(s); the premise needs {size} bytes \
                 in 1 request",
                bytes.map_or("no".to_string(), |b| b.to_string()),
                requests.map_or("no".to_string(), |r| r.to_string()),
            ));
        }
    }
    Ok(IoStats {
        bytes: size,
        requests: 1,
    })
}

/// One sample's client-side transfer: `(bytes_read, http_requests)`.
pub type Transfer = (Option<u64>, Option<u64>);

/// Each format label's measured samples.
pub type MeasuredTransfers = std::collections::HashMap<String, Vec<Transfer>>;

/// A cell the matrix did not measure, waiting for its format's premise.
#[derive(Debug, Clone)]
pub struct Pending {
    pub dataset: String,
    pub label: String,
    pub scenario: Scenario,
    /// The tag a measured row of this cell would carry (window, probe, ...).
    pub notes: String,
}

impl Pending {
    /// The derived CSV row, in the coordinator's header order: transfer
    /// copied, `result_count`, every time and memory field empty, zero
    /// repeats, `notes` = the cell's own tag, then [`DERIVED_TAG`] and
    /// [`DERIVED_STATUS`].
    pub fn render(&self, io: IoStats) -> String {
        let mut notes: Vec<&str> = Vec::new();
        if !self.notes.is_empty() {
            notes.push(&self.notes);
        }
        notes.push(DERIVED_TAG);
        notes.push(DERIVED_STATUS);
        format!(
            "{},{},{},,,,,,,,,,,,0,{},{},{},,,,",
            self.dataset,
            self.label,
            self.scenario.as_str(),
            notes.join(";"),
            io.bytes,
            io.requests,
        )
    }

    /// The cell's name in the run's diagnostics.
    pub fn cell(&self) -> String {
        if self.notes.is_empty() {
            format!("{}/{}", self.label, self.scenario.as_str())
        } else {
            format!("{}/{}/{}", self.label, self.scenario.as_str(), self.notes)
        }
    }
}

/// What [`resolve`] decided for a run's pending cells.
#[derive(Debug, Default)]
pub struct Resolution {
    /// The derived CSV rows, for the formats whose premise held.
    pub lines: Vec<String>,
    /// The params sidecar's `whole_file_derivation.formats` record.
    pub record: serde_json::Map<String, serde_json::Value>,
    /// One loud line per format whose premise failed.
    pub warnings: Vec<String>,
}

/// Derives every pending cell of each format whose premise holds on its
/// measured samples (`measured[label]`) against its artefact size
/// (`sizes[label]`); a format whose premise fails gets no derived row, a
/// warning and a `premise-failed` record.
pub fn resolve(
    pending: &[Pending],
    measured: &MeasuredTransfers,
    sizes: &std::collections::HashMap<String, Option<u64>>,
) -> Resolution {
    let mut out = Resolution::default();
    let mut labels: Vec<&str> = Vec::new();
    for p in pending {
        if !labels.contains(&p.label.as_str()) {
            labels.push(&p.label);
        }
    }
    for label in labels {
        let cells: Vec<&Pending> = pending.iter().filter(|p| p.label == label).collect();
        let names: Vec<String> = cells.iter().map(|p| p.cell()).collect();
        let samples = measured.get(label).map(Vec::as_slice).unwrap_or(&[]);
        let size = sizes.get(label).copied().flatten();
        let entry = match premise(samples, size) {
            Ok(io) => {
                out.lines.extend(cells.iter().map(|p| p.render(io)));
                serde_json::json!({
                    "status": "derived",
                    "artefact_size": io.bytes,
                    "requests": io.requests,
                    "samples_checked": samples.len(),
                    "cells": names,
                })
            }
            Err(reason) => {
                out.warnings.push(format!(
                    "cityparquet-readbench: WARNING: NOT DERIVED for {label}: the whole-object \
                     premise does not hold ({reason}); its {} unmeasured cell(s) are absent from \
                     the CSV: {}",
                    names.len(),
                    names.join(", ")
                ));
                serde_json::json!({
                    "status": "premise-failed",
                    "reason": reason,
                    "artefact_size": size,
                    "samples_checked": samples.len(),
                    "cells": names,
                })
            }
        };
        out.record.insert(label.to_string(), entry);
    }
    out
}

/// The cross-format consistency check's report on the cells it skipped
/// because they were derived, not measured; `None` when there are none.
pub fn consistency_skips(derived: &[String]) -> Option<String> {
    (!derived.is_empty()).then(|| {
        format!(
            "cityparquet-readbench: cross-format consistency skipped {} derived cell(s) (not \
             measured, no result count): {}",
            derived.len(),
            derived.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_settings_and_rejects_the_rest() {
        let parse = |s: &str| s.parse::<WholeFileScenarios>();
        assert_eq!(parse("all"), Ok(WholeFileScenarios::All));
        assert_eq!(
            parse("full-read,id-lookup"),
            Ok(WholeFileScenarios::FullReadAndIdLookup)
        );
        assert_eq!(
            parse(" ID-LOOKUP , full-read"),
            Ok(WholeFileScenarios::FullReadAndIdLookup)
        );
        assert_eq!(parse("full-read"), Ok(WholeFileScenarios::FullRead));
        assert!(parse("id-lookup").is_err(), "read all must be measured");
        assert!(parse("full-read,count").is_err());
        assert!(parse("").is_err());
        assert_eq!(
            WholeFileScenarios::default().as_str(),
            "full-read,id-lookup"
        );
        for s in [
            WholeFileScenarios::All,
            WholeFileScenarios::FullRead,
            WholeFileScenarios::FullReadAndIdLookup,
        ] {
            assert_eq!(parse(s.as_str()), Ok(s));
        }
    }

    #[test]
    fn indexed_formats_are_always_measured() {
        for scenario in Scenario::ALL {
            for format in [Format::FlatCityBuf, Format::CityParquet] {
                assert!(WholeFileScenarios::FullRead.measures(format, scenario));
            }
        }
        let default = WholeFileScenarios::default();
        assert!(default.measures(Format::CityGml, Scenario::FullRead));
        assert!(default.measures(Format::CityJsonSeq, Scenario::IdLookup));
        assert!(!default.measures(Format::CityJson, Scenario::BBoxQuery));
        assert!(!default.measures(Format::CityGml, Scenario::Count));
        assert!(!WholeFileScenarios::FullRead.measures(Format::CityGml, Scenario::IdLookup));
        assert!(WholeFileScenarios::All.measures(Format::CityGml, Scenario::AttrStats));
    }

    #[test]
    fn premise_holds_on_one_whole_object_request_per_sample() {
        let io = premise(&[(Some(500), Some(1)); 4], Some(500)).unwrap();
        assert_eq!((io.bytes, io.requests), (500, 1));
    }

    #[test]
    fn premise_fails_on_any_other_transfer() {
        let ok = (Some(500), Some(1));
        assert!(
            premise(&[ok, (Some(500), Some(2))], Some(500)).is_err(),
            "two requests"
        );
        assert!(
            premise(&[ok, (Some(499), Some(1))], Some(500)).is_err(),
            "short body"
        );
        assert!(premise(&[ok, (None, None)], Some(500)).is_err(), "no tally");
        assert!(premise(&[ok], None).is_err(), "size unknown");
        assert!(premise(&[], Some(500)).is_err(), "nothing measured");
    }

    #[test]
    fn a_derived_row_copies_the_transfer_and_leaves_every_time_empty() {
        let pending = Pending {
            dataset: "rotterdam".into(),
            label: "citygml".into(),
            scenario: Scenario::BBoxQuery,
            notes: "bbox-1pct".into(),
        };
        let row = pending.render(IoStats {
            bytes: 1234,
            requests: 1,
        });
        let fields: Vec<&str> = row.split(',').collect();
        assert_eq!(fields.len(), 22, "{row}");
        assert_eq!(&fields[..3], ["rotterdam", "citygml", "bbox-query"]);
        assert!(
            fields[3..14].iter().all(|f| f.is_empty()),
            "selectivity..peak_rss empty: {row}"
        );
        assert_eq!(fields[14], "0", "zero repeats");
        assert_eq!(
            fields[15],
            "bbox-1pct;derived-from=full-read;status=derived"
        );
        assert_eq!((fields[16], fields[17]), ("1234", "1"));
        assert!(fields[18..].iter().all(|f| f.is_empty()));
        let count = Pending {
            scenario: Scenario::Count,
            notes: String::new(),
            ..pending
        };
        assert!(
            count
                .render(IoStats {
                    bytes: 1,
                    requests: 1
                })
                .contains(",0,derived-from=full-read;status=derived,1,1,")
        );
    }

    fn pending(label: &str, scenario: Scenario, notes: &str) -> Pending {
        Pending {
            dataset: "rotterdam".into(),
            label: label.into(),
            scenario,
            notes: notes.into(),
        }
    }

    #[test]
    fn resolve_derives_only_the_formats_whose_premise_holds() {
        use std::collections::HashMap;
        let cells = [
            pending("citygml", Scenario::Count, ""),
            pending("citygml", Scenario::BBoxQuery, "bbox-1pct"),
            pending("cityjson", Scenario::Count, ""),
        ];
        let measured = HashMap::from([
            ("citygml".to_string(), vec![(Some(900), Some(1)); 3]),
            (
                "cityjson".to_string(),
                vec![(Some(700), Some(1)), (Some(700), Some(2))],
            ),
        ]);
        let sizes = HashMap::from([
            ("citygml".to_string(), Some(900)),
            ("cityjson".to_string(), Some(700)),
        ]);
        let r = resolve(&cells, &measured, &sizes);
        assert_eq!(r.lines.len(), 2, "{:?}", r.lines);
        assert!(
            r.lines
                .iter()
                .all(|l| l.starts_with("rotterdam,citygml,") && l.contains(",900,1,"))
        );
        assert_eq!(r.record["citygml"]["status"], "derived");
        assert_eq!(r.record["citygml"]["samples_checked"], 3);
        assert_eq!(r.record["cityjson"]["status"], "premise-failed");
        assert_eq!(r.warnings.len(), 1);
        assert!(
            r.warnings[0].contains("NOT DERIVED for cityjson")
                && r.warnings[0].contains("cityjson/count")
        );
    }

    #[test]
    fn the_consistency_report_names_every_skipped_derived_cell() {
        assert_eq!(consistency_skips(&[]), None);
        let report = consistency_skips(&[
            "citygml/count".into(),
            "citygml/bbox-query/bbox-1pct".into(),
        ])
        .unwrap();
        assert!(
            report.contains("skipped 2 derived cell(s)")
                && report.contains("citygml/bbox-query/bbox-1pct")
        );
    }
}
