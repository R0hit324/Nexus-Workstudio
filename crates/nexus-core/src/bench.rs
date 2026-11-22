//! Lightweight benchmark infrastructure for Nexus (Part 13).
//!
//! Provides named, report-producing benchmarks over `FnMut` workloads. The
//! reports feed the Part 13 performance loop: measure, profile, optimize,
//! benchmark again, and detect regressions.

/// A named benchmark report produced by [`Benchmark::run`].
#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkReport {
    /// Name of the benchmark.
    pub name: String,
    /// Number of iterations executed.
    pub iterations: u64,
    /// Total wall-clock duration of the run, in milliseconds.
    pub total_duration_ms: u64,
    /// Observed throughput: completed iterations per second.
    pub ops_per_second: f64,
}

/// A named benchmark over a repeatable workload.
#[derive(Debug, Clone)]
pub struct Benchmark {
    name: String,
}

impl Benchmark {
    /// Creates a named benchmark.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// The benchmark's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Runs `f` `iterations` times and returns a [`BenchmarkReport`].
    pub fn run(&self, mut f: impl FnMut() -> u64, iterations: u64) -> BenchmarkReport {
        let started = std::time::Instant::now();
        let mut completed = 0u64;
        while completed < iterations {
            f();
            completed += 1;
        }
        let total_duration_ms = started.elapsed().as_millis() as u64;
        BenchmarkReport {
            name: self.name.clone(),
            iterations: completed,
            total_duration_ms,
            ops_per_second: if total_duration_ms == 0 {
                0.0
            } else {
                completed as f64 / (total_duration_ms as f64 / 1000.0)
            },
        }
    }
}

/// Detects whether a `current` benchmark run regressed against a `baseline`.
///
/// Returns `true` when the current throughput fell below the baseline by more
/// than `max_regression` (a fraction, e.g. `0.1` for a 10% budget).
#[must_use]
pub fn is_regressed(
    baseline: &BenchmarkReport,
    current: &BenchmarkReport,
    max_regression: f64,
) -> bool {
    if baseline.ops_per_second <= 0.0 || current.ops_per_second <= 0.0 {
        return false;
    }
    current.ops_per_second < baseline.ops_per_second * (1.0 - max_regression)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_reports_iterations_and_throughput() {
        let benchmark = Benchmark::new("nexus.test");
        let counter = std::sync::atomic::AtomicU64::new(0);
        let counter_ref = std::sync::Arc::new(counter);

        let report = benchmark.run(
            || {
                counter_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                1
            },
            10,
        );

        assert_eq!(report.name, "nexus.test");
        assert_eq!(report.iterations, 10);
        assert!(report.ops_per_second > 0.0);
    }

    #[test]
    fn benchmark_with_zero_iterations_reports_zeroed_metrics() {
        let benchmark = Benchmark::new("nexus.empty");
        let report = benchmark.run(|| 0, 0);
        assert_eq!(report.iterations, 0);
        assert_eq!(report.ops_per_second, 0.0);
    }

    #[test]
    fn regression_detected_when_throughput_drops() {
        let baseline = BenchmarkReport {
            name: String::from("nexus.event_publish"),
            iterations: 10_000,
            total_duration_ms: 100,
            ops_per_second: 100_000.0,
        };
        let current = BenchmarkReport {
            name: String::from("nexus.event_publish"),
            iterations: 10_000,
            total_duration_ms: 200,
            ops_per_second: 50_000.0,
        };
        assert!(is_regressed(&baseline, &current, 0.0));
    }

    #[test]
    fn no_regression_within_budget() {
        let baseline = BenchmarkReport {
            name: String::from("nexus.event_publish"),
            iterations: 10_000,
            total_duration_ms: 100,
            ops_per_second: 100_000.0,
        };
        let current = BenchmarkReport {
            name: String::from("nexus.event_publish"),
            iterations: 10_000,
            total_duration_ms: 101,
            ops_per_second: 99_000.0,
        };
        assert!(!is_regressed(&baseline, &current, 0.05));
    }
}
