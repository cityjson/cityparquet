//! The [`Scenario`] enum and [`QueryParams`] shared across every format
//! runner (`formats::FormatRunner`).
//!
//! This is the one seam the whole read-benchmark milestone hangs off:
//! Task 8 establishes it against the CityParquet backend; Tasks 9/10 reuse
//! it, unchanged, for CityJSONSeq/gzipped-CityJSONSeq and FlatCityBuf; the
//! Task 11 coordinator is the only thing that ever *populates* a
//! [`QueryParams`] with real values (dataset bbox windows, a sampled
//! attribute column/predicate, a sampled id) — this task's own `--child`
//! CLI parsing in `main.rs` exists only so the CityParquet runner can be
//! exercised end-to-end today.

use std::str::FromStr;

/// One read-access-pattern scenario. Every format backend
/// (`formats::FormatRunner`) implements all seven via its own natural
/// mechanism — see the milestone plan's "Scenario & metric contract" table
/// for the per-format mapping and the common materialisation target each
/// variant forces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scenario {
    /// Decode every feature's geometry; the "no format may skip work"
    /// baseline.
    FullRead,
    /// Total feature/object count.
    Count,
    /// Ids of objects whose bbox intersects a query window.
    BBoxQuery,
    /// Count of objects matching an attribute predicate.
    AttrFilter,
    /// `(min, max, sum, count)` of a numeric attribute.
    AttrStats,
    /// The single object with a given id.
    IdLookup,
    /// One attribute column read across every row; non-null count.
    Project,
}

impl Scenario {
    /// Every variant, in the milestone plan's canonical order.
    pub const ALL: [Scenario; 7] = [
        Scenario::FullRead,
        Scenario::Count,
        Scenario::BBoxQuery,
        Scenario::AttrFilter,
        Scenario::AttrStats,
        Scenario::IdLookup,
        Scenario::Project,
    ];

    /// The canonical kebab-case CLI/CSV spelling (round-trips through
    /// [`FromStr`]).
    pub fn as_str(self) -> &'static str {
        match self {
            Scenario::FullRead => "full-read",
            Scenario::Count => "count",
            Scenario::BBoxQuery => "bbox-query",
            Scenario::AttrFilter => "attr-filter",
            Scenario::AttrStats => "attr-stats",
            Scenario::IdLookup => "id-lookup",
            Scenario::Project => "project",
        }
    }
}

impl std::fmt::Display for Scenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scenario {
    type Err = String;

    /// Accepts the canonical kebab-case spelling case-insensitively, plus a
    /// no-separator lowercase alias (`fullread`, `bboxquery`, ...) for
    /// convenience.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "full-read" | "fullread" => Ok(Scenario::FullRead),
            "count" => Ok(Scenario::Count),
            "bbox-query" | "bboxquery" | "bbox" => Ok(Scenario::BBoxQuery),
            "attr-filter" | "attrfilter" => Ok(Scenario::AttrFilter),
            "attr-stats" | "attrstats" => Ok(Scenario::AttrStats),
            "id-lookup" | "idlookup" => Ok(Scenario::IdLookup),
            "project" => Ok(Scenario::Project),
            other => Err(format!(
                "unknown scenario '{other}'; expected one of: {}",
                Scenario::ALL
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

/// A CLI-passable mirror of `cityparquet::query::AttrPredicate` — kept as
/// its own small enum here (rather than depending on the library type
/// directly) so this crate's CLI-parsing layer never needs to know about
/// `cityparquet`'s internals; each format runner maps its own
/// [`AttrPred`] onto whatever predicate type its backend actually wants
/// (`cityparquet::query::AttrPredicate` for the CityParquet runner;
/// Tasks 9/10 do the analogous mapping for their own backends).
#[derive(Debug, Clone, PartialEq)]
pub enum AttrPred {
    /// Equality against a string or number (dispatched by the runner on the
    /// column's actual type).
    Eq(serde_json::Value),
    /// `value >= bound`.
    Ge(f64),
    /// `value <= bound`.
    Le(f64),
    /// `lo <= value <= hi`.
    Range(f64, f64),
}

/// Every parameter a [`Scenario`] might need, populated by the (Task 11)
/// coordinator when it drives real runs and — for this task's `--child`
/// CLI — directly from flags, so the CityParquet runner can be exercised
/// end-to-end without the coordinator existing yet. Every field is
/// `Option`: a given scenario only reads the fields it needs (see
/// `formats::cityparquet`'s `Scenario` match for which).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryParams {
    /// `[minx, miny, minz, maxx, maxy, maxz]` query window for
    /// [`Scenario::BBoxQuery`].
    pub bbox: Option<[f64; 6]>,
    /// Attribute column name for [`Scenario::AttrFilter`],
    /// [`Scenario::AttrStats`], and [`Scenario::Project`].
    pub attr_column: Option<String>,
    /// Predicate for [`Scenario::AttrFilter`].
    pub attr_pred: Option<AttrPred>,
    /// Target object id for [`Scenario::IdLookup`].
    pub target_id: Option<String>,
    /// Free-text label (e.g. `bbox-1pct`) the coordinator threads through
    /// to the results CSV's `notes` column; no runner reads this itself.
    pub selectivity_tag: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_round_trips_through_its_canonical_string() {
        for scenario in Scenario::ALL {
            let parsed: Scenario = scenario.as_str().parse().unwrap();
            assert_eq!(parsed, scenario);
        }
    }

    #[test]
    fn from_str_is_case_insensitive_and_rejects_unknown_names() {
        assert_eq!("COUNT".parse::<Scenario>().unwrap(), Scenario::Count);
        assert_eq!(
            "Bbox-Query".parse::<Scenario>().unwrap(),
            Scenario::BBoxQuery
        );
        assert!("not-a-scenario".parse::<Scenario>().is_err());
    }
}
