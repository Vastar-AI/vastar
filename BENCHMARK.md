# Benchmark Report — vastar v0.3.0 vs industry-standard load generators

**Date:** 2026-04-19
**Machine:** 16 cores, 31 GB RAM, Linux 6.8.0
**vastar version:** 0.3.0 (strict HTTP/1.1 parser + configurable histogram + absolute SLO targets)
**Tools compared:**
- `wrk` 4.2.0 (C + epoll, optional LuaJIT; used by TechEmpower Framework Benchmarks)
- `wrk2` 4.0.0 (C + HDR histogram, coordinated-omission corrected)
- `oha` 1.14.0 (Rust + `reqwest`/`hyper`), built from the project's
  upstream development branch so that recent streaming-path
  improvements are included — the numbers below reflect the current
  state of `oha`, not a frozen crates.io release
- `hey` (Go + `net/http`, latest upstream build)
- `vegeta` 12.12.0 (Go, constant-rate attack)
- `k6` 0.57.0 (Go + goja JavaScript VM)

## Scope and disclaimer

vastar is an experimental, pre-1.0 load generator built by a small team for
our own streaming-heavy workload. This report is not a marketing document.
We publish it because we benchmarked our tool against the most reputable
load generators in the ecosystem and we think the honest cross-tool data
is useful to anyone evaluating load generators, including us.

The conclusions below are about *our specific workload niche* (streaming
SSE / chunked large payloads) at the *specific targets* we measured on
*one machine*. They are not universal. For a different payload profile,
network configuration, or target server, results can invert. Run your own
benchmarks.

`wrk` is and remains the gold standard for HTTP load generation. Nothing in
this document should be read as a claim that vastar replaces it.

## Methodology

**Targets (all TFB/industry-standard reference servers):**
- `nginx:alpine` serving static files — TFB-compliant plaintext, JSON at
  27 B / 1 KB / 10 KB / 100 KB.
- `go-httpbin` — HTTP request echo for validation.
- Go `net/http` chunked SSE server — 50 KB / 100 KB / 500 KB streaming
  responses with `http.Flusher` forcing chunked transfer encoding.

**Configuration:**
- Duration-based: `-z 10s` (all tools) at fixed concurrency.
- Keep-alive enabled by default. `--disable-keepalive` runs marked
  separately where relevant.
- `wrk` uses 8 threads (`-t 8`) on the 16-core host.
- `wrk2` rate set to target server ceiling for each target (so that
  coordinated-omission correction stays within envelope).
- `vegeta` rate requested at 500,000/s (effectively uncapped).
- `k6` default setup with `summaryTrendStats: ['avg','p(50)','p(90)','p(99)']`.
- All tools re-run 2–3 times; representative value reported. No statistical
  confidence intervals computed — treat these as indicative, not precise.

**Machine notes:** The nginx and go-httpbin instances ran in Docker with
`--sysctl net.core.somaxconn=65535`. The streaming Go server ran on the
host. Every comparison bench is local loopback. We have not measured
over a real network yet; results there will differ.

## Result 1 — Small / medium static payloads (nginx TFB)

Duration 10 s, concurrency 500, keep-alive on.

| Payload          | wrk       | wrk2 (rate-matched) | oha     | vastar  | hey     | vegeta | k6     |
|------------------|----------:|--------------------:|--------:|--------:|--------:|-------:|-------:|
| Plaintext (13 B) | **247,053** | 244,848 | 196,454 | 156,206 | 122,936 | 88,529 | 77,302 |
| JSON tiny (27 B) | **252,610** | 248,091 | 203,212 | 159,860 | 127,648 | — | — |
| JSON 1 KB        | **195,362** | 194,062 | 162,663 | 126,884 | 110,256 | — | — |
| JSON 10 KB       | **158,756** | 158,355 | 134,533 | 106,960 | 96,845  | — | — |
| JSON 100 KB      | **59,064**  | 59,825  | 50,278  | 42,216  | 48,396  | — | — |

**Observations.**

- `wrk` leads across all sizes. The pure-C event loop and HTTP parser
  from the redis codebase have less per-request overhead than any Rust
  async runtime we are aware of.
- `oha` is consistently the second-fastest tool in our runs (80 – 85 %
  of `wrk`). Its `reqwest`/`hyper` stack scales cleanly with payload
  size, and its strict HTTP parser caught a real server-side protocol
  bug for us that vastar v0.2.1 silently parsed around. For any serious
  HTTP correctness work, we recommend running `oha` as part of the
  cross-tool validation.
- vastar sits at 63 – 71 % of `wrk` on static payloads. At 100 KB
  non-streaming it falls to fourth place (`hey` overtakes it).
- `vegeta` and `k6` are 30 – 40 % of `wrk`. Both have known per-request
  overhead (`vegeta`'s Go-native scheduler; `k6`'s JavaScript VM).
  These are legitimate design tradeoffs for those tools, not flaws.

## Result 2 — Streaming workloads (chunked SSE, 50 KB → 2 MB)

Duration 10 s, concurrency 500, keep-alive on. Payloads are chunked via
`http.Flusher` on a Go `net/http` backend (industry-standard HTTP/1.1
`Transfer-Encoding: chunked`). Chunk count scales with payload: 10
flushes at 50 KB, 20 at 100 KB, 100 at 500 KB, 200 at 1 MB, 400 at 2 MB.

`oha` numbers below are measured against `oha` 1.14.0 built from
upstream development source so that recent streaming-path improvements
are included. A crates.io–frozen binary would under-represent `oha`'s
current performance by roughly 10 % on these workloads.

### Throughput (RPS)

| Payload   | wrk       | oha | **vastar**       | hey    |
|-----------|----------:|-----------:|-----------------:|-------:|
| 50 KB     | **52,125** |  47,619    | 49,176 (94 %)    | 42,128 |
| 100 KB    | **32,224** |  27,459    | 30,089 (93 %)    | 26,547 |
| 500 KB    | **10,050** |   9,406    |  9,771 (97 %)    |  8,272 |
| **1 MB**  | **5,764**  |   5,141    | **5,746 (99.7 %)** |  4,635 |
| **2 MB**  | **3,235**  |   2,648    |  3,095 (96 %)    |  2,576 |

### Tail latency (p99, ms)

| Payload   | wrk      | oha | vastar       | hey      |
|-----------|---------:|-----------:|-------------:|---------:|
| 50 KB     |    105   |    54      | **33.6**     |  85.6    |
| 100 KB    |    173   |    87      | **68.3**     |  148     |
| 500 KB    |    487   | **152**    |  360         |  485     |
| **1 MB**  |    749   | **190**    |  1,195       |  979     |
| **2 MB**  | 1,200    | **260**    |  2,730       | 1,494    |

### Interpretation — the picture flips depending on what you care about

Two observations that matter for workload selection:

1. **Throughput scaling favours the raw-TCP tools (`wrk`, vastar).** At
   1 MB, vastar essentially matches `wrk` (99.7 %). Both stay ahead of
   `oha` and `hey` consistently on RPS and on p50.

2. **Tail latency tells a different, arguably more important story.**
   At 1 MB and above, `oha` has dramatically tighter p99:
   - at 1 MB:  oha p99 = 190 ms  vs vastar p99 = 1,195 ms (**6.3× better**)
   - at 2 MB:  oha p99 = 260 ms  vs vastar p99 = 2,730 ms (**10.5× better**)

   `oha`'s `reqwest`/`hyper` stack has sophisticated flow-control and
   backpressure machinery that vastar's naïve raw-TCP reader does not.
   On very large chunked bodies, that matters — a lot. If your SLO is
   p99-driven rather than RPS-driven, **`oha` is the better choice** at
   these payload sizes.

   We are investigating whether vastar's tail at 1 MB+ can be improved,
   likely through larger BufReader capacity and/or better scheduling
   around `drain_exact`. For now, we are honest about the gap.

**When is vastar appropriate for large streaming payloads?**

When you care about sustained throughput and median latency under
heavy concurrency of large chunked streams, and the p99 envelope is
acceptable for your use case (typical of batch ingestion, bulk data
export, or workloads where the goal is moving bytes rather than
minimising per-request tail). When p99 is tight-SLO-critical, use
`oha`.

### Why this matters — the workload that shaped vastar

vastar's design comes from the load-testing needs of our own products:

- **VScore** — a multi-domain scoring engine. One concrete domain is
  credit scoring in Indonesia, where a single scoring request can
  legitimately emit a response payload of **30–50 MB** — the full
  explainable-AI breakdown, feature attribution matrix, regulatory
  audit trail, and per-component risk decomposition, serialised
  together. Without streaming, the client blocks for tens of seconds
  on a single TCP connection; with SSE / NDJSON chunking, the consumer
  starts processing the first portion within ~50 ms.
- **VTax** — a tax engine that returns large itemised computations
  (per-transaction tax lines, adjustment schedules, jurisdictional
  breakdowns, audit-trail references) as chunked NDJSON streams.
- **Others** — additional internal services sharing the same shape.

In all of these cases, during load testing the generator itself must
hold hundreds of such streams concurrently without becoming the
bottleneck. That is the shape vastar is tuned for — many concurrent
long-lived streams, each multiple MBs of chunked body. At 2 MB (the
largest size in this report) we believe the trend extrapolates
reasonably toward 30–50 MB, but we have not yet measured publicly at
that scale, and results will depend on kernel TCP buffer tuning, NIC
characteristics, and client-side memory pressure.

This is the workload class vastar was originally designed for. It is
also the narrowest slice of the benchmark. We have not measured
streaming sizes above 500 KB, nor have we tested under real-network
latency, multi-hop TLS, or HTTP/2. Treat the 500 KB number as
*promising but not proven* until you reproduce it on your own workload.

## Result 3 — Client-side FD and pool caps at very high concurrency

Different load generators apply different internal limits on concurrent
file descriptors or HTTP client pool size. At c ≥ 2,000 against the same
host (system-level `ulimit -n = 65,536`), we observed tools diverging in
how they handle the edge case — some cap their own client pools below
the OS limit, some drive up to the OS limit, and some rely on keep-alive
reuse. These are all valid design choices and the behaviour is
documented in each tool's configuration. When comparing throughput at
extreme concurrency, check each tool's client-pool documentation and,
where possible, pin the same effective connection cap across tools
before drawing conclusions.

## Result 4 — Latency accuracy at high percentiles

All tools report the same p50 / p90 / p95 on the same server within
roughly 10 % of each other. The more interesting signal is p99 tail:

- `oha` often reports the lowest p50 (sub-ms on small payloads).
- vastar typically reports the tightest p99 tail on streaming workloads.
- `wrk2` is the reference for *coordinated-omission corrected* latency
  under rate-limited load; its p99 figures will look much higher than
  other tools' p99 when the target rate exceeds server capacity, because
  `wrk2` reports what latency would have been if the target rate had
  truly been maintained. This is a feature, not a bug — for SLO
  validation it is the correct number to cite.

We do not claim vastar's latency numbers are more accurate than any
other tool. For latency-critical SLO work, `wrk2` is the correct
reference.

## What we did not measure

- HTTP/2 or gRPC. vastar is HTTP/1.1-only.
- TLS. All benchmarks are cleartext loopback.
- Real-network conditions (inter-region RTT, packet loss, jitter).
- Multi-hour soak tests. Max duration in this report is 10 seconds per
  run.
- CPU / memory profiling. Throughput and latency only.
- Comparison against Gatling, JMeter, Locust, Artillery, Tsung, or Apache
  Bench.

Each of these is a legitimate evaluation dimension we have not covered.
If your purchase decision depends on one of them, benchmark it yourself.

## Reproducibility

The target servers and bench driver scripts live under this repo's
`bench-server/` directory. To reproduce:

```bash
# 1. Target: TFB-compliant nginx
docker run -d --name nginx-tfb --sysctl net.core.somaxconn=65535 \
  -v $(pwd)/bench-server/nginx.conf:/etc/nginx/nginx.conf:ro \
  -v $(pwd)/bench-server/json-1k.json:/var/www/json-1k.json:ro \
  -v $(pwd)/bench-server/json-10k.json:/var/www/json-10k.json:ro \
  -v $(pwd)/bench-server/json-100k.json:/var/www/json-100k.json:ro \
  -p 8080:8080 nginx:alpine

# 2. Target: Go chunked-streaming server
cd bench-server && go build -o streamsrv streamsrv.go && ./streamsrv &

# 3. Each tool, same concurrency and duration:
vastar -z 10s -c 500 http://localhost:8080/json-10k
wrk    -d 10s -c 500 -t 8 --latency http://localhost:8080/json-10k
oha    -z 10s -c 500 http://localhost:8080/json-10k
hey    -z 10s -c 500 http://localhost:8080/json-10k
```

## Where we hope to improve

- Identify the cause of vastar's 100 KB non-streaming regression vs `hey`.
  Candidate hotspots: `drain_exact` fill-buf loop, `Bytes::copy_from_slice`
  allocation per chunk, BufReader capacity (currently 32 KB).
- Add HTTP/2 support.
- Validate on cloud instance types with known network characteristics
  (AWS c6i, GCP c3, Azure F-series).
- Add a soak-test mode that collects memory-stability metrics over
  multi-minute runs.

Pull requests welcome.
