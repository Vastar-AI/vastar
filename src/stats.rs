use std::collections::HashMap;
use std::time::Duration;

use crate::engine::WorkerResult;

pub struct BenchResult {
    pub total_duration: Duration,
    pub total_requests: usize,
    pub total_errors: u64,
    /// Requests that were in-flight when drain_cap expired past the
    /// duration deadline. Counted separately from transport errors.
    /// Always 0 outside duration mode.
    pub total_drain_aborted: u64,
    /// Time spent past the user-requested duration, draining in-flight
    /// requests. `Some(d)` in duration mode, `None` in n-based mode.
    pub drain_duration: Option<Duration>,
    pub total_bytes: u64,
    pub rps: f64,
    pub avg_latency: f64,
    pub min_latency: f64,
    pub max_latency: f64,
    pub concurrency: usize,
    pub percentiles: Percentiles,
    pub histogram: Vec<HistBucket>,
    pub status_dist: HashMap<u16, usize>,
    pub details: PhaseDetails,
}

#[derive(Clone)]
pub struct Percentiles {
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    pub p999: f64,
    pub p9999: f64,
}

pub struct HistBucket {
    pub mark: f64,
    pub count: usize,
}

/// Phase timing breakdown — matches hey's "Details" section.
pub struct PhaseDetails {
    pub req_write: PhaseStat,
    pub resp_wait: PhaseStat,
    pub resp_read: PhaseStat,
}

pub struct PhaseStat {
    pub avg: f64,
    pub min: f64,
    pub max: f64,
}

pub fn aggregate(
    results: Vec<WorkerResult>,
    elapsed: Duration,
    concurrency: usize,
) -> BenchResult {
    // Default 16 bins — slightly higher than hey/oha (11) for better
    // tail visibility without cluttering the output. Override per run
    // via `aggregate_with`.
    aggregate_with(results, elapsed, None, concurrency, 16)
}

pub fn aggregate_with(
    results: Vec<WorkerResult>,
    elapsed: Duration,
    drain_duration: Option<Duration>,
    concurrency: usize,
    hist_bins: usize,
) -> BenchResult {
    let total_errors: u64 = results.iter().map(|r| r.errors).sum();
    let total_drain_aborted: u64 = results.iter().map(|r| r.drain_aborted).sum();
    let total_bytes: u64 = results.iter().map(|r| r.bytes_recv).sum();

    let cap: usize = results.iter().map(|r| r.latencies.len()).sum();
    let mut all_latencies = Vec::with_capacity(cap);
    let mut status_dist: HashMap<u16, usize> = HashMap::new();

    // Merge phase timing accumulators
    let mut w_sum = 0u64; let mut w_min = u64::MAX; let mut w_max = 0u64;
    let mut t_sum = 0u64; let mut t_min = u64::MAX; let mut t_max = 0u64;
    let mut r_sum = 0u64; let mut r_min = u64::MAX; let mut r_max = 0u64;
    let mut phase_count = 0u64;

    for r in &results {
        all_latencies.extend_from_slice(&r.latencies);
        for &code in &r.status_codes {
            *status_dist.entry(code).or_insert(0) += 1;
        }
        if r.write.count > 0 {
            w_sum += r.write.sum; w_min = w_min.min(r.write.min); w_max = w_max.max(r.write.max);
            t_sum += r.wait.sum;  t_min = t_min.min(r.wait.min);  t_max = t_max.max(r.wait.max);
            r_sum += r.read.sum;  r_min = r_min.min(r.read.min);  r_max = r_max.max(r.read.max);
            phase_count += r.write.count;
        }
    }

    let total_requests = all_latencies.len() + total_errors as usize + total_drain_aborted as usize;

    let ns = |v: u64| v as f64 / 1_000_000_000.0;
    let phase_avg = |sum: u64| if phase_count > 0 { ns(sum / phase_count) } else { 0.0 };
    let phase_min = |min: u64| if min == u64::MAX { 0.0 } else { ns(min) };

    let details = PhaseDetails {
        req_write: PhaseStat { avg: phase_avg(w_sum), min: phase_min(w_min), max: ns(w_max) },
        resp_wait: PhaseStat { avg: phase_avg(t_sum), min: phase_min(t_min), max: ns(t_max) },
        resp_read: PhaseStat { avg: phase_avg(r_sum), min: phase_min(r_min), max: ns(r_max) },
    };

    if all_latencies.is_empty() {
        return BenchResult {
            total_duration: elapsed,
            total_requests,
            total_errors,
            total_drain_aborted,
            drain_duration,
            total_bytes,
            rps: 0.0,
            avg_latency: 0.0,
            min_latency: 0.0,
            max_latency: 0.0,
            concurrency,
            percentiles: Percentiles {
                p10: 0.0, p25: 0.0, p50: 0.0, p75: 0.0,
                p90: 0.0, p95: 0.0, p99: 0.0, p999: 0.0, p9999: 0.0,
            },
            histogram: vec![],
            status_dist,
            details,
        };
    }

    all_latencies.sort_unstable();

    let n = all_latencies.len();
    let min_lat = ns(all_latencies[0]);
    let max_lat = ns(all_latencies[n - 1]);
    let sum: u64 = all_latencies.iter().sum();
    let avg_lat = ns(sum / n as u64);

    let pctl = |p: usize| -> f64 {
        let idx = (p * n / 100).min(n - 1);
        ns(all_latencies[idx])
    };

    // p99.9 and p99.99 need finer resolution than integer percentages
    let pctl_fine = |p_thousandths: usize| -> f64 {
        let idx = (p_thousandths * n / 10000).min(n - 1);
        ns(all_latencies[idx])
    };

    let percentiles = Percentiles {
        p10: pctl(10), p25: pctl(25), p50: pctl(50), p75: pctl(75),
        p90: pctl(90), p95: pctl(95), p99: pctl(99),
        p999: pctl_fine(9990), p9999: pctl_fine(9999),
    };

    let histogram = build_histogram(&all_latencies, min_lat, max_lat, n, hist_bins);
    let rps = total_requests as f64 / elapsed.as_secs_f64();

    BenchResult {
        total_duration: elapsed,
        total_requests, total_errors, total_drain_aborted, drain_duration, total_bytes,
        rps, avg_latency: avg_lat, min_latency: min_lat, max_latency: max_lat,
        concurrency, percentiles, histogram, status_dist, details,
    }
}

fn build_histogram(sorted: &[u64], min_lat: f64, max_lat: f64, n: usize, num_buckets: usize) -> Vec<HistBucket> {
    let num_buckets = num_buckets.max(1);
    let range = max_lat - min_lat;

    if range <= 0.0 {
        return vec![HistBucket { mark: min_lat, count: n }];
    }

    let bucket_size = range / num_buckets as f64;
    let ns_to_sec = |ns: u64| ns as f64 / 1_000_000_000.0;

    let mut counts = vec![0usize; num_buckets];
    for &lat_ns in sorted {
        let lat = ns_to_sec(lat_ns);
        let mut idx = ((lat - min_lat) / bucket_size) as usize;
        if idx >= num_buckets { idx = num_buckets - 1; }
        counts[idx] += 1;
    }

    (0..num_buckets)
        .map(|i| HistBucket {
            mark: min_lat + bucket_size * i as f64,
            count: counts[i],
        })
        .collect()
}
