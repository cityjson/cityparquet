//! A variant package is built by one code path, the same bytes on every
//! build, reused while its chain stamp is current and rebuilt otherwise — on
//! the real Delft fixture.

use std::path::{Path, PathBuf};

use cityparquet::variant::Variant;
use cityparquet_readbench::variant_package::{self, CHAIN_VERSION, reuse_or_build};

const ID: &str = "cityparquet+nobloom";

fn delft() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib/cityparquet-rs/tests/fixtures/delft.city.jsonl");
    assert!(
        p.exists(),
        "missing fixture delft.city.jsonl; run `just fixtures`"
    );
    p
}

fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&p).unwrap())
        })
        .collect();
    out.sort();
    out
}

#[test]
fn two_builds_are_the_same_bytes_and_stamped() {
    let variant = Variant::parse(ID).unwrap();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let pa = variant_package::build("delft", ID, &variant, a.path(), &delft()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let pb = variant_package::build("delft", ID, &variant, b.path(), &delft()).unwrap();
    assert_eq!(pa, a.path().join("delft.cityparquet+nobloom.parquet"));
    assert_eq!(files(&pa), files(&pb));
    let stamp = std::fs::read_to_string(variant_package::stamp_path(a.path(), "delft", ID));
    assert_eq!(stamp.unwrap().trim(), CHAIN_VERSION.to_string());
}

#[test]
fn a_current_package_is_reused_and_a_stale_one_rebuilt() {
    let variant = Variant::parse(ID).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (pkg, reused) = reuse_or_build("delft", ID, &variant, dir.path(), &delft()).unwrap();
    assert!(!reused, "nothing to reuse on the first run");
    let marker = pkg.join("marker");
    std::fs::write(&marker, "kept").unwrap();
    let (_, reused) = reuse_or_build("delft", ID, &variant, dir.path(), &delft()).unwrap();
    assert!(
        reused && marker.exists(),
        "a current package is read as it is"
    );
    let stamp = variant_package::stamp_path(dir.path(), "delft", ID);
    std::fs::write(&stamp, "7\n").unwrap();
    let (_, reused) = reuse_or_build("delft", ID, &variant, dir.path(), &delft()).unwrap();
    assert!(!reused && !marker.exists(), "a stale package is rebuilt");
}

#[test]
fn chain_version_matches_the_prepare_script() {
    let script = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/readbench_prepare.sh"),
    )
    .unwrap();
    let line = format!("CHAIN_VERSION={CHAIN_VERSION}");
    assert!(script.lines().any(|l| l == line), "missing `{line}`");
}
