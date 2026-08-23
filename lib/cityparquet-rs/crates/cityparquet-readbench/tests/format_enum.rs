//! `Format` is the vocabulary the whole harness shares: the `--format`
//! child dispatch, the coordinator's artefact naming, the CSV's `format`
//! column and the plotter's ordering all spell a format the same way.
//!
//! It exists as an enum rather than a `&str` because a format used to be
//! matched in three unrelated places plus two doc-comment lists plus a test
//! list, with no compiler help - so adding one was six edits and a hope.

use std::str::FromStr;

use cityparquet_readbench::format::Format;

#[test]
fn every_variant_round_trips_through_its_canonical_spelling() {
    for f in Format::ALL {
        assert_eq!(
            Format::from_str(f.as_str()).unwrap(),
            f,
            "{} did not round-trip",
            f.as_str()
        );
    }
}

#[test]
fn the_canonical_spellings_are_the_documented_tags() {
    let names: Vec<&str> = Format::ALL.iter().map(|f| f.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "citygml",
            "cityjson",
            "cityjsonseq",
            "cityjsonseq-gz",
            "flatcitybuf",
            "cityparquet",
            "cityparquet-hilbert",
            "duckdb-parquet",
        ]
    );
}

#[test]
fn an_unknown_name_names_every_valid_one() {
    let err = Format::from_str("not-a-format").unwrap_err();
    for f in Format::ALL {
        assert!(err.contains(f.as_str()), "error should list {}", f.as_str());
    }
}

// Which formats the `--child` path will run is asserted where the dispatch
// itself lives: `formats::resolve`'s own unit test enumerates `Format::ALL`
// and requires a runner for every variant except `duckdb-parquet`, which must
// error cleanly. That is the production statement of the same fact, so it is
// tested there rather than against a second, parallel classifier here.
