//! Merging tiled inputs: identical copies of one feature, and ids that only
//! mean something within their tile.
//!
//! Montréal's building models show both. 2020 Ville-Marie ships 900 buildings
//! twice — a building straddling two tiles appears in each, identical but for
//! the folder its (byte-identical) texture image sits in. And RhinoCity's
//! generated `GroupeNNN` ids restart in every tile, so `Groupe133` names five
//! different buildings of 2016 Verdun. `MergeOptions::dedupe_identical` keeps
//! one copy of the first; `MergeOptions::prefix_ids_by_input` makes every id
//! `<input stem>.<id>`, the second.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cityparquet::merge::{MergeOptions, merge_sources};
use cityparquet::source::Source;

fn data_fixture(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    assert!(p.exists(), "missing committed fixture {name}");
    p
}

const DEDUPE: MergeOptions = MergeOptions {
    dedupe_identical: true,
    prefix_ids_by_input: false,
};
const PREFIX: MergeOptions = MergeOptions {
    dedupe_identical: false,
    prefix_ids_by_input: true,
};

/// `building_with_appearance.gml` copied to `dir/<name>.gml` with its texture
/// folder renamed to `folder`, and that folder's image written as `bytes` — a
/// tile of its own, as a straddling building arrives in each tile it touches.
fn tile(dir: &Path, name: &str, folder: &str, bytes: &[u8]) -> PathBuf {
    let text = std::fs::read_to_string(data_fixture("building_with_appearance.gml")).unwrap();
    assert!(
        text.contains("textures/wall.jpg"),
        "fixture names its image"
    );
    let tile_dir = dir.join(name);
    std::fs::create_dir_all(tile_dir.join(folder)).unwrap();
    std::fs::write(tile_dir.join(folder).join("wall.jpg"), bytes).unwrap();
    let path = tile_dir.join(format!("{name}.gml"));
    std::fs::write(
        &path,
        text.replace("textures/wall.jpg", &format!("{folder}/wall.jpg")),
    )
    .unwrap();
    path
}

#[test]
fn the_same_feature_twice_is_kept_once() {
    let path = data_fixture("building_with_parts.gml");
    let one = Source::open(&path).unwrap();
    let n = one.features().unwrap().count();
    let merged = merge_sources(&[one, Source::open(&path).unwrap()], &DEDUPE).unwrap();
    assert_eq!(merged.features.len(), n);
    assert_eq!(merged.deduplicated, n);
    assert_eq!(merged.duplicate_ids, 0, "nothing distinct shares an id");
}

#[test]
fn without_the_option_both_copies_are_kept_and_counted() {
    let path = data_fixture("building_with_parts.gml");
    let one = Source::open(&path).unwrap();
    let n = one.features().unwrap().count();
    let merged = merge_sources(
        &[one, Source::open(&path).unwrap()],
        &MergeOptions::default(),
    )
    .unwrap();
    assert_eq!(merged.features.len(), 2 * n);
    assert_eq!((merged.deduplicated, merged.duplicate_ids), (0, n));
}

#[test]
fn copies_whose_images_differ_only_in_folder_are_one_feature() {
    let d = tempfile::tempdir().unwrap();
    let a = tile(d.path(), "VM13_2020", "VM13_2020_Appearance", b"JPEG");
    let b = tile(d.path(), "VM15_2020", "VM15_2020_Appearance", b"JPEG");
    let merged = merge_sources(
        &[Source::open(&a).unwrap(), Source::open(&b).unwrap()],
        &DEDUPE,
    )
    .unwrap();
    assert_eq!((merged.features.len(), merged.deduplicated), (1, 1));
    // The copy kept is the first, image path and all.
    let json = serde_json::to_string(&merged.features[0]).unwrap();
    assert!(json.contains("VM13_2020_Appearance/wall.jpg"), "{json}");
}

#[test]
fn copies_whose_images_differ_in_content_are_both_kept() {
    let d = tempfile::tempdir().unwrap();
    let a = tile(d.path(), "VM13_2020", "VM13_2020_Appearance", b"JPEG");
    let b = tile(d.path(), "VM15_2020", "VM15_2020_Appearance", b"OTHER");
    let merged = merge_sources(
        &[Source::open(&a).unwrap(), Source::open(&b).unwrap()],
        &DEDUPE,
    )
    .unwrap();
    assert_eq!(merged.features.len(), 2);
    assert_eq!((merged.deduplicated, merged.duplicate_ids), (0, 1));
}

#[test]
fn every_id_and_reference_is_prefixed_by_its_input() {
    let parts = data_fixture("building_with_parts.gml");
    let other = data_fixture("nonbuilding_objects.gml");
    let merged = merge_sources(
        &[Source::open(&parts).unwrap(), Source::open(&other).unwrap()],
        &PREFIX,
    )
    .unwrap();

    let mut ids = BTreeSet::new();
    for f in &merged.features {
        let stem = if f.id.starts_with("building_with_parts.") {
            "building_with_parts."
        } else {
            "nonbuilding_objects."
        };
        assert!(f.id.starts_with(stem), "feature id {}", f.id);
        for (id, co) in &f.city_objects {
            assert!(id.starts_with(stem), "object id {id}");
            assert!(ids.insert(id.clone()), "ids stay unique: {id}");
            for r in co
                .parents
                .iter()
                .flatten()
                .chain(co.children.iter().flatten())
            {
                assert!(r.starts_with(stem), "reference {r} of {id}");
                assert!(f.city_objects.contains_key(r), "{r} still resolves");
            }
        }
        assert!(
            f.city_objects.contains_key(&f.id),
            "the feature's own object"
        );
    }
    let with_parts = merged
        .features
        .iter()
        .flat_map(|f| f.city_objects.values())
        .filter(|co| co.children.as_ref().is_some_and(|c| !c.is_empty()))
        .count();
    assert!(
        with_parts > 0,
        "the fixture exercises parent/child references"
    );
}

#[test]
fn a_lone_input_is_prefixed_too() {
    let parts = data_fixture("building_with_parts.gml");
    let merged = merge_sources(&[Source::open(&parts).unwrap()], &PREFIX).unwrap();
    assert!(
        merged
            .features
            .iter()
            .all(|f| f.id.starts_with("building_with_parts."))
    );
}

#[test]
fn two_inputs_with_one_stem_cannot_be_told_apart() {
    let d = tempfile::tempdir().unwrap();
    let a = tile(d.path(), "x", "a", b"A");
    let b_dir = d.path().join("y");
    std::fs::create_dir_all(&b_dir).unwrap();
    let b = b_dir.join("x.gml");
    std::fs::copy(&a, &b).unwrap();
    let err = merge_sources(
        &[Source::open(&a).unwrap(), Source::open(&b).unwrap()],
        &PREFIX,
    )
    .expect_err("a prefix that names two inputs is no prefix");
    assert!(err.to_string().contains("x"), "{err}");
}

#[test]
fn identical_copies_are_found_before_their_ids_are_prefixed() {
    let d = tempfile::tempdir().unwrap();
    let a = tile(d.path(), "VM13_2020", "VM13_2020_Appearance", b"JPEG");
    let b = tile(d.path(), "VM15_2020", "VM15_2020_Appearance", b"JPEG");
    let both = MergeOptions {
        dedupe_identical: true,
        prefix_ids_by_input: true,
    };
    let merged = merge_sources(
        &[Source::open(&a).unwrap(), Source::open(&b).unwrap()],
        &both,
    )
    .unwrap();
    assert_eq!(merged.features.len(), 1);
    assert!(merged.features[0].id.starts_with("VM13_2020."));
}
