//! The benchmark has two axes and measures them separately: which FORMAT,
//! and (for CityParquet) which ORDERING. A run that varies both at once
//! answers neither question cleanly.
//!
//! Both sets are named consts on [`Format`] rather than on the coordinator:
//! `coordinator` is a module of the BINARY (`src/main.rs` declares `mod
//! coordinator;`), so an integration test cannot reach it at all, and the
//! justfile recipes and docs need to name the ordering set as well as the
//! default one. `Format::ALL` already lives here, so its two measured
//! subsets belong beside it.

use cityparquet_readbench::format::Format;

#[test]
fn the_default_set_is_one_tag_per_format_family() {
    assert_eq!(
        Format::DEFAULT_SET,
        [
            Format::CityGml,
            Format::CityJson,
            Format::CityJsonSeq,
            Format::FlatCityBuf,
            Format::CityParquetHilbert,
        ]
    );
}

#[test]
fn cityparquet_is_represented_by_its_best_configuration() {
    // Hilbert ordering is the configuration we would ship, so it is the one
    // the format comparison should carry - otherwise CityParquet is
    // handicapped by an ordering choice the other formats never face.
    let d = Format::DEFAULT_SET;
    assert!(d.contains(&Format::CityParquetHilbert));
    assert!(!d.contains(&Format::CityParquet));
}

#[test]
fn the_ordering_set_isolates_the_sort_strategy() {
    assert_eq!(
        Format::ORDERING_SET,
        [Format::CityParquet, Format::CityParquetHilbert]
    );
}

#[test]
fn compression_and_engine_baselines_are_opt_in() {
    let d = Format::DEFAULT_SET;
    assert!(
        !d.contains(&Format::CityJsonSeqGz),
        "a compression variant is not a format"
    );
    assert!(
        !d.contains(&Format::DuckDbParquet),
        "an engine baseline is not a format"
    );
}

/// Both measured sets are subsets of the canonical vocabulary, in its
/// canonical order — so a chart built from either reads left-to-right the
/// same way [`Format::ALL`] does, and neither set can name a format the
/// harness does not know.
#[test]
fn both_sets_are_ordered_subsets_of_the_canonical_vocabulary() {
    for set in [
        Format::DEFAULT_SET.as_slice(),
        Format::ORDERING_SET.as_slice(),
    ] {
        let positions: Vec<usize> = set
            .iter()
            .map(|f| {
                Format::ALL
                    .iter()
                    .position(|c| c == f)
                    .unwrap_or_else(|| panic!("{f} is not in Format::ALL"))
            })
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "set {set:?} is not in Format::ALL's canonical order"
        );
    }
}
