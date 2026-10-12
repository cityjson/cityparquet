//! How many timed samples one cell takes: the stop rule of the sampling loop.
//!
//! A cell is one (dataset, format, scenario, query) measurement. It runs one
//! discarded warm-up and then up to `repeat` timed samples, back to back.
//! With a cell time budget set, sampling stops early once the budget is
//! spent, but never below the `min_repeat` floor.
//!
//! The rule, after each timed sample (`taken` = timed samples so far,
//! `elapsed` = wall time of the cell's runs so far, warm-up included): stop
//! when `taken == repeat`, or when a budget is set, `elapsed >= budget` and
//! `taken >= min_repeat`. A `min_repeat` above `repeat` is clamped to
//! `repeat` (a smoke run with `repeat = 1` keeps the default floor of 7
//! without failing), so `repeat` always remains the ceiling.

use std::time::Duration;

use anyhow::{Result, bail};

/// The resolved sampling parameters of a run; see the module documentation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplingPlan {
    /// The most timed samples a cell takes.
    pub repeat: usize,
    /// The cell time budget, `None` when sampling always takes `repeat`.
    pub cell_budget: Option<Duration>,
    /// The fewest timed samples a cell takes, already clamped to `repeat`.
    pub min_repeat: usize,
}

impl SamplingPlan {
    /// Validates and resolves the CLI values: `repeat` and `min_repeat` must
    /// be >= 1, a budget must be a positive finite number of seconds, and
    /// `min_repeat` is clamped to `repeat`.
    pub fn new(repeat: usize, cell_budget_s: Option<f64>, min_repeat: usize) -> Result<Self> {
        if repeat == 0 {
            bail!("--repeat must be >= 1");
        }
        if min_repeat == 0 {
            bail!("--min-repeat must be >= 1");
        }
        let cell_budget = match cell_budget_s {
            None => None,
            Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
                Some(Duration::from_secs_f64(seconds))
            }
            Some(seconds) => bail!("--cell-budget-s must be a positive number, got {seconds}"),
        };
        Ok(Self {
            repeat,
            cell_budget,
            min_repeat: min_repeat.min(repeat),
        })
    }

    /// Whether the cell stops after its `taken`-th timed sample, `elapsed`
    /// being the wall time of all its runs so far, warm-up included.
    pub fn stop_after(&self, taken: usize, elapsed: Duration) -> bool {
        taken >= self.repeat
            || (taken >= self.min_repeat
                && self.cell_budget.is_some_and(|budget| elapsed >= budget))
    }

    /// Whether a cell that took `taken` timed samples stopped on the budget
    /// (and so carries the `budget` tag in `notes`).
    pub fn stopped_early(&self, taken: usize) -> bool {
        taken < self.repeat
    }

    /// The budget in seconds, as the sidecars record it.
    pub fn cell_budget_s(&self) -> Option<f64> {
        self.cell_budget.map(|budget| budget.as_secs_f64())
    }
}

/// Runs the stop rule over a sequence of per-sample wall times (warm-up
/// first), returning how many timed samples the cell takes.
pub fn samples_taken(plan: &SamplingPlan, run_times: &[Duration]) -> usize {
    let mut elapsed = Duration::ZERO;
    for (index, time) in run_times.iter().enumerate() {
        elapsed += *time;
        if index == 0 {
            continue;
        }
        if plan.stop_after(index, elapsed) {
            return index;
        }
    }
    run_times.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(values: &[f64]) -> Vec<Duration> {
        values.iter().map(|v| Duration::from_secs_f64(*v)).collect()
    }

    #[test]
    fn no_budget_takes_exactly_repeat() {
        let plan = SamplingPlan::new(25, None, 7).unwrap();
        assert_eq!(samples_taken(&plan, &secs(&[100.0; 40])), 25);
        assert!(!plan.stopped_early(25));
    }

    #[test]
    fn stops_once_budget_spent_past_the_floor() {
        let plan = SamplingPlan::new(25, Some(10.0), 7).unwrap();
        // warm-up 1 s, then 1 s per sample: elapsed reaches 10 s after sample 9.
        assert_eq!(samples_taken(&plan, &secs(&[1.0; 40])), 9);
        assert!(plan.stopped_early(9));
    }

    #[test]
    fn warm_up_counts_toward_the_budget() {
        let plan = SamplingPlan::new(25, Some(10.0), 2).unwrap();
        let mut times = vec![8.0];
        times.extend([1.0; 30]);
        assert_eq!(samples_taken(&plan, &secs(&times)), 2);
    }

    #[test]
    fn never_below_the_floor_even_when_the_warm_up_spends_the_budget() {
        let plan = SamplingPlan::new(25, Some(1.0), 7).unwrap();
        let mut times = vec![60.0];
        times.extend([5.0; 30]);
        assert_eq!(samples_taken(&plan, &secs(&times)), 7);
    }

    #[test]
    fn repeat_is_the_ceiling_when_the_budget_is_never_spent() {
        let plan = SamplingPlan::new(25, Some(1_000.0), 7).unwrap();
        assert_eq!(samples_taken(&plan, &secs(&[0.1; 40])), 25);
        assert!(!plan.stopped_early(25));
    }

    #[test]
    fn min_repeat_above_repeat_is_clamped() {
        let plan = SamplingPlan::new(1, Some(0.5), 7).unwrap();
        assert_eq!(plan.min_repeat, 1);
        assert_eq!(samples_taken(&plan, &secs(&[1.0; 5])), 1);
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(SamplingPlan::new(0, None, 7).is_err());
        assert!(SamplingPlan::new(25, None, 0).is_err());
        assert!(SamplingPlan::new(25, Some(0.0), 7).is_err());
        assert!(SamplingPlan::new(25, Some(-1.0), 7).is_err());
        assert!(SamplingPlan::new(25, Some(f64::NAN), 7).is_err());
        assert!(SamplingPlan::new(25, Some(f64::INFINITY), 7).is_err());
    }

    #[test]
    fn records_the_budget_in_seconds() {
        let plan = SamplingPlan::new(25, Some(2.5), 7).unwrap();
        assert_eq!(plan.cell_budget_s(), Some(2.5));
        assert_eq!(
            SamplingPlan::new(25, None, 7).unwrap().cell_budget_s(),
            None
        );
    }
}
