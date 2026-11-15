//! `src/alloc.rs`'s high-water mark, tested alone.
//!
//! `PeakAlloc` counts every allocation in the process. In the binary's own
//! unit tests, the other tests run on parallel threads and allocate while
//! this one measures, so neither the rise nor the reset can be asserted
//! there. An integration test file is a binary of its own, and this one
//! holds a single test, so nothing else allocates while it runs. The module
//! is included by path so the test exercises the allocator the benchmark
//! uses, not a copy.

#[path = "../src/alloc.rs"]
mod alloc;

#[test]
fn peak_tracks_and_resets() {
    alloc::reset();
    let before = alloc::peak_heap_bytes();
    let v: Vec<u8> = vec![7u8; 4 * 1024 * 1024]; // 4 MiB
    let after = alloc::peak_heap_bytes();
    assert!(
        after >= before + 4 * 1024 * 1024,
        "peak did not rise: {before} -> {after}"
    );
    drop(v);
    alloc::reset();
    assert!(
        alloc::peak_heap_bytes() < 4 * 1024 * 1024,
        "reset did not lower the high-water mark"
    );
}
