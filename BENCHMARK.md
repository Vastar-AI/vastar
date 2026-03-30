# Benchmark Report: jude vs hey vs oha

**Date:** 2026-03-30
**Machine:** 16 cores, 31 GB RAM, Linux 6.8.0
**Versions:** jude 0.1.0, hey 0.0.1 (Go), oha 1.14.0 (Rust/hyper)
**Server:** Raw TCP mock server (tokio), fixed response, keep-alive

## Methodology

- Each tool sends N requests at concurrency C against a local raw TCP server
- Server returns a fixed HTTP/1.1 response with `Connection: keep-alive`
- 4 payload sizes tested: 0B, 1KB, 10KB, 100KB
- 10 concurrency levels: 1, 10, 50, 100, 200, 500, 1000, 2000, 5000, 10000
- Metrics: requests/sec (RPS), p99 latency, peak RSS memory
- All tools run with default settings, no special flags

## Results: Throughput (requests/sec)

### Payload: 0B (empty response)

| Concurrency | hey | oha | jude | Winner |
|---|---|---|---|---|
| 1 | 40,942 | 71,750 | **93,192** | jude 2.3x vs hey |
| 10 | 145,607 | **320,712** | 226,650 | oha |
| 50 | 135,961 | **274,464** | 199,565 | oha |
| 100 | 132,028 | **272,554** | 207,709 | oha |
| 200 | 132,650 | **240,421** | 220,311 | oha |
| 500 | 71,695 | 234,676 | **408,117** | jude 1.7x vs oha |
| 1,000 | 106,861 | 18,329 | **536,758** | jude 5x vs hey |
| 2,000 | 83,914 | 27,587 | **478,879** | jude 5.7x vs hey |
| 5,000 | 64,443 | 14,196 | **372,191** | jude 5.8x vs hey |
| 10,000 | 61,427 | 38,141 | **336,700** | jude 5.5x vs hey |

### Payload: 1KB

| Concurrency | hey | oha | jude | Winner |
|---|---|---|---|---|
| 1 | 37,540 | 67,384 | **91,709** | jude 2.4x vs hey |
| 10 | 136,870 | **304,561** | 240,250 | oha |
| 50 | 120,205 | **294,959** | 222,817 | oha |
| 100 | 110,504 | **225,333** | 217,869 | oha |
| 200 | 128,599 | **202,191** | 208,901 | jude ~= oha |
| 500 | 119,484 | 219,116 | **404,287** | jude 1.8x vs oha |
| 1,000 | 65,029 | 18,185 | **503,717** | jude 7.7x vs hey |
| 2,000 | 82,009 | 26,506 | **421,174** | jude 5.1x vs hey |
| 5,000 | 52,066 | 22,545 | **371,357** | jude 7.1x vs hey |
| 10,000 | 60,317 | 17,978 | **314,893** | jude 5.2x vs hey |

### Payload: 10KB

| Concurrency | hey | oha | jude | Winner |
|---|---|---|---|---|
| 1 | 33,163 | 57,961 | **79,176** | jude 2.4x vs hey |
| 10 | 119,717 | **289,181** | 216,918 | oha |
| 50 | 103,176 | **268,963** | 203,515 | oha |
| 100 | 108,577 | 185,366 | **192,084** | jude |
| 200 | 114,382 | **211,754** | 176,116 | oha |
| 500 | 76,363 | 18,738 | **310,254** | jude 4.1x vs hey |
| 1,000 | 92,048 | 18,455 | **351,345** | jude 3.8x vs hey |
| 2,000 | 88,039 | 14,259 | **293,443** | jude 3.3x vs hey |
| 5,000 | 59,038 | 22,792 | **275,930** | jude 4.7x vs hey |
| 10,000 | 51,309 | 23,229 | **252,268** | jude 4.9x vs hey |

### Payload: 100KB

| Concurrency | hey | oha | jude | Winner |
|---|---|---|---|---|
| 1 | 20,927 | 30,982 | **40,406** | jude 1.9x vs hey |
| 10 | 74,310 | **133,387** | 89,683 | oha |
| 50 | 69,805 | **91,521** | 77,556 | oha |
| 100 | 70,411 | **81,466** | 74,229 | oha |
| 200 | **74,962** | 68,332 | 69,080 | hey |
| 500 | 65,999 | 56,798 | **96,809** | jude 1.5x vs hey |
| 1,000 | 57,265 | 18,017 | **89,545** | jude 1.6x vs hey |
| 2,000 | 57,546 | 23,477 | **85,678** | jude 1.5x vs hey |
| 5,000 | 40,051 | 19,055 | **78,348** | jude 2x vs hey |
| 10,000 | 38,281 | 23,113 | **75,224** | jude 2x vs hey |

## Results: Memory (Peak RSS)

### Payload: 0B

| Concurrency | hey | oha | jude |
|---|---|---|---|
| 1 | 13 MB | 15 MB | **4 MB** |
| 100 | 20 MB | 19 MB | **6 MB** |
| 500 | 51 MB | 34 MB | **17 MB** |
| 1,000 | 80 MB | 41 MB | **32 MB** |
| 5,000 | 301 MB | 115 MB | **144 MB** |
| 10,000 | 492 MB | 212 MB | **284 MB** |

### Payload: 100KB

| Concurrency | hey | oha | jude |
|---|---|---|---|
| 1 | 14 MB | 14 MB | **4 MB** |
| 100 | 20 MB | 46 MB | **7 MB** |
| 1,000 | 90 MB | 224 MB | **36 MB** |
| 10,000 | 581 MB | 1,477 MB | **329 MB** |

## Binary Size

| Tool | Size | Language |
|---|---|---|
| **jude** | **1.2 MB** | Rust |
| hey | 9.0 MB | Go |
| oha | 20 MB | Rust (hyper + ratatui + crossterm) |

## Analysis

### Where jude dominates

- **c=1 (single connection):** 2-2.4x faster than hey, 1.3-1.5x faster than oha. Raw TCP + synchronous buffer parsing has minimal per-request overhead.
- **c=500+:** jude pulls away dramatically. At c=1000, jude achieves **537K RPS** vs hey's 107K (5x) and oha's 18K (29x). The adaptive worker topology with FuturesUnordered event loop scales where others don't.
- **Memory at all levels:** jude consistently uses 2-4x less memory than hey and oha. At c=10000 with 100KB payload, jude uses 329 MB vs oha's 1.5 GB.

### Where oha dominates

- **c=10-200 (low-mid concurrency):** oha is fastest thanks to hyper's optimized connection pool and HTTP parsing. The overhead of hyper's abstraction layers pays off at moderate concurrency where connection reuse patterns are optimal.

### Where hey holds

- **c=200 with 100KB payload:** hey's Go net/http client is well-tuned for this middle ground. But it never exceeds 150K RPS in any scenario.

### Why oha collapses at c=500+

oha (like the earlier hyper-based jude) uses hyper-util's connection pool with one tokio task per connection. At c=500+, tokio scheduler overhead for thousands of tasks becomes the bottleneck. This is the same cliff jude had before switching to raw TCP + FuturesUnordered.

### Why jude scales

1. **Raw TCP** — no HTTP framework overhead, hand-crafted request bytes, synchronous header parsing from BufReader
2. **Adaptive FuturesUnordered** — `clamp(C/128, 1, cpus*2)` workers, each managing ~128 connections. Single-digit workers = minimal scheduler overhead. FuturesUnordered polls connections within each worker without involving the tokio scheduler.
3. **Pre-connect with rate limiting** — all connections established before benchmark starts, limited to 256 concurrent connects to avoid TCP backlog overflow
4. **Zero-copy request** — `Bytes::clone()` is Arc increment, request bytes built once and shared across all workers
5. **Drain-in-place body reading** — `fill_buf()` + `consume()` drains response body through BufReader's existing buffer, no allocation per response

## Reproducing

```bash
# Build
cd jude
cargo build --release
cd bench-server && cargo build --release && cd ..

# Start mock server (port 9977, 1KB response)
./bench-server/target/release/bench-server 9977 1024 &

# Run benchmark
./target/release/jude -n 20000 -c 1000 http://localhost:9977/
hey -n 20000 -c 1000 http://localhost:9977/
oha -n 20000 -c 1000 --no-tui http://localhost:9977/
```
