use peak_alloc::PeakAlloc;

#[global_allocator]
pub static PEAK: PeakAlloc = PeakAlloc;

/// Reset the heap high-water mark to the current live usage.
///
/// Unused outside tests until a later task wires per-format benchmark runs
/// through this allocator; `#[allow(dead_code)]` avoids a `-D warnings`
/// failure in the interim.
#[allow(dead_code)]
pub fn reset() {
    PEAK.reset_peak_usage();
}

/// Peak live heap bytes since the last `reset()`.
#[allow(dead_code)]
pub fn peak_heap_bytes() -> usize {
    PEAK.peak_usage()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peak_tracks_and_resets() {
        reset();
        let before = peak_heap_bytes();
        let v: Vec<u8> = vec![7u8; 4 * 1024 * 1024]; // 4 MiB
        let after = peak_heap_bytes();
        assert!(
            after >= before + 4 * 1024 * 1024,
            "peak did not rise: {before} -> {after}"
        );
        drop(v);
        reset();
        assert!(
            peak_heap_bytes() < 4 * 1024 * 1024,
            "reset did not lower the high-water mark"
        );
    }
}
