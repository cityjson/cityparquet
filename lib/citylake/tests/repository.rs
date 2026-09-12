mod common;

use citylake::core::interface::repository::CityLakeRepository;
use citylake::core::interface::types::DatasetName;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The trait is the boundary handlers see, so it must be object-safe: a
/// handler holds Arc<dyn CityLakeRepository>, not a concrete service.
#[tokio::test]
async fn the_service_is_usable_as_a_trait_object() {
    let (service, _dir) = common::test_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);

    let name = DatasetName::new("delft").unwrap();
    repo.create_dataset(&name, common::fixture("delft.city.jsonl").to_str().unwrap())
        .await
        .expect("create through the trait object");
    assert!(repo
        .list_datasets()
        .await
        .unwrap()
        .contains(&"delft".to_string()));
}

/// A CityJSONSeq fixture with `count` Building features, each with its own id
/// and a disjoint footprint. Generated rather than committed: what matters is
/// only that `create_dataset` on it takes long enough to overlap with a
/// concurrently-scheduled task, and every fixture already under
/// `tests/data/` loads too fast for that.
fn large_fixture(dir: &std::path::Path, count: usize) -> std::path::PathBuf {
    let path = dir.join("large.city.jsonl");
    let mut file = std::fs::File::create(&path).expect("create the fixture file");
    writeln!(
        file,
        r#"{{"type":"CityJSON","version":"2.0","transform":{{"scale":[0.001,0.001,0.001],"translate":[0.0,0.0,0.0]}},"metadata":{{"referenceSystem":"https://www.opengis.net/def/crs/EPSG/0/7415"}}}}"#
    )
    .unwrap();
    for i in 0..count {
        let id = format!("NL.IMBAG.Pand.{i:012}");
        let x = i as i64 * 100;
        writeln!(
            file,
            r#"{{"type":"CityJSONFeature","id":"{id}","CityObjects":{{"{id}":{{"type":"Building","attributes":{{"bouwjaar":1980}},"geometry":[{{"type":"Solid","lod":"2.2","boundaries":[[[[0,1,2,3]],[[4,5,6,7]],[[0,1,5,4]],[[1,2,6,5]],[[2,3,7,6]],[[3,0,4,7]]]]}}]}}}},"vertices":[[{x},0,0],[{x},10,0],[{x2},10,0],[{x2},0,0],[{x},0,5],[{x},10,5],[{x2},10,5],[{x2},0,5]]}}"#,
            x2 = x + 10,
        )
        .unwrap();
    }
    path
}

/// The DuckDB connection is behind a blocking mutex. If a trait method ran
/// its DuckDB work inline on the async executor instead of in
/// `spawn_blocking`, a slow call would pin whichever worker thread polled it
/// — freezing every *other* task scheduled on that thread, including ones
/// that never touch the database — until the call finished.
///
/// Two concurrent `list_datasets` calls cannot show this: both go through
/// the same connection mutex regardless of whether the DB work is offloaded,
/// so on a multi-thread runtime they may simply run one after another on
/// separate OS threads without either ever occupying an async worker thread
/// synchronously. That would pass whether or not `spawn_blocking` was used —
/// it does not discriminate.
///
/// This test instead pins the runtime to a single worker thread, starts a
/// slow `create_dataset` call, and races it against a "ticker" task that
/// touches the database not at all — it only sleeps and counts. Running
/// `create_dataset`'s DuckDB work inline on that one worker thread, instead
/// of via `spawn_blocking`, pins the thread for the call's whole duration:
/// the ticker cannot be polled at all until the call's poll returns, so it
/// records zero ticks for the whole thing.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_slow_call_does_not_block_unrelated_work_on_the_worker_thread() {
    let (service, dir) = common::test_service();
    let repo: Arc<dyn CityLakeRepository> = Arc::new(service);
    // Small enough to keep the suite quick, but this dataset's fixed
    // bootstrap cost alone (schema creation, CRS derivation, routing) already
    // takes on the order of two seconds against this extension — comfortably
    // longer than the 5ms ticker interval needs to prove the point.
    let source = large_fixture(dir.path(), 50);
    let name = DatasetName::new("large").unwrap();

    let slow_repo = Arc::clone(&repo);
    let slow_source = source.to_str().unwrap().to_string();
    let slow = tokio::spawn(async move {
        let started = Instant::now();
        let result = slow_repo.create_dataset(&name, &slow_source).await;
        (result, started.elapsed())
    });

    let ticks = Arc::new(AtomicUsize::new(0));
    let ticker_ticks = Arc::clone(&ticks);
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(5)).await;
            ticker_ticks.fetch_add(1, Ordering::SeqCst);
        }
    });

    let (slow_result, slow_elapsed) = slow.await.unwrap();
    ticker.abort();

    assert!(
        slow_result.is_ok(),
        "create_dataset failed: {slow_result:?}"
    );
    assert!(
        slow_elapsed > Duration::from_millis(200),
        "create_dataset finished in {slow_elapsed:?}, too fast for this test \
         to tell blocking from non-blocking apart — grow `count` in \
         large_fixture"
    );
    let observed = ticks.load(Ordering::SeqCst);
    assert!(
        observed > 5,
        "the ticker only advanced {observed} times in {slow_elapsed:?} of \
         create_dataset running — on a single worker thread that means \
         create_dataset's DuckDB work pinned the executor instead of running \
         inside spawn_blocking"
    );
}
