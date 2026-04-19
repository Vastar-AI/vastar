use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct BenchConfig {
    pub uri: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
    pub num_requests: usize,
    pub concurrency: usize,
    pub duration: Option<Duration>,
    pub timeout: Duration,
    #[allow(dead_code)]
    pub qps: f64,
    pub disable_keepalive: bool,
}

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

pub struct WorkerResult {
    pub latencies: Vec<u64>,
    pub status_codes: Vec<u16>,
    pub errors: u64,
    pub bytes_recv: u64,
    // Phase timing accumulators (sum/min/max in nanoseconds)
    pub write: PhaseAcc,
    pub wait: PhaseAcc,
    pub read: PhaseAcc,
}

/// Accumulates min/max/sum for a timing phase without storing per-request data.
pub struct PhaseAcc {
    pub sum: u64,
    pub min: u64,
    pub max: u64,
    pub count: u64,
}

impl PhaseAcc {
    pub fn new() -> Self {
        Self { sum: 0, min: u64::MAX, max: 0, count: 0 }
    }
    #[inline]
    pub fn record(&mut self, ns: u64) {
        self.sum += ns;
        if ns < self.min { self.min = ns; }
        if ns > self.max { self.max = ns; }
        self.count += 1;
    }
}

/// Timing breakdown from a single request.
pub struct RequestTimings {
    pub write_ns: u64,
    pub wait_ns: u64,
    pub read_ns: u64,
}

// ---------------------------------------------------------------------------
// URL parsing + raw request builder (unchanged)
// ---------------------------------------------------------------------------

fn parse_url(url: &str) -> (String, u16, String) {
    let s = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match s.find('/') {
        Some(i) => (&s[..i], s[i..].to_string()),
        None => (s, "/".to_string()),
    };
    let (host, port) = match host_port.rfind(':') {
        Some(i) => (&host_port[..i], host_port[i + 1..].parse().unwrap_or(80)),
        None => (host_port, 80u16),
    };
    (host.to_string(), port, path)
}

fn build_raw_request(
    method: &str, host: &str, port: u16, path: &str,
    headers: &[(String, String)], body: &[u8], keepalive: bool,
) -> Bytes {
    let mut buf = Vec::with_capacity(512 + body.len());
    buf.extend_from_slice(method.as_bytes());
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(path.as_bytes());
    buf.extend_from_slice(b" HTTP/1.1\r\n");
    buf.extend_from_slice(b"Host: ");
    buf.extend_from_slice(host.as_bytes());
    if port != 80 {
        buf.push(b':');
        buf.extend_from_slice(port.to_string().as_bytes());
    }
    buf.extend_from_slice(b"\r\n");
    if !body.is_empty() {
        buf.extend_from_slice(b"Content-Length: ");
        buf.extend_from_slice(body.len().to_string().as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    if keepalive {
        buf.extend_from_slice(b"Connection: keep-alive\r\n");
    } else {
        buf.extend_from_slice(b"Connection: close\r\n");
    }
    for (k, v) in headers {
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(v.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
    buf.extend_from_slice(b"\r\n");
    buf.extend_from_slice(body);
    Bytes::from(buf)
}

// ---------------------------------------------------------------------------
// Coordinator
// ---------------------------------------------------------------------------

pub async fn run(config: BenchConfig) -> (Vec<WorkerResult>, Duration) {
    let progress = Arc::new(Progress::new());
    let stop = Arc::new(AtomicBool::new(false));
    let is_duration_mode = config.duration.is_some();
    let total_display = if is_duration_mode { 0 } else { config.num_requests };

    let prog = progress.clone();
    let stop_r = stop.clone();
    let render_handle = tokio::spawn(async move {
        crate::report::render_progress(prog, total_display, is_duration_mode, stop_r).await;
    });

    if let Some(dur) = config.duration {
        let s = stop.clone();
        tokio::spawn(async move {
            tokio::time::sleep(dur).await;
            s.store(true, Ordering::Release);
        });
    }

    let (host, port, path) = parse_url(&config.uri);
    let addr: SocketAddr = tokio::net::lookup_host(format!("{}:{}", host, port))
        .await
        .expect("DNS lookup failed")
        .next()
        .expect("no addresses found");

    let request_bytes = build_raw_request(
        &config.method, &host, port, &path,
        &config.headers, &config.body, !config.disable_keepalive,
    );

    // Adaptive worker topology — smooth scaling, no cliff.
    //
    //   workers = clamp(C / 128, 1, cpus * 2)
    //
    // Each worker manages ~128 conns via FuturesUnordered.
    // Workers scale from 1 → cpus*2 as concurrency grows.
    // cpus*2 cap keeps tokio scheduler overhead bounded.
    //
    // 16-core example:
    // | C     | workers | conns/worker |
    // |-------|---------|--------------|
    // | 50    | 1       | 50           |
    // | 200   | 2       | 100          |
    // | 500   | 4       | 125          |
    // | 1000  | 8       | 125          |
    // | 2000  | 16      | 125          |
    // | 5000  | 32      | 156          |
    // | 10000 | 32      | 312          |
    const TARGET_CONNS: usize = 128;
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let max_workers = cpus * 2;
    let num_workers = (config.concurrency / TARGET_CONNS)
        .max(1)
        .min(max_workers)
        .min(config.concurrency);

    let c = config.concurrency;

    // Phase 0: Pre-connect ALL connections in parallel.
    // Rate-limited to 256 simultaneous connects to avoid TCP backlog overflow.
    let connect_limit = c.min(256);
    let sem = Arc::new(tokio::sync::Semaphore::new(connect_limit));
    let mut connect_futs = FuturesUnordered::new();
    for _ in 0..c {
        let sem = sem.clone();
        let timeout = config.timeout;
        connect_futs.push(async move {
            let _permit = sem.acquire().await.ok()?;
            match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
                Ok(Ok(stream)) => {
                    let _ = stream.set_nodelay(true);
                    Some(BufReader::with_capacity(32768, stream))
                }
                _ => None,
            }
        });
    }
    let mut all_conns: Vec<BufReader<TcpStream>> = Vec::with_capacity(c);
    let mut connect_failures = 0u64;
    while let Some(result) = connect_futs.next().await {
        if let Some(conn) = result {
            all_conns.push(conn);
        } else {
            connect_failures += 1;
        }
    }
    drop(connect_futs);

    // If ALL connections failed, return immediately with error count
    if all_conns.is_empty() {
        stop.store(true, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(150)).await;
        let _ = render_handle.abort();
        return (vec![WorkerResult {
            latencies: vec![], status_codes: vec![],
            errors: connect_failures,
            bytes_recv: 0,
            write: PhaseAcc::new(), wait: PhaseAcc::new(), read: PhaseAcc::new(),
        }], Duration::ZERO);
    }

    // Distribute connections round-robin to workers
    let mut worker_conns: Vec<Vec<BufReader<TcpStream>>> =
        (0..num_workers).map(|_| Vec::new()).collect();
    for (i, conn) in all_conns.into_iter().enumerate() {
        worker_conns[i % num_workers].push(conn);
    }

    let total_reqs = config.num_requests;
    let reqs_base = if is_duration_mode { usize::MAX } else { total_reqs / num_workers };
    let reqs_extra = if is_duration_mode { 0 } else { total_reqs % num_workers };

    let start = Instant::now();
    let mut handles = Vec::with_capacity(num_workers);

    for i in 0..num_workers {
        let nr = if is_duration_mode {
            usize::MAX
        } else {
            reqs_base + if i < reqs_extra { 1 } else { 0 }
        };
        let conns = std::mem::take(&mut worker_conns[i]);

        let rb = request_bytes.clone();
        let progress = progress.clone();
        let stop = stop.clone();
        let timeout = config.timeout;
        let keepalive = !config.disable_keepalive;

        handles.push(tokio::spawn(async move {
            core_worker(addr, rb, conns, nr, timeout, progress, stop, keepalive).await
        }));
    }

    let mut results = Vec::with_capacity(num_workers);
    for h in handles {
        results.push(h.await.unwrap());
    }

    let elapsed = start.elapsed();
    stop.store(true, Ordering::Release);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _ = render_handle.abort();

    (results, elapsed)
}

// ---------------------------------------------------------------------------
// Core worker — one per CPU core, manages C/N connections via event loop
// ---------------------------------------------------------------------------

async fn core_worker(
    addr: SocketAddr,
    request_bytes: Bytes,
    pre_conns: Vec<BufReader<TcpStream>>,
    num_requests: usize,
    timeout: Duration,
    progress: Arc<Progress>,
    stop: Arc<AtomicBool>,
    keepalive: bool,
) -> WorkerResult {
    let cap = num_requests.min(100_000);
    let mut latencies = Vec::with_capacity(cap);
    let mut status_codes = Vec::with_capacity(cap);
    let mut errors = 0u64;
    let mut bytes_recv = 0u64;
    let mut requests_sent = 0usize;
    let mut write_acc = PhaseAcc::new();
    let mut wait_acc = PhaseAcc::new();
    let mut read_acc = PhaseAcc::new();

    let mut in_flight: FuturesUnordered<_> = FuturesUnordered::new();

    // Seed: launch first request on each pre-established connection.
    // Connections are already open — zero connect overhead in benchmark phase.
    for conn in pre_conns {
        if requests_sent >= num_requests || stop.load(Ordering::Relaxed) {
            break;
        }
        requests_sent += 1;
        in_flight.push(do_one_request(addr, Some(conn), request_bytes.clone(), timeout, keepalive));
    }

    // Event loop: as each request completes, send next on same connection.
    // FuturesUnordered polls all in-flight futures within THIS single task —
    // tokio scheduler only sees 1 task, not num_conns tasks.
    while let Some((conn_opt, result)) = in_flight.next().await {
        match result {
            Ok((status, size, latency_ns, timings)) => {
                latencies.push(latency_ns);
                status_codes.push(status);
                bytes_recv += size;
                write_acc.record(timings.write_ns);
                wait_acc.record(timings.wait_ns);
                read_acc.record(timings.read_ns);
                progress.total_ns.fetch_add(latency_ns, Ordering::Relaxed);
            }
            Err(_) => {
                errors += 1;
            }
        }
        progress.completed.fetch_add(1, Ordering::Relaxed);

        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Send next request, reusing connection if available
        if requests_sent < num_requests {
            requests_sent += 1;
            in_flight.push(do_one_request(
                addr, conn_opt, request_bytes.clone(), timeout, keepalive,
            ));
        }
    }

    WorkerResult {
        latencies, status_codes, errors, bytes_recv,
        write: write_acc, wait: wait_acc, read: read_acc,
    }
}

// ---------------------------------------------------------------------------
// Single request cycle: connect (if needed) → write → read → return conn
// ---------------------------------------------------------------------------

async fn do_one_request(
    addr: SocketAddr,
    existing_conn: Option<BufReader<TcpStream>>,
    request_bytes: Bytes,
    timeout: Duration,
    keepalive: bool,
) -> (Option<BufReader<TcpStream>>, Result<(u16, u64, u64, RequestTimings), ()>) {
    let t0 = Instant::now();

    let result = tokio::time::timeout(timeout, async {
        // Get or create connection
        let mut rdr = match existing_conn {
            Some(c) => c,
            None => {
                let stream = TcpStream::connect(addr).await.map_err(|_| ())?;
                let _ = stream.set_nodelay(true);
                BufReader::with_capacity(32768, stream)
            }
        };

        // Phase 1: req write
        let tw = Instant::now();
        rdr.get_mut().write_all(&request_bytes).await.map_err(|_| ())?;
        let write_ns = tw.elapsed().as_nanos() as u64;

        // Phase 2: resp wait (time to first byte) + Phase 3: resp read
        let (status, size, wait_ns, read_ns) = read_response_timed(&mut rdr).await?;
        let latency = t0.elapsed().as_nanos() as u64;

        let timings = RequestTimings { write_ns, wait_ns, read_ns };
        let conn_out = if keepalive { Some(rdr) } else { None };
        Ok::<_, ()>((conn_out, status, size, latency, timings))
    })
    .await;

    match result {
        Ok(Ok((conn, status, size, latency, timings))) => {
            (conn, Ok((status, size, latency, timings)))
        }
        _ => (None, Err(())),
    }
}

// ---------------------------------------------------------------------------
// HTTP/1.1 response parser — synchronous parse from BufReader buffer
// ---------------------------------------------------------------------------

/// Read response with timing: returns (status, size, wait_ns, read_ns).
/// wait_ns = time to first byte; read_ns = time to parse + drain body.
///
/// Header + chunk-size parsing uses `read_until(b'\n', ...)` which correctly
/// handles lines spanning multiple `fill_buf` batches (TCP packet boundaries).
/// Prior implementation used `fill_buf` + `find_header_end` / `position('\n')`
/// and, when the delimiter was missing, incorrectly `consume(len)` + bailed —
/// discarding partial header/chunk-size bytes and reporting spurious errors
/// on SSE keep-alive streams (where chunk-size lines are tiny and frequently
/// get cut at buffer boundaries).
async fn read_response_timed(
    reader: &mut BufReader<TcpStream>,
) -> Result<(u16, u64, u64, u64), ()> {
    // Phase 1: wait for first line (status line) → TTFB
    let tw = Instant::now();
    let mut line_buf = Vec::with_capacity(128);
    let n = reader.read_until(b'\n', &mut line_buf).await.map_err(|_| ())?;
    if n == 0 { return Err(()); }
    let wait_ns = tw.elapsed().as_nanos() as u64;

    // Phase 2: parse status line + remaining headers (strict)
    //
    // Status line layout (HTTP/1.1 200 OK):
    //   H T T P / 1 . 1 SP 2 0 0 SP O K \r \n
    //   0 1 2 3 4 5 6 7 8  9 10 11 12 ...
    let tr = Instant::now();
    if line_buf.len() < 13 || !line_buf.starts_with(b"HTTP/1.") {
        return Err(());
    }
    // Minor version at index 7 must be '0' or '1'
    if !(line_buf[7] == b'0' || line_buf[7] == b'1') { return Err(()); }
    if line_buf[8] != b' ' { return Err(()); }
    // Status code: 3 ASCII digits at positions 9..12
    let status_bytes = &line_buf[9..12];
    if !status_bytes.iter().all(|b| b.is_ascii_digit()) { return Err(()); }
    let status: u16 = std::str::from_utf8(status_bytes)
        .map_err(|_| ())?
        .parse()
        .map_err(|_| ())?;

    let mut content_length: Option<usize> = None;
    let mut is_chunked = false;

    loop {
        line_buf.clear();
        let n = reader.read_until(b'\n', &mut line_buf).await.map_err(|_| ())?;
        if n == 0 { return Err(()); }
        if line_buf.as_slice() == b"\r\n" || line_buf.as_slice() == b"\n" {
            break;
        }
        let line = line_buf.as_slice();
        // Strict: header lines must contain a colon.
        if !line.contains(&b':') { return Err(()); }
        if line.len() > 16 && starts_with_ci(line, b"content-length:") {
            let val = std::str::from_utf8(&line[15..]).map_err(|_| ())?.trim();
            content_length = Some(val.parse().map_err(|_| ())?);
        } else if line.len() > 19 && starts_with_ci(line, b"transfer-encoding:") {
            let val = std::str::from_utf8(&line[18..]).map_err(|_| ())?.trim();
            if val.eq_ignore_ascii_case("chunked") {
                is_chunked = true;
            }
        }
    }

    // Phase 3: drain body
    let size = if let Some(cl) = content_length {
        drain_exact(reader, cl).await?;
        cl as u64
    } else if is_chunked {
        drain_chunked(reader).await?
    } else {
        0
    };
    let read_ns = tr.elapsed().as_nanos() as u64;
    Ok((status, size, wait_ns, read_ns))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[inline]
fn starts_with_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack[..needle.len()]
            .iter()
            .zip(needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
}

async fn drain_exact(
    reader: &mut BufReader<TcpStream>,
    mut remaining: usize,
) -> Result<(), ()> {
    while remaining > 0 {
        let buf = reader.fill_buf().await.map_err(|_| ())?;
        if buf.is_empty() { return Err(()); }
        let take = remaining.min(buf.len());
        reader.consume(take);
        remaining -= take;
    }
    Ok(())
}

async fn drain_chunked(reader: &mut BufReader<TcpStream>) -> Result<u64, ()> {
    let mut total = 0u64;
    loop {
        let chunk_size = read_chunk_size(reader).await?;
        if chunk_size == 0 {
            // Final chunk — consume empty trailer line (CRLF or just LF).
            // Strict: must be exactly an empty line; any other content is a
            // protocol violation (we don't support HTTP/1.1 trailers).
            let mut trailer = Vec::with_capacity(8);
            let n = reader.read_until(b'\n', &mut trailer).await.map_err(|_| ())?;
            if n == 0 { return Err(()); }
            if trailer.as_slice() != b"\r\n" && trailer.as_slice() != b"\n" {
                return Err(());
            }
            break;
        }
        total += chunk_size as u64;
        drain_exact(reader, chunk_size + 2).await?; // body + trailing CRLF
    }
    Ok(total)
}

async fn read_chunk_size(reader: &mut BufReader<TcpStream>) -> Result<usize, ()> {
    // STRICT: returns Err on non-hex chunk size line. The old `unwrap_or(0)`
    // fallback silently treated corruption as "last chunk", masking upstream
    // protocol bugs (e.g., a stray `HTTP/1.1 200 OK` inside the body).
    let mut line = Vec::with_capacity(32);
    let n = reader.read_until(b'\n', &mut line).await.map_err(|_| ())?;
    if n == 0 { return Err(()); }
    let hex_str = std::str::from_utf8(&line)
        .map_err(|_| ())?
        .trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ')
        .split(';')  // allow chunk extensions
        .next()
        .ok_or(())?;
    if hex_str.is_empty() { return Err(()); }
    // Must be pure hex digits, nothing else.
    if !hex_str.bytes().all(|b| b.is_ascii_hexdigit()) { return Err(()); }
    usize::from_str_radix(hex_str, 16).map_err(|_| ())
}

#[allow(dead_code)]
async fn skip_line(reader: &mut BufReader<TcpStream>) -> Result<(), ()> {
    let mut sink = Vec::with_capacity(8);
    reader.read_until(b'\n', &mut sink).await.map_err(|_| ())?;
    Ok(())
}
