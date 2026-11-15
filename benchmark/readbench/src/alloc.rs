use peak_alloc::PeakAlloc;

#[global_allocator]
pub static PEAK: PeakAlloc = PeakAlloc;

/// Reset the heap high-water mark to the current live usage.
pub fn reset() {
    PEAK.reset_peak_usage();
}

/// Peak live heap bytes since the last `reset()`.
pub fn peak_heap_bytes() -> usize {
    PEAK.peak_usage()
}
