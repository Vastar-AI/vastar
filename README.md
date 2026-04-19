# vastar

**Status: experimental.** An HTTP load generator written in Rust. Raw TCP,
zero-copy buffer handling, tokio-based async runtime. A newcomer in an
ecosystem that already has battle-tested, reputation-heavy tools — we are
not claiming to replace any of them.

### TL;DR of our benchmarks

- **`wrk` wins throughput** at every payload size we tested.
- **`oha` wins tail latency** — particularly at streaming payloads ≥ 500 KB,
  where its `hyper`-based flow control outperforms everything else we
  measured.
- **vastar does not win either category outright.** Its current niche is
  a narrow one: at 1 MB chunked streaming, vastar matches `wrk` on
  throughput (99.7 %) while offering built-in SLO-coloured histogram,
  live progress display, and an adaptive concurrency finder (`vastar
  sweep`). Outside that niche, use `wrk` or `oha`.

The industry standards are `wrk` / `wrk2` (used by TechEmpower Framework
Benchmarks) for maximum throughput measurement, and `oha` for a beautifully
engineered Rust load generator with strict HTTP parsing that has caught
real protocol bugs for us — see the "Credit where it is due" section
below. For different niches there are also `hey`, `vegeta`, `k6`. If you
need a production-grade load generator today, use one of those. vastar is
a small-team project we ship publicly for the specific workload we
encountered repeatedly: **long-running streaming responses with large
payloads** (SSE token streams, chunked JSON batches, multi-hundred-KB to
multi-MB responses) where we wanted tighter tail-latency measurement and
a bench UX that is comfortable to watch for minutes at a time.

### Why we wrote it

While building AI-gateway services, real-time simulation engines, and
streaming data pipelines, we ran long-duration load tests daily. The
existing tools covered most of our needs. Two small preferences pushed us
to experiment:

- **Live progress feedback on multi-minute runs** — we wanted an
  ASCII-only, minimal-redraw live display that could be left running on
  a remote SSH session without surprises from terminal/font variation on
  our specific stack. This is a preference, not a criticism of any
  existing tool — different teams have different terminal environments.
- **Distribution-aware analysis in the default output** — we wanted the
  p50/p95/p99/p99.9 percentile story and a colored histogram to appear in
  the terminal by default, without flags or post-processing.

Everything else vastar offers — HTTP/1.1 parser, raw TCP client, chunked
streaming reader — we wrote because we needed a substrate for those UX
goals. It turned out to be reasonably fast on streaming workloads
(benchmarks below), but that was a side-effect, not a starting goal.

### What to expect

vastar is pre-1.0, built by a small team, used primarily inside our own
systems. It has unknown bugs. Its parser is strict about HTTP/1.1 and has
surfaced a few edge cases we did not anticipate. We publish our benchmark
methodology and honest cross-tool comparisons so you can decide whether
vastar's tradeoffs fit your use case. See [BENCHMARK.md](BENCHMARK.md) for
the full matrix (wrk, wrk2, oha, hey, vegeta, k6 vs vastar, across payload
sizes from 13 B to 500 KB streaming).

If you are researching load generators for a serious evaluation, start
with `wrk` / `wrk2`. If that pipeline is working for you, there is likely
no reason to switch.

```
$ vastar -n 3000 -c 300 -m POST -T "application/json" \
    -d '{"prompt":"bench"}' http://localhost:3081/api/gw/trigger

Summary:
  Total:        0.5027 secs
  Slowest:      0.0966 secs
  Fastest:      0.0012 secs
  Average:      0.0469 secs
  Requests/sec: 5968.13
  Total data:   6205800 bytes
  Size/request: 2068 bytes

Response time distribution:
  10.00% in 0.0190 secs
  25.00% in 0.0434 secs
  50.00% in 0.0488 secs  (48.81ms)
  75.00% in 0.0567 secs
  90.00% in 0.0649 secs
  95.00% in 0.0706 secs  (70.58ms)
  99.00% in 0.0804 secs  (80.42ms)
  99.90% in 0.0955 secs  (95.52ms)
  99.99% in 0.0966 secs

Response time histogram:        (11-level SLO color gradient)
  0.0012 [107]  ■■
  0.0099 [180]  ■■■
  0.0185 [170]  ■■■
  0.0272 [85]   ■
  0.0359 [350]  ■■■■■■
  0.0445 [1088] ■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■■
  0.0532 [637]  ■■■■■■■■■■■■■■■■■■■■■■■■■■■■
  0.0619 [233]  ■■■■■■■■■■
  0.0706 [117]  ■■■■■
  0.0792 [25]   ■
  0.0879 [8]

  SLO:
  ██ elite      <=21.7ms    ██ excellent  <=43.4ms    ██ good       <=48.8ms
  ██ normal     <=56.7ms    ██ acceptable <=64.9ms    ██ degraded   <=70.6ms
  ██ slow       <=80.4ms    ██ very slow  <=121ms     ██ critical   <=161ms
  ██ severe     <=241ms     ██ violation  >241ms

Status code distribution:
  [200] 3000 responses

Details (average, fastest, slowest):
  req write:    0.0000 secs, 0.0000 secs, 0.0002 secs
  resp wait:    0.0469 secs, 0.0012 secs, 0.0966 secs
  resp read:    0.0000 secs, 0.0000 secs, 0.0000 secs

Insight:
  Latency spread p99/p50 = 1.6x -- good consistency
  Tail ratio p99/p95 = 1.1x -- clean tail
  Outlier ratio p99.9/p99 = 1.2x -- no significant outliers
```

*With SLO color gradient (dark green → red):*

<img src="docs/assets/vastar-bench-output.png" width="50%" />

## Where vastar fits (and where it does not)

Summary of [BENCHMARK.md](BENCHMARK.md). Targets are industry-standard
reference endpoints — `nginx` serving TFB-compliant plaintext/JSON at
various sizes, plus a Go `net/http` chunked SSE server delivering 50 KB
to 2 MB streaming responses (chunk count scales with payload). All
tools tested at c = 500, duration 10 s. Numbers are RPS; higher is
better.

### Throughput

`oha` numbers are from `oha` 1.14.0 built from upstream development
source (streaming-path improvements are ahead of the frozen crates.io
binary), so the figures below reflect its current state.

| Workload               | wrk (gold) | oha        | **vastar** | hey     |
|------------------------|-----------:|-----------:|-----------:|--------:|
| Plaintext (13 B)       | **247,053** | 196,454    | 156,206    | 122,936 |
| JSON 1 KB              | **195,362** | 162,663    | 126,884    | 110,256 |
| JSON 10 KB             | **158,756** | 134,533    | 106,960    | 96,845  |
| JSON 100 KB            | **59,064**  | 50,278     | 42,216     | 48,396  |
| SSE chunked 50 KB      | **52,125**  | 47,619     | 49,176     | 42,128  |
| SSE chunked 100 KB     | **32,224**  | 27,459     | 30,089     | 26,547  |
| SSE chunked 500 KB     | **10,050**  |  9,406     |  9,771     |  8,272  |
| **SSE chunked 1 MB**   | **5,764**   |  5,141     | **5,746** (99.7 %) | 4,635 |
| SSE chunked 2 MB       | **3,235**   |  2,648     |  3,095 (96 %)    |  2,576 |

### Tail latency (p99) on large streaming payloads

| Payload | wrk | oha (⚡) | vastar | hey |
|---|---:|---:|---:|---:|
| 50 KB   | 105 ms | 54 ms | **33.6 ms** | 86 ms |
| 100 KB  | 173 ms | 87 ms | **68.3 ms** | 148 ms |
| 500 KB  | 487 ms | **152 ms** | 360 ms | 485 ms |
| 1 MB    | 749 ms | **190 ms** | 1,195 ms | 979 ms |
| 2 MB    | 1,200 ms | **260 ms** | 2,730 ms | 1,494 ms |

### Honest interpretation

- **`wrk` remains the throughput leader at all sizes** — decades of C +
  `epoll` + hand-tuned HTTP parser. On TFB plaintext, vastar reaches
  ~63 % of `wrk`. `oha` reaches 80 %.
- **At 1 MB streaming, vastar and `wrk` are effectively tied on RPS**
  (99.7 %), and vastar's median latency is competitive. This is the
  niche vastar was built for.
- **But `oha` is the clear tail-latency winner at 500 KB – 2 MB
  streaming.** At 1 MB: oha p99 = 181 ms vs vastar 1.2 s (**6.6×
  better**). At 2 MB: oha 298 ms vs vastar 2.7 s (**9.1× better**). If
  your SLO is p99-driven rather than RPS-driven at these payload sizes,
  **`oha` is the better tool** — its `reqwest`/`hyper` stack has
  sophisticated flow-control and backpressure that vastar's naïve
  raw-TCP reader does not yet have. We are investigating whether this
  can be narrowed in future versions.
- **On tiny payloads (< 100 KB), vastar does not beat any of the
  industry standards.** The C + `epoll` hot path of `wrk` and the
  mature `hyper` client of `oha` both have less per-request overhead
  than our current Rust tokio implementation.
- **Other tools' niches:** `wrk2` is the reference for
  coordinated-omission-corrected latency under fixed rate. `vegeta`
  excels at constant-rate SLO testing with HDR histograms. `k6` has
  JavaScript scripting and rich observability. Each is a deliberate
  design with its own strengths.

### Why we care about big-payload streaming

vastar's design is driven by the load-testing needs of our own
products:

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
- **Others** — additional internal services with the same shape:
  many concurrent long-lived streams, each multi-MB of chunked body.

In all of these cases, during load testing the generator itself must
hold hundreds of such streams concurrently without becoming the
bottleneck. That is what vastar is tuned for, and why we chose raw TCP
over a higher-level HTTP client for this tool.

We recommend vastar for teams whose workload meaningfully resembles
that shape (SSE, chunked large JSON batches, LLM token streams, bulk
NDJSON export) and who can tolerate the current tail-latency envelope
at 1 MB+. For everything else, or when p99 matters more than peak RPS,
use `oha` or `wrk`.

### Known caveats

- The strict HTTP/1.1 parser is a deliberate choice — it surfaces
  protocol anomalies (invalid chunk-size hex, malformed status lines,
  header lines without `:`) that permissive parsers can parse around.
  Permissive parsers are a valid design choice for real-world clients
  that need to interoperate with slightly non-conformant servers; we
  made the other choice because our primary use case is catching bugs
  in systems we ourselves build. **If you run vastar against the same
  target as another load generator and see an error-count discrepancy,
  cross-check with `oha`** — in our experience, when `oha` and vastar
  agree, the error is almost always real on the server side.
- Very large payloads (≥ 100 KB non-streaming, single response) show
  vastar falling behind `hey` in our measurements. We have not yet
  instrumented this path; if it matters for your workload, run your own
  comparison and do not rely on our numbers.
- vastar is pre-1.0. API, output format, and flag semantics may change
  in minor versions until we reach stability.

## Features

- **`vastar sweep` subcommand** — adaptive concurrency sweet-spot finder. Runs multi-point sweep, detects the knee of the throughput-vs-latency curve, disqualifies points with unhealthy tails or excessive errors, and emits both a pretty table and machine-readable JSON for CI/driver consumption. Paired mode (`--vs REFERENCE_URL`) measures platform overhead vs an upstream and picks the `c` where the gateway/proxy/mesh is still transparent. Domain-agnostic: no workload classifications, no CPU-core heuristics — the curve is measured, not guessed.
- **11-level SLO color histogram** — ANSI 256-color gradient (dark green to dark red) mapped to percentile thresholds. SLO levels are relative to the current run's own distribution — not absolute latency targets. Organizations like Google, AWS, and others define custom SLO policies per service (e.g., p99 < 200ms for API, p99 < 50ms for cache). Use vastar's SLO as a visual distribution guide, then define your own thresholds in your monitoring platform.
- **Automated Insight** — latency spread, tail ratio, outlier detection from p50/p95/p99/p99.9
- **Key percentile highlights** — p50, p95, p99, p99.9 annotated with colored (ms) values
- **Phase timing Details** — req write, resp wait, resp read breakdown (like hey)
- **Live progress bar** — ASCII-only, terminal-aware, no emoji, no aggressive clear screen
- **Chunked transfer** — supports Content-Length and Transfer-Encoding: chunked (SSE/streaming)

## Install

```bash
# One-line install (Linux/macOS, auto-detects platform)
curl -sSf https://raw.githubusercontent.com/Vastar-AI/vastar/main/install.sh | sh

# Or via Cargo
cargo install vastar

# Or via Homebrew
brew tap Vastar-AI/tap && brew install vastar
```

Or build from source:

```bash
git clone https://github.com/Vastar-AI/vastar.git
cd vastar
cargo build --release
# Binary at ./target/release/vastar (1.2 MB)
```

## Usage

```
Usage: vastar [OPTIONS] <URL>            # flat bench mode (default)
       vastar <COMMAND> [OPTIONS]        # subcommand mode

Commands:
  sweep  Adaptive concurrency sweep — finds the sweet-spot `c` empirically
  help   Print this message or the help of the given subcommand(s)

Options:
  -n <REQUESTS>              Number of requests [default: 200]
  -c <CONCURRENCY>           Concurrent workers [default: 50]
  -z <DURATION>              Duration (e.g. 10s, 1m). Overrides -n
  -q <QPS>                   Rate limit per worker [default: 0]
  -m <METHOD>                HTTP method [default: GET]
  -d <BODY>                  Request body
  -D <BODY_FILE>             Request body from file
  -T <CONTENT_TYPE>          Content-Type [default: text/html]
  -H <HEADER>                Custom header (repeatable)
  -t <TIMEOUT>               Timeout in seconds [default: 20]
  -A <ACCEPT>                Accept header
  -a <AUTH>                  Basic auth (user:pass)
      --disable-keepalive    Disable keep-alive
      --disable-compression  Disable compression
      --disable-redirects    Disable redirects
      --hist-bins <N>        Histogram bucket count [default: 16]
      --slo-target <SPEC>    Absolute SLO: "p95=100ms,p99=250ms"
  -h, --help                 Print help
  -V, --version              Print version
```

### Strict HTTP/1.1 parser (v0.3.0+) — learned from `oha`

We learned this from `oha`. Before v0.3.0, vastar's parser was
permissive and silently parsed around protocol violations that `oha`
correctly rejected. After watching `oha` flag a real upstream bug that
our tool missed, we studied how its `reqwest`/`hyper` stack validates
responses and tightened vastar's parser to match that level of rigour.
The detailed story is in the "Credit where it is due" note below.

vastar's response parser now rejects protocol violations rather than
parsing around them. The tradeoff is explicit: strict parsing surfaces
upstream bugs early, at the cost of being less forgiving of
non-conformant servers. Different tools make this tradeoff differently
— neither choice is universally better.

- **Invalid chunk-size line** — non-hex digits → error (prior
  `unwrap_or(0)` fallback in our own code silently truncated streams)
- **Bad status line format** — must be `HTTP/1.[01] NNN ...` exactly
- **Malformed header** — line without `:` → error
- **Invalid `Content-Length`** — non-numeric → error
- **Chunked terminator violation** — trailer must be empty line

**Credit where it is due.** The strict parser in vastar v0.3.0 exists
because of `oha`. While profiling a streaming gateway (`vflow_http`) we
built, `oha` was the only tool in our lineup that consistently reported
`error reading a body from connection` — other load generators,
including vastar v0.2.1 at that time, returned 100 % success. The
discrepancy forced us to `tcpdump` the traffic. The server turned out
to be emitting a second `HTTP/1.1 200 OK` on the same connection for a
single request, which `oha`'s `reqwest`/`hyper` parser correctly flagged
as a protocol violation. Our own tool was parsing around it. Once we
had the ground truth, we fixed the server bug and then tightened
vastar's parser to match `oha`'s level of strictness.

We genuinely recommend running `oha` alongside any strict-parsing load
generator when shaking down a new HTTP server. The Rust `hyper` stack
underneath is carefully engineered and has caught bugs in our code that
we would have shipped otherwise.

### Histogram resolution

Default is 11 buckets (SLO-aligned). Use `--hist-bins 32` or `--hist-bins 64`
for finer tail-latency resolution when investigating outliers.

```bash
vastar --hist-bins 32 -n 10000 -c 200 http://localhost:8080/api
```

### Absolute SLO targets

By default, SLO colors (elite/excellent/good/…) are computed from the run's
own percentile distribution. For CI gates or fleet-wide comparable reporting,
specify absolute thresholds:

```bash
vastar --slo-target "p50=20ms,p95=100ms,p99=250ms" \
  -n 5000 -c 200 http://localhost:8080/api
```

Missing percentiles fall back to run-relative. Units: `ms` (default) or `s`.

## Examples

```bash
# Simple GET
vastar http://localhost:8080/

# POST with JSON body
vastar -m POST -T "application/json" -d '{"key":"value"}' http://localhost:8080/api

# 10000 requests, 500 concurrent
vastar -n 10000 -c 500 http://localhost:8080/

# Duration mode: run for 30 seconds
vastar -z 30s -c 200 http://localhost:8080/

# Custom headers
vastar -H "Authorization: Bearer token" -H "X-Custom: value" http://localhost:8080/

# Basic auth
vastar -a user:pass http://localhost:8080/

# Body from file
vastar -m POST -T "application/json" -D payload.json http://localhost:8080/api
```

## `vastar sweep` — Adaptive Concurrency Sweet-Spot Finder

Finding the right `-c` for a given endpoint is usually an operator guess. Too low and throughput is understated; too high and queueing explodes the tail without warning. `vastar sweep` runs multiple concurrency levels against the same URL, detects the knee of the throughput-vs-latency curve, and picks the smallest `c` that still delivers near-peak throughput with a healthy tail.

```bash
# Auto-sweep (log-spaced 10..1000), pick knee at 95% of peak
vastar sweep -n 2000 --repeats 3 http://localhost:8080/api

# Explicit concurrency list
vastar sweep --conc "10,50,100,500,1000" -n 2000 http://localhost:8080/api

# Log-spaced range
vastar sweep --conc "10..1000:log=6" -n 2000 http://localhost:8080/api

# Refine: coarse sweep, then bracket ±50% around winner for finer resolution
vastar sweep --conc auto --refine -n 2000 http://localhost:8080/api

# Emit JSON for script consumption (downstream bench drivers, CI gates)
vastar sweep -o json --conc auto -n 2000 http://localhost:8080/api \
  > /tmp/sweep.json

BENCH_C=$(jq -r .sweet_spot.concurrency /tmp/sweep.json)
```

### Paired mode — platform overhead vs upstream

For gateways, proxies, meshes, or any service that fronts an upstream, a single-endpoint sweep can be misleading — the target's curve can look healthy even when *your platform* is the bottleneck, simply because the upstream is doing the heavy lifting. Paired mode runs both endpoints at each concurrency and picks the sweet spot where the platform still stays close to the upstream's own performance:

```bash
vastar sweep \
  --vs http://localhost:4545/v1/chat/completions \    # reference (upstream)
  --max-overhead-pct 25 \                             # DQ when target p99 >25% of ref
  -m POST -T application/json \
  -d '{"prompt":"bench"}' \
  http://localhost:3080/trigger                       # target (gateway)
```

For sweeping many gateway endpoints against the same upstream, cache the reference once and reuse:

```bash
# Once: cache upstream curve
vastar sweep -o json --conc auto -n 2000 --repeats 3 \
  -m POST -T application/json -d '{"prompt":"bench"}' \
  http://localhost:4545/v1/chat/completions > /tmp/upstream.json

# Many times: reuse for each gateway test, no re-sweep
vastar sweep --ref-from-json /tmp/upstream.json --max-overhead-pct 20 \
  ... http://localhost:3080/trigger
```

### Sweep flags

```
vastar sweep [OPTIONS] <URL>

  --conc <SPEC>              "10,50,100,500" | "10..1000:log=6" | "10..200:step=20" | "auto"
  --refine                   Bracket ±50% around coarse winner for finer resolution
  --repeats <N>              Run each c-level N times, take median [default: 1]

  --pick <knee|score>        Selection algorithm [default: knee]
  --knee-ratio <0.95>        Smallest c reaching this fraction of peak rps
  --baseline-c <1>           Concurrency used as reference for tail-degradation check

  --max-spread <4.0>         DQ if p99/p50 > this
  --max-p999-ratio <8.0>     DQ if p99.9/p50 > this
  --max-tail-mult <3.0>      DQ if p99 > baseline_p99 × this
  --max-errors <0.01>        DQ if error_rate > this (fraction)

  --vs <URL>                 Reference endpoint (paired mode)
  --vs-method / --vs-body / --vs-content-type   Override per-reference
  --ref-from-json <FILE>     Load reference curve from prior sweep JSON
  --max-overhead-pct <25>    DQ if target overhead vs reference > this%
  --max-rps-deficit-pct <50> DQ if target rps deficit vs reference > this%

  -o <text|json|ndjson>      Output format
  --json-path <FILE>         Also write JSON to file

  # Pass-through to each sub-benchmark (same as flat vastar):
  -n / -z / -m / -d / -D / -T / -H / -A / -a / -t / --disable-keepalive / --disable-compression
```

## Architecture

<img src="docs/assets/architecture.svg" width="100%" />

### How it works

1. **Pre-connect phase.** All C connections are established in parallel before the benchmark starts. A semaphore limits to 256 concurrent connects to avoid TCP backlog overflow.

2. **Adaptive worker topology.** Workers scale as `clamp(C/128, 1, cpus*2)`. At c=50 that's 1 worker. At c=500 that's 4 workers. At c=5000 that's 32 workers (capped at cpus*2). Each worker runs a FuturesUnordered event loop managing ~128 connections. The tokio scheduler sees N workers (not C tasks) — drastically less scheduling overhead at high concurrency.

3. **Raw TCP request.** HTTP/1.1 request bytes are pre-built once at startup (`method + path + headers + body`). Each request is a single `write_all()` of pre-built `Bytes` (Arc-backed, zero-copy clone). No per-request allocation, no header map construction, no URI parsing.

4. **Synchronous response parsing.** One `fill_buf()` call gets data into BufReader's 32KB buffer. Headers are parsed synchronously from buffered data (find `\r\n\r\n`, scan for `Content-Length`/`Transfer-Encoding`). Body is drained via `fill_buf()` + `consume()` — no per-response allocation. Chunked transfer encoding is handled inline.

5. **Phase timing.** Each request measures write time, wait-for-first-byte time, and read time separately. These are accumulated per-worker (sum/min/max) and merged once at the end — no per-request timing allocation.

6. **SLO color histogram.** 11 histogram buckets mapped to 11 ANSI 256-color levels via the color cube path: `(0,1,0)→(0,4,0)→(1,5,0)→(5,5,0)→(5,1,0)→(4,0,0)→(2,0,0)` (dark green through yellow to dark red). Color only emitted when stdout is a terminal.

## Dependencies

```
tokio         — async runtime
bytes         — zero-copy buffers
clap          — CLI parsing
futures-util  — FuturesUnordered for connection multiplexing
```

4 crates. No HTTP framework. No TUI framework.

## Roadmap

vastar is currently an HTTP/1.1 load generator. The roadmap expands it into a **multi-protocol load generator** for high-throughput systems.

| Phase | Scope | Status |
|---|---|---|
| **0. Concurrency sweep** | `vastar sweep` — adaptive sweet-spot finder, knee detection, paired-mode overhead comparison, JSON/NDJSON output | **shipped v0.2.0** |
| **1. HTTP parity** | HTTPS/TLS, HTTP/2, proxy, redirects, JSON/CSV output, coordinated omission correction | planned |
| **2. HTTPS + HTTP/2** | rustls, h2 crate, ALPN negotiation, mTLS | planned |
| **3. Multi-protocol** | gRPC, WebSocket, QUIC/HTTP/3, MQTT, NATS, Kafka, AMQP, RSocket, GraphQL, raw TCP/UDP. SSE already supported. | planned |
| **4. Advanced analysis** | Coordinated omission correction, comparative mode, distributed load gen, custom SLO, CI/CD gates | planned |
| **5. Ecosystem** | vastar-cloud, HTML report generator, GitHub Action, IDE plugin | planned |
| **6. AI Engineering** | `vastar ai-bench` — TTFT, TPS, inter-token latency, cost estimation, multi-model, prompt sweep, guardrail overhead | planned |
| **7. Data layer** | `vastar sql` (Postgres/MySQL), `vastar redis`, `vastar vector` (Qdrant/Milvus), `vastar tsdb`, `vastar search`, `vastar graph` | planned |
| **8. Storage & cache** | `vastar s3` (MinIO/S3), `vastar cache`, distributed FS | planned |
| **9. Infrastructure** | `vastar dns`, `vastar gateway`, `vastar mesh`, `vastar serverless`, `vastar edge`, `vastar lb` | planned |
| **10. Emerging** | `vastar blockchain`, `vastar realtime`, `vastar wasm`, `vastar ml`, `vastar media`, `vastar audio` | planned |

30+ subcommands planned, all sharing the same core engine. See [ROADMAP.md](ROADMAP.md) for full details.

See [ROADMAP.md](ROADMAP.md) for full details including feature-by-feature comparison with hey and oha.

## Known Issues

See [ROADMAP.md](ROADMAP.md#known-bugs) for known bugs and workarounds.

## License

MIT OR Apache-2.0
