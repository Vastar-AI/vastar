use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// Benchmark configuration parsed from CLI.
pub struct BenchConfig {
    pub uri: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub num_requests: usize,
    pub concurrency: usize,
    pub duration: Option<Duration>,
    pub timeout: Duration,
    pub qps: f64,
    pub disable_keepalive: bool,
}

/// Shared atomic counters for live progress — zero lock, zero contention.
pub struct Progress {
    pub completed: AtomicU64,
    pub errors: AtomicU64,
    pub total_ns: AtomicU64,
}

impl Progress {
    pub fn new() -> Self {
        Self {
            completed: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            total_ns: AtomicU64::new(0),
        }
    }
}

/// Per-worker result — local Vec, no channel, no lock on hot path.
pub struct WorkerResult {
    pub latencies: Vec<u64>,
    pub status_codes: Vec<u16>,
    pub errors: u64,
    pub bytes_recv: u64,
}

/// Run the benchmark. Returns all worker results + wall clock elapsed.
pub async fn run(config: BenchConfig) -> (Vec<WorkerResult>, Duration) {
    let progress = Arc::new(Progress::new());
    let stop = Arc::new(AtomicBool::new(false));
    let is_duration_mode = config.duration.is_some();
    let total_display = if is_duration_mode { 0 } else { config.num_requests };

    // Spawn progress renderer (reads atomics, zero lock)
    let prog = progress.clone();
    let stop_r = stop.clone();
    let render_handle = tokio::spawn(async move {
        crate::report::render_progress(prog, total_display, is_duration_mode, stop_r).await;
    });

    // Duration timer — sets stop flag when time is up
    if let Some(dur) = config.duration {
        let s = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(dur).await;
            s.store(true, Ordering::Release);
        });
    }

    // Build hyper client — connection pool sized to concurrency
    let idle = if config.disable_keepalive { 0 } else { config.concurrency };
    let client = Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(idle)
        .build_http::<Full<Bytes>>();

    // Parse once, share via Arc clone (zero-copy for Bytes body)
    let method: hyper::Method = config.method.parse().expect("invalid HTTP method");
    let uri: hyper::Uri = config.uri.parse().expect("invalid URI");
    let host = uri.authority().map(|a| a.to_string()).unwrap_or_default();

    // Distribute requests evenly across workers
    let c = config.concurrency;
    let base = if is_duration_mode { usize::MAX } else { config.num_requests / c };
    let extra = if is_duration_mode { 0 } else { config.num_requests % c };

    let start = Instant::now();
    let mut handles = Vec::with_capacity(c);

    for i in 0..c {
        let n = if is_duration_mode { usize::MAX } else { base + if i < extra { 1 } else { 0 } };
        let client = client.clone();
        let uri = uri.clone();
        let method = method.clone();
        let headers = config.headers.clone();
        let body = config.body.clone(); // Bytes::clone = Arc increment, zero copy
        let timeout = config.timeout;
        let qps = config.qps;
        let progress = progress.clone();
        let stop = stop.clone();
        let disable_keepalive = config.disable_keepalive;
        let host = host.clone();

        handles.push(tokio::spawn(async move {
            let cap = n.min(100_000);
            let mut latencies = Vec::with_capacity(cap);
            let mut status_codes = Vec::with_capacity(cap);
            let mut errors = 0u64;
            let mut bytes_recv = 0u64;

            let throttle_dur = if qps > 0.0 {
                Some(Duration::from_secs_f64(1.0 / qps))
            } else {
                None
            };

            for _ in 0..n {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                if let Some(d) = throttle_dur {
                    tokio::time::sleep(d).await;
                }

                // Build request — body.clone() is zero-copy (Bytes = Arc<[u8]>)
                let mut builder = hyper::Request::builder()
                    .method(method.clone())
                    .uri(uri.clone());

                // Set host header (required for HTTP/1.1)
                if !host.is_empty() {
                    builder = builder.header("host", host.as_str());
                }

                for (k, v) in &headers {
                    builder = builder.header(k.as_str(), v.as_str());
                }

                if disable_keepalive {
                    builder = builder.header("connection", "close");
                }

                let req = match builder.body(Full::new(body.clone())) {
                    Ok(r) => r,
                    Err(_) => {
                        errors += 1;
                        progress.completed.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                };

                let t0 = Instant::now();

                let result = tokio::time::timeout(timeout, async {
                    let resp = client.request(req).await.map_err(|e| e.to_string())?;
                    let status = resp.status().as_u16();
                    let collected = resp.into_body().collect().await.map_err(|e| e.to_string())?;
                    let size = collected.to_bytes().len() as u64;
                    Ok::<_, String>((status, size))
                })
                .await;

                let elapsed_ns = t0.elapsed().as_nanos() as u64;

                match result {
                    Ok(Ok((status, size))) => {
                        latencies.push(elapsed_ns);
                        status_codes.push(status);
                        bytes_recv += size;
                    }
                    _ => {
                        errors += 1;
                    }
                }

                progress.completed.fetch_add(1, Ordering::Relaxed);
                progress.total_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
            }

            WorkerResult {
                latencies,
                status_codes,
                errors,
                bytes_recv,
            }
        }));
    }

    // Collect — merge happens once at the end, not per-request
    let mut results = Vec::with_capacity(c);
    for h in handles {
        results.push(h.await.unwrap());
    }

    let elapsed = start.elapsed();

    // Stop renderer
    stop.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = render_handle.abort();

    (results, elapsed)
}
