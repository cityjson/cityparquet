//! The timing statistics a results-CSV row reports over its warm samples.
//!
//! One definition for every timing column, shared with the database family
//! (`benchmark/databases/src/citybench/stats.py`), so a timing quoted from
//! either CSV is the same statistic:
//!
//! - `mean`: the arithmetic mean;
//! - `std`: the POPULATION standard deviation about the mean (the warm
//!   repeats are the whole measured set, not a draw used to infer a wider one);
//! - `median`, `q1`, `q3`: linear interpolation at position `p * (n - 1)` on
//!   the sorted samples (numpy's default; Python's
//!   `statistics.quantiles(method="inclusive")`), so the median of an even
//!   count is the mean of the two middle values;
//! - `min`, `max`: the extreme samples.

/// The seven timing statistics, in the CSV timing block's column order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingStats {
    pub mean: f64,
    pub std: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub q1: f64,
    pub q3: f64,
}

impl TimingStats {
    /// Every statistic of `values`. Panics on an empty slice: a row always
    /// has at least one warm sample.
    pub fn of(values: &[f64]) -> Self {
        assert!(
            !values.is_empty(),
            "timing statistics need at least one sample"
        );
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values
            .iter()
            .map(|v| {
                let d = v - mean;
                d * d
            })
            .sum::<f64>()
            / values.len() as f64;
        TimingStats {
            mean,
            std: variance.sqrt(),
            median: quantile_sorted(&sorted, 0.5),
            min: sorted[0],
            max: sorted[sorted.len() - 1],
            q1: quantile_sorted(&sorted, 0.25),
            q3: quantile_sorted(&sorted, 0.75),
        }
    }
}

/// The `p` quantile (0 <= p <= 1) of an already-sorted, non-empty slice, by
/// linear interpolation at position `p * (n - 1)` (see the module docs).
pub fn quantile_sorted(sorted: &[f64], p: f64) -> f64 {
    assert!(!sorted.is_empty(), "a quantile needs at least one sample");
    let position = p * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = (lower + 1).min(sorted.len() - 1);
    let fraction = position - lower as f64;
    sorted[lower] + (sorted[upper] - sorted[lower]) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn even_count_interpolates_median_and_quartiles() {
        let s = TimingStats::of(&[4.0, 1.0, 3.0, 2.0]);
        assert!(close(s.median, 2.5), "{s:?}");
        assert!(close(s.q1, 1.75), "{s:?}");
        assert!(close(s.q3, 3.25), "{s:?}");
        assert_eq!((s.min, s.max), (1.0, 4.0));
        assert!(close(s.mean, 2.5));
    }

    #[test]
    fn odd_count_lands_on_samples() {
        let s = TimingStats::of(&[5.0, 3.0, 1.0, 4.0, 2.0]);
        assert_eq!((s.median, s.q1, s.q3), (3.0, 2.0, 4.0));
        assert_eq!((s.min, s.max), (1.0, 5.0));
    }

    #[test]
    fn single_sample_collapses_to_it() {
        let s = TimingStats::of(&[0.7]);
        assert_eq!(
            s,
            TimingStats {
                mean: 0.7,
                std: 0.0,
                median: 0.7,
                min: 0.7,
                max: 0.7,
                q1: 0.7,
                q3: 0.7
            }
        );
    }

    #[test]
    fn std_is_population_not_sample() {
        assert!(close(TimingStats::of(&[1.0, 3.0]).std, 1.0));
    }
}
