//! `cityparquet_readbench::params` against a real converted CityParquet
//! package, built here with `cityparquet::package::convert` from a committed
//! fixture — no network, no external tool, no prepared corpus.

use std::path::PathBuf;

use cityparquet::package::{ConvertOptions, convert};
use cityparquet_readbench::params::scan_row_bboxes;

fn fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures")
        .join(name);
    assert!(p.exists(), "missing fixture {name}; run `just fixtures`");
    p
}

/// Converts `delft.city.jsonl` into a package in a fresh temp dir and returns
/// its single main table. Delft is the fixture that by-type-converts to
/// exactly ONE table (Building + BuildingPart both map to the "Building"
/// family), which is what these tests need.
fn delft_table() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("delft.parquet");
    convert(&ConvertOptions::new(
        fixture("delft.city.jsonl"),
        out.clone(),
    ))
    .expect("converting delft");
    let table = out.join("building.parquet");
    assert!(table.exists(), "expected {} to exist", table.display());
    (dir, table)
}

#[test]
fn scan_row_bboxes_returns_one_box_per_row_and_their_union() {
    let (_dir, table) = delft_table();
    let scanned = scan_row_bboxes(&table).expect("scanning row bboxes");

    assert!(
        !scanned.boxes.is_empty(),
        "delft's table has rows, so it has row bboxes"
    );

    // The union must contain every row box, on every axis.
    for row in &scanned.boxes {
        for axis in 0..3 {
            assert!(
                scanned.dataset[axis] <= row[axis],
                "dataset min on axis {axis} must not exceed a row's min"
            );
            assert!(
                scanned.dataset[axis + 3] >= row[axis + 3],
                "dataset max on axis {axis} must not be below a row's max"
            );
        }
    }
}

use cityparquet_readbench::params::{citygml_ids, scan_seq};

#[test]
fn scan_seq_reads_the_stream_in_order_and_skips_the_metadata_line() {
    let ids = scan_seq(&fixture("delft.city.jsonl"), false)
        .expect("reading seq ids")
        .ids;
    assert!(!ids.is_empty(), "delft has features");
    assert!(
        !ids.iter().any(|id| id.is_empty()),
        "no feature id may be empty"
    );
    // The first line is the CityJSON metadata object, not a feature, so the
    // count is the feature count rather than the line count.
    let lines = std::fs::read_to_string(fixture("delft.city.jsonl"))
        .expect("reading the fixture")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(
        ids.len(),
        lines - 1,
        "one id per feature line, the metadata line excluded"
    );
}

/// Delft is a 3DBAG extract: every `Building` declares a
/// `geographicalExtent`, and the CityParquet `bbox` unions it in, reaching
/// up to 3 cm beyond the vertex box the other formats test. On the real
/// package, a window untied against the `bbox` edges alone lands inside such
/// a band for some target fraction (the formats would then disagree); untied
/// with the bands, no window edge lands inside one, for any target.
#[test]
fn delft_windows_keep_out_of_the_declared_extent_bands() {
    use cityparquet_readbench::params::{EdgeBands, untie_window, window_for_target};

    let (_dir, table) = delft_table();
    let rows = scan_row_bboxes(&table).expect("scanning row bboxes");
    let bands = scan_seq(&fixture("delft.city.jsonl"), false)
        .expect("scanning delft")
        .bands;
    assert!(
        (0..2).any(|a| !bands.lower[a].is_empty() || !bands.upper[a].is_empty()),
        "delft's declared extents reach beyond its vertices"
    );
    let inside = |w: &[f64; 6]| {
        (0..2).any(|a| {
            bands.lower[a]
                .iter()
                .any(|&(lo, hi)| lo <= w[a] && w[a] <= hi)
                || bands.upper[a]
                    .iter()
                    .any(|&(lo, hi)| lo <= w[a + 3] && w[a + 3] <= hi)
        })
    };
    let q = [0.001, 0.001];
    let mut hazards = 0;
    for percent in 1..=60 {
        let target = f64::from(percent) / 100.0;
        let raw = window_for_target(&rows.boxes, rows.dataset, target, "probe");
        let points_only = untie_window(&rows, raw.clone(), q, &EdgeBands::default());
        hazards += usize::from(inside(&points_only.window));
        let untied = untie_window(&rows, raw, q, &bands);
        assert!(
            !inside(&untied.window),
            "{percent}%: {:?} lies inside a band",
            untied.window
        );
    }
    assert!(hazards > 0, "no target exercised a band on delft");
}

/// The first Delft feature, its root object given a declared
/// `geographicalExtent` 2 mm wider than its subtree on every side (as 3DBAG
/// declares them, up to 1.5 mm wider): each x/y edge gets a band from the
/// vertex box to the declared extent, and the bands follow a swapped axis
/// order.
#[test]
fn scan_seq_bands_span_from_the_vertex_box_to_a_wider_declared_extent() {
    let text = std::fs::read_to_string(fixture("delft.city.jsonl")).expect("reading delft");
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().expect("header");
    let mut feature: serde_json::Value =
        serde_json::from_str(lines.next().expect("a feature")).expect("parsing the feature");
    let h: serde_json::Value = serde_json::from_str(header).expect("parsing the header");
    let (scale, translate) = (&h["transform"]["scale"], &h["transform"]["translate"]);
    let real = |v: &serde_json::Value, axis: usize| {
        v[axis].as_i64().unwrap() as f64 * scale[axis].as_f64().unwrap()
            + translate[axis].as_f64().unwrap()
    };
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for v in feature["vertices"].as_array().unwrap() {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(real(v, axis));
            hi[axis] = hi[axis].max(real(v, axis));
        }
    }
    let root = feature["id"].as_str().unwrap().to_string();
    let reach = 0.002;
    feature["CityObjects"][&root]["geographicalExtent"] = serde_json::json!([
        lo[0] - reach,
        lo[1] - reach,
        lo[2],
        hi[0] + reach,
        hi[1] + reach,
        hi[2]
    ]);
    let dir = tempfile::tempdir().expect("temp dir");
    let seq = dir.path().join("declared.city.jsonl");
    std::fs::write(&seq, format!("{header}\n{feature}\n")).expect("writing the stream");

    let bands = scan_seq(&seq, false).expect("scanning").bands;
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    for axis in 0..2 {
        let [lower] = bands.lower[axis][..] else {
            panic!("one lower band on axis {axis}: {:?}", bands.lower[axis]);
        };
        assert!(
            close(lower.0, hi[axis]) && close(lower.1, hi[axis] + reach),
            "{lower:?}"
        );
        let [upper] = bands.upper[axis][..] else {
            panic!("one upper band on axis {axis}: {:?}", bands.upper[axis]);
        };
        assert!(
            close(upper.0, lo[axis] - reach) && close(upper.1, lo[axis]),
            "{upper:?}"
        );
    }

    let swapped = scan_seq(&seq, true).expect("scanning swapped").bands;
    assert_eq!(swapped.lower[0], bands.lower[1]);
    assert_eq!(swapped.upper[1], bands.upper[0]);
}

/// `b1_lod2_cs_w_sem.gml` is one of the two CityGML 2.0 files `just
/// fixtures` fetches — a single semantically-decomposed building.
#[test]
fn citygml_ids_collects_every_city_object_key() {
    let ids = citygml_ids(&fixture("b1_lod2_cs_w_sem.gml")).expect("reading citygml ids");
    assert!(!ids.is_empty(), "the CityGML fixture has city objects");
}

use cityparquet_readbench::params::resolve;
use std::path::Path;

#[test]
fn resolve_produces_three_populated_windows_and_four_id_probes() {
    let (_dir, table) = delft_table();
    let resolved = resolve(
        "delft.city.jsonl",
        &table,
        Some(&fixture("delft.city.jsonl")),
        None,
    )
    .expect("resolving params");

    assert_eq!(resolved.windows.len(), 3, "three bbox windows");
    for window in &resolved.windows {
        assert!(
            window.achieved > 0.0,
            "{} selected no rows — the defect this replaces",
            window.tag
        );
    }

    assert_eq!(resolved.id_probes.len(), 4, "three deciles plus a miss");
    let attr_filter = resolved
        .attr_filter
        .as_ref()
        .expect("delft has string attributes, so a predicate is derivable");
    assert!(
        !attr_filter.column.is_empty(),
        "an attribute column was chosen"
    );
    assert!(
        attr_filter.matched > 0,
        "the predicate must match at least one row"
    );
    assert!(resolved.cp_object_total > 0, "a non-zero denominator");
}

#[test]
fn resolve_fails_loudly_when_the_seq_artefact_is_missing() {
    let (_dir, table) = delft_table();
    let err = resolve(
        "delft.city.jsonl",
        &table,
        Some(Path::new("/nonexistent/delft.city.jsonl")),
        None,
    )
    .expect_err("a seq artefact that is named but unreadable must be a hard failure");
    let message = format!("{err:#}");
    assert!(
        message.contains("delft.city.jsonl"),
        "the error must name the missing artefact, got: {message}"
    );
}

/// No seq artefact in the prepared directory means no canonical order to cut
/// deciles from. Deriving them from some other order would quietly redefine
/// what a probe's position means, so the scenario is skipped instead — the
/// same treatment a dataset with no numeric attribute gets.
#[test]
fn resolve_yields_no_id_probes_when_there_is_no_seq_artefact() {
    let (_dir, table) = delft_table();
    let resolved =
        resolve("delft.city.jsonl", &table, None, None).expect("resolving without a seq artefact");

    assert!(
        resolved.id_probes.is_empty(),
        "no seq artefact means no id probes, got {:?}",
        resolved.id_probes
    );
    // Everything the cityparquet package alone can answer still resolves.
    assert_eq!(resolved.windows.len(), 3, "windows need only the package");
    assert!(resolved.cp_object_total > 0);
}

// --- attr-filter predicate derivation, against a real package -------------

use cityparquet_readbench::params::{
    AttrFilterPred, open_arrow_schema, open_metadata, pick_attr_filter,
};

fn meta_of(table: &Path) -> cityparquet_schema::CityMetadata {
    open_metadata(table).expect("reading the package's city metadata")
}

fn schema_of(table: &Path) -> arrow_schema::Schema {
    open_arrow_schema(table).expect("reading the package's arrow schema")
}

/// `delft.city.jsonl` is not named in `params::HAND_PICKED`, so its
/// predicate comes from the derived rule: of its string attribute columns
/// with between 2 and 1000 distinct values, `b3_dak_type`'s most frequent
/// value (`slanted`) has the share of rows closest to 0.25 — 584 of 2231
/// CityObjects, independently confirmed with DuckDB over the converted
/// package. It is an attribute of the CityJSON `attributes` map, so
/// FlatCityBuf's `fcb ser -A` B+-tree indexes it; the reserved
/// `object_type` column this scenario used to be driven with is not, which
/// is why every committed FlatCityBuf row carried `no-attr-index`.
#[test]
fn the_derived_rule_picks_an_indexable_attribute_near_the_target_share() {
    let (_dir, table) = delft_table();
    let picked = pick_attr_filter(
        "delft.city.jsonl",
        &meta_of(&table),
        &schema_of(&table),
        &table,
    )
    .expect("deriving the attr-filter predicate")
    .expect("delft has string attribute columns");

    assert_eq!(picked.column, "b3_dak_type");
    assert_eq!(picked.pred, AttrFilterPred::Eq("slanted".to_string()));
    assert_eq!(picked.matched, 584);
    assert!(
        !picked.hand_picked,
        "delft is not in the hand-picked table; it must come from the derived rule"
    );
    assert!(
        (picked.share - 584.0 / 2231.0).abs() < 1e-9,
        "share must be the matched fraction of the table's rows, got {}",
        picked.share
    );
}

/// The same package, asked for under a 3DBAG slice's name: the hand-picked
/// table wins over the derived rule, and says so. (`delft.city.jsonl` IS
/// 3DBAG data, so the pick resolves against this table — which is what
/// makes this checkable without the multi-gigabyte corpus.)
#[test]
fn the_hand_picked_table_wins_over_the_derived_rule_and_is_disclosed() {
    let (_dir, table) = delft_table();
    let picked = pick_attr_filter(
        "3dbag_n1000.city.jsonl",
        &meta_of(&table),
        &schema_of(&table),
        &table,
    )
    .expect("deriving the attr-filter predicate")
    .expect("the hand-picked column is present in this package");

    assert_eq!(picked.column, "b3_dak_type");
    assert_eq!(picked.pred, AttrFilterPred::Eq("slanted".to_string()));
    assert!(
        picked.hand_picked,
        "a hand-picked predicate must be disclosed as one in the sidecar"
    );
}

/// A hand-picked entry naming a column this package does not carry must
/// fall back to the derived rule rather than fail the run or measure a
/// query that returns nothing.
#[test]
fn a_hand_picked_column_the_package_lacks_falls_back_to_the_derived_rule() {
    let (_dir, table) = delft_table();
    let picked = pick_attr_filter(
        "rotterdam_delfshaven.city.jsonl",
        &meta_of(&table),
        &schema_of(&table),
        &table,
    )
    .expect("a missing hand-picked column is not an error")
    .expect("the derived rule still finds a predicate");

    assert_eq!(
        picked.column, "b3_dak_type",
        "delft carries no TerrainHeight, so the derived rule must answer instead"
    );
    assert!(!picked.hand_picked);
}

/// A package converted from `input` with small row groups, with or without
/// bloom filters, Hilbert-ordered as the benchmark's packages are; returns
/// its single `building.parquet` table.
fn small_group_table(input: &Path, rows: usize, bloom: bool) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = dir.path().join("pkg.parquet");
    let mut opts = ConvertOptions::new(input.to_path_buf(), out.clone());
    opts.generate_lod0 = false;
    opts.recipe.row_group_size = rows;
    opts.recipe.bloom.enabled = bloom;
    convert(&opts).expect("converting");
    let table = out.join("building.parquet");
    assert!(table.exists(), "expected {} to exist", table.display());
    (dir, table)
}

/// The miss probes must be identifiers that row-group min/max statistics
/// cannot reject on their own: without bloom filters a miss must still be
/// read from (some) row groups, or the bloom axis compares two prunings that
/// are both free. With bloom filters, the filters reject it. Returns
/// `(row_groups_total, bloom_pruned, stats_pruned, single_valued)` per
/// variant and probe, `single_valued` being the row groups whose statistics
/// on the probe's column have `min == max`: a group holding one identifier
/// rejects every other one, so no absent probe can sit inside its range.
type Counters = (usize, usize, usize, usize);
fn miss_probe_counters(input: &Path, rows: usize) -> Vec<(bool, String, Counters)> {
    let mut out = Vec::new();
    for bloom in [false, true] {
        let (_dir, table) = small_group_table(input, rows, bloom);
        let resolved = resolve("probe", &table, Some(input), None).expect("resolving params");
        let id_miss = resolved
            .id_probes
            .iter()
            .find(|p| p.tag == "id-miss")
            .expect("an id-miss probe");
        let feature_miss = resolved
            .feature_probes
            .iter()
            .find(|p| p.tag == "feature-miss")
            .expect("a feature-miss probe");
        let single_valued = |column: &str| {
            cityparquet_readbench::params::row_group_string_ranges(&table, column)
                .unwrap()
                .iter()
                .filter(|r| r.as_ref().is_some_and(|(min, max)| min == max))
                .count()
        };
        let (totals, s) = cityparquet::query::id_lookup_visit(&table, &id_miss.id).unwrap();
        assert_eq!(
            totals.objects, 0,
            "the id-miss probe {} is absent",
            id_miss.id
        );
        out.push((
            bloom,
            "id-miss".to_string(),
            (
                s.row_groups_total,
                s.bloom_pruned,
                s.stats_pruned,
                single_valued("id"),
            ),
        ));
        let (totals, s) =
            cityparquet::query::feature_lookup_visit(&table, &feature_miss.id).unwrap();
        assert_eq!(totals.objects, 0, "the feature-miss probe is absent");
        out.push((
            bloom,
            "feature-miss".to_string(),
            (
                s.row_groups_total,
                s.bloom_pruned,
                s.stats_pruned,
                single_valued("feature_id"),
            ),
        ));
    }
    out
}

fn assert_statistics_cannot_reject_the_misses(input: &Path, rows: usize) {
    let counters = miss_probe_counters(input, rows);
    eprintln!("{}: {counters:?}", input.display());
    for (bloom, tag, (total, bloom_pruned, stats_pruned, single_valued)) in counters {
        assert!(total > 1, "{tag}: the fixture must span several row groups");
        if bloom {
            assert!(
                bloom_pruned > 0,
                "{tag}: the bloom filters must reject the miss somewhere"
            );
        } else {
            assert_eq!(bloom_pruned, 0, "{tag}: no filters, nothing bloom-pruned");
            assert!(
                stats_pruned < total,
                "{tag}: statistics rejected every group"
            );
            assert_eq!(
                stats_pruned, single_valued,
                "{tag}: statistics alone rejected {stats_pruned} of {total} row groups, \
                 but only {single_valued} hold a single identifier"
            );
        }
    }
}

#[test]
fn statistics_reject_the_miss_probes_only_in_single_identifier_groups_on_delft() {
    assert_statistics_cannot_reject_the_misses(&fixture("delft.city.jsonl"), 256);
}

#[test]
fn statistics_reject_the_miss_probes_only_in_single_identifier_groups_on_tokyo() {
    let tokyo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tokyo_chiyoda_40.city.jsonl");
    assert_statistics_cannot_reject_the_misses(&tokyo, 16);
}
