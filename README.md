# jude

HTTP load generator. Fast, zero-copy, Rust.

```
$ jude -n 20000 -c 1000 http://localhost:8080/

Summary:
  Total:        0.0374 secs
  Slowest:      0.0089 secs
  Fastest:      0.0000 secs
  Average:      0.0012 secs
  Requests/sec: 534891.02

Latency distribution:
  50% in 0.0008 secs
  95% in 0.0046 secs
  99% in 0.0065 secs

Status code distribution:
  [200] 20000 responses
```

## Why jude

|  | jude | hey | oha |
|---|---|---|---|
| Language | Rust | Go | Rust |
| Binary | **1.2 MB** | 9 MB | 20 MB |
| RPS (c=1000) | **537K** | 107K | 18K |
| Memory (c=1000) | **32 MB** | 80 MB | 41 MB |

jude is a drop-in replacement for [hey](https://github.com/rakyll/hey) with the same CLI flags. At high concurrency (c=500+) it is **3-8x faster** while using **2-4x less memory**.

See [BENCHMARK.md](BENCHMARK.md) for full comparison with hey and oha across 10 concurrency levels and 4 payload sizes.

## Install

```bash
cargo install jude
```

Or build from source:

```bash
git clone https://github.com/Vastar-AI/jude.git
cd jude
cargo build --release
# Binary at ./target/release/jude (1.2 MB)
```

## Usage

```
Usage: jude [OPTIONS] <URL>

Arguments:
  <URL>  Target URL

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
  -o <OUTPUT>                Output type (csv)
      --disable-keepalive    Disable keep-alive
      --disable-compression  Disable compression
      --disable-redirects    Disable redirects
  -h, --help                 Print help
  -V, --version              Print version
```

## Examples

```bash
# Simple GET
jude http://localhost:8080/

# POST with JSON body
jude -m POST -T "application/json" -d '{"key":"value"}' http://localhost:8080/api

# 10000 requests, 500 concurrent
jude -n 10000 -c 500 http://localhost:8080/

# Duration mode: run for 30 seconds
jude -z 30s -c 200 http://localhost:8080/

# Custom headers
jude -H "Authorization: Bearer token" -H "X-Custom: value" http://localhost:8080/

# Basic auth
jude -a user:pass http://localhost:8080/

# Body from file
jude -m POST -T "application/json" -D payload.json http://localhost:8080/api
```

## Architecture

```
                CLI (clap)
                    |
                Coordinator
                    |
        +-----------+-----------+
        |           |           |
    Worker 0    Worker 1    Worker N     <- clamp(C/128, 1, cpus*2)
        |           |           |
   FuturesUnordered x ~128 conns each
        |           |           |
    Raw TCP     Raw TCP     Raw TCP      <- hand-crafted HTTP/1.1
```

**Key design decisions:**

- **Raw TCP** instead of hyper/reqwest. Request bytes are pre-built once and written directly to `TcpStream`. Response headers are parsed synchronously from `BufReader`'s buffer. No HTTP framework overhead.

- **Adaptive worker topology.** Workers scale as `clamp(C/128, 1, cpus*2)`. Each worker manages ~128 connections via `FuturesUnordered`. This keeps tokio scheduler overhead bounded while maximizing I/O parallelism.

- **Pre-connect with rate limiting.** All TCP connections are established before the benchmark starts, limited to 256 concurrent connects to avoid TCP backlog overflow.

- **Zero-copy request sharing.** Request bytes use `bytes::Bytes` (Arc-backed). `Bytes::clone()` is a pointer increment, not a copy.

- **Drain-in-place body reading.** Response bodies are consumed via `fill_buf()` + `consume()` through BufReader's existing 32KB buffer. No per-response allocation.

- **Lock-free progress.** Live progress uses `AtomicU64` counters read at 10 FPS. Workers only do `fetch_add(1, Relaxed)` per request. ASCII-only display, no emoji, no aggressive clear screen.

## Dependencies

```
tokio         — async runtime
bytes         — zero-copy buffers
clap          — CLI parsing
futures-util  — FuturesUnordered for connection multiplexing
```

4 crates. No HTTP framework. No TUI framework.

## License

MIT OR Apache-2.0
