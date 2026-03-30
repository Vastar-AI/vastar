use std::collections::HashMap;
use std::time::Duration;

use crate::engine::WorkerResult;

pub struct BenchResult {
    pub total_duration: Duration,
    pub total_requests: usize,
    pub total_errors: u64,
    pub total_bytes: u64,
    pub rps: f64,
    pub avg_latency: f64,
    pub min_latency: f64,
    pub max_latency: f64,
    pub percentiles: Percentiles,
    pub histogram: Vec<HistBucket>,
    pub status_dist: HashMap<u16, usize>,
}

pub struct Percentiles {
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
}

pub struct HistBucket {
    pub mark: f64,
    pub count: usize,
    pub frequency: f64,
}

/// Merge all worker results into a single BenchResult.
/// Sort + percentile computation happens ONCE here, not per-request.
pub fn aggregate(results: Vec<WorkerResult>, elapsed: Duration) -> BenchResult {
    let total_errors: u64 = results.iter().map(|r| r.errors).sum();
    let total_bytes: u64 = results.iter().map(|r| r.bytes_recv).sum();

    let cap: usize = results.iter().map(|r| r.latencies.len()).sum();
    let mut all_latencies = Vec::with_capacity(cap);
    let mut status_dist: HashMap<u16, usize> = HashMap::new();

    for r in &results {
        all_latencies.extend_from_slice(&r.latencies);
        for &code in &r.status_codes {
            *status_dist.entry(code).or_insert(0) += 1;
        }
    }

    let total_requests = all_latencies.len() + total_errors as usize;

    if all_latencies.is_empty() {
        return BenchResult {
            total_duration: elapsed,
            total_requests,
            total_errors,
            total_bytes,
            rps: 0.0,
            avg_latency: 0.0,
            min_latency: 0.0,
            max_latency: 0.0,
            percentiles: Percentiles {
                p10: 0.0, p25: 0.0, p50: 0.0, p75: 0.0,
                p90: 0.0, p95: 0.0, p99: 0.0,
            },
            histogram: vec![],
            status_dist,
        };
    }

    // Single sort — O(n log n) once, not per-request
    all_latencies.sort_unstable();

    let ns_to_sec = |ns: u64| ns as f64 / 1_000_000_000.0;
    let n = all_latencies.len();

    let min_lat = ns_to_sec(all_latencies[0]);
    let max_lat = ns_to_sec(all_latencies[n - 1]);
    let sum: u64 = all_latencies.iter().sum();
    let avg_lat = ns_to_sec(sum / n as u64);

    let pctl = |p: usize| -> f64 {
        let idx = (p * n / 100).min(n - 1);
        ns_to_sec(all_latencies[idx])
    };

    let percentiles = Percentiles {
        p10: pctl(10),
        p25: pctl(25),
        p50: pctl(50),
        p75: pctl(75),
        p90: pctl(90),
        p95: pctl(95),
        p99: pctl(99),
    };

    // Histogram — 10 buckets
    let histogram = build_histogram(&all_latencies, min_lat, max_lat, n);

    let rps = total_requests as f64 / elapsed.as_secs_f64();

    BenchResult {
        total_duration: elapsed,
        total_requests,
        total_errors,
        total_bytes,
        rps,
        avg_latency: avg_lat,
        min_latency: min_lat,
        max_latency: max_lat,
        percentiles,
        histogram,
        status_dist,
    }
}

fn build_histogram(sorted: &[u64], min_lat: f64, max_lat: f64, n: usize) -> Vec<HistBucket> {
    let num_buckets = 10;
    let range = max_lat - min_lat;

    if range <= 0.0 {
        return vec![HistBucket {
            mark: min_lat,
            count: n,
            frequency: 1.0,
        }];
    }

    let bucket_size = range / num_buckets as f64;
    let ns_to_sec = |ns: u64| ns as f64 / 1_000_000_000.0;

    let mut counts = vec![0usize; num_buckets];
    for &lat_ns in sorted {
        let lat = ns_to_sec(lat_ns);
        let mut idx = ((lat - min_lat) / bucket_size) as usize;
        if idx >= num_buckets {
            idx = num_buckets - 1;
        }
        counts[idx] += 1;
    }

    (0..num_buckets)
        .map(|i| HistBucket {
            mark: min_lat + bucket_size * i as f64,
            count: counts[i],
            frequency: counts[i] as f64 / n as f64,
        })
        .collect()
}
