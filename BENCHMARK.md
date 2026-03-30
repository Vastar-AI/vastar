# Benchmark Report: vastar vs hey vs oha

**Date:** 2026-03-30
**Machine:** 16 cores, 31 GB RAM, Linux 6.8.0
**Versions:** vastar 0.1.0, hey 0.0.1 (Go), oha 1.14.0 (Rust/hyper)
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

| Concurrency | hey | oha | vastar | Winner |
|---|---|---|---|---|
| 1 | 40,942 | 71,750 | **93,192** | vastar 2.3x vs hey |
| 10 | 145,607 | **320,712** | 226,650 | oha |
| 50 | 135,961 | **274,464** | 199,565 | oha |
| 100 | 132,028 | **272,554** | 207,709 | oha |
| 200 | 132,650 | **240,421** | 220,311 | oha |
| 500 | 71,695 | 234,676 | **408,117** | vastar 1.7x vs oha |
| 1,000 | 106,861 | 18,329 | **536,758** | vastar 5x vs hey |
| 2,000 | 83,914 | 27,587 | **478,879** | vastar 5.7x vs hey |
| 5,000 | 64,443 | 14,196 | **372,191** | vastar 5.8x vs hey |
| 10,000 | 61,427 | 38,141 | **336,700** | vastar 5.5x vs hey |

### Payload: 1KB

| Concurrency | hey | oha | vastar | Winner |
|---|---|---|---|---|
| 1 | 37,540 | 67,384 | **91,709** | vastar 2.4x vs hey |
| 10 | 136,870 | **304,561** | 240,250 | oha |
| 50 | 120,205 | **294,959** | 222,817 | oha |
| 100 | 110,504 | **225,333** | 217,869 | oha |
| 200 | 128,599 | **202,191** | 208,901 | vastar ~= oha |
| 500 | 119,484 | 219,116 | **404,287** | vastar 1.8x vs oha |
| 1,000 | 65,029 | 18,185 | **503,717** | vastar 7.7x vs hey |
| 2,000 | 82,009 | 26,506 | **421,174** | vastar 5.1x vs hey |
| 5,000 | 52,066 | 22,545 | **371,357** | vastar 7.1x vs hey |
| 10,000 | 60,317 | 17,978 | **314,893** | vastar 5.2x vs hey |

### Payload: 10KB

| Concurrency | hey | oha | vastar | Winner |
|---|---|---|---|---|
| 1 | 33,163 | 57,961 | **79,176** | vastar 2.4x vs hey |
| 10 | 119,717 | **289,181** | 216,918 | oha |
| 50 | 103,176 | **268,963** | 203,515 | oha |
| 100 | 108,577 | 185,366 | **192,084** | vastar |
| 200 | 114,382 | **211,754** | 176,116 | oha |
| 500 | 76,363 | 18,738 | **310,254** | vastar 4.1x vs hey |
| 1,000 | 92,048 | 18,455 | **351,345** | vastar 3.8x vs hey |
| 2,000 | 88,039 | 14,259 | **293,443** | vastar 3.3x vs hey |
| 5,000 | 59,038 | 22,792 | **275,930** | vastar 4.7x vs hey |
| 10,000 | 51,309 | 23,229 | **252,268** | vastar 4.9x vs hey |

### Payload: 100KB

| Concurrency | hey | oha | vastar | Winner |
|---|---|---|---|---|
| 1 | 20,927 | 30,982 | **40,406** | vastar 1.9x vs hey |
| 10 | 74,310 | **133,387** | 89,683 | oha |
| 50 | 69,805 | **91,521** | 77,556 | oha |
| 100 | 70,411 | **81,466** | 74,229 | oha |
| 200 | **74,962** | 68,332 | 69,080 | hey |
| 500 | 65,999 | 56,798 | **96,809** | vastar 1.5x vs hey |
| 1,000 | 57,265 | 18,017 | **89,545** | vastar 1.6x vs hey |
| 2,000 | 57,546 | 23,477 | **85,678** | vastar 1.5x vs hey |
| 5,000 | 40,051 | 19,055 | **78,348** | vastar 2x vs hey |
| 10,000 | 38,281 | 23,113 | **75,224** | vastar 2x vs hey |

## Results: Memory (Peak RSS)

### Payload: 0B

| Concurrency | hey | oha | vastar |
|---|---|---|---|
| 1 | 13 MB | 15 MB | **4 MB** |
| 100 | 20 MB | 19 MB | **6 MB** |
| 500 | 51 MB | 34 MB | **17 MB** |
| 1,000 | 80 MB | 41 MB | **32 MB** |
| 5,000 | 301 MB | 115 MB | **144 MB** |
| 10,000 | 492 MB | 212 MB | **284 MB** |

### Payload: 100KB

| Concurrency | hey | oha | vastar |
|---|---|---|---|
| 1 | 14 MB | 14 MB | **4 MB** |
| 100 | 20 MB | 46 MB | **7 MB** |
| 1,000 | 90 MB | 224 MB | **36 MB** |
| 10,000 | 581 MB | 1,477 MB | **329 MB** |

## Binary Size

| Tool | Size | Language |
|---|---|---|
| **vastar** | **1.2 MB** | Rust |
| hey | 9.0 MB | Go |
| oha | 20 MB | Rust (hyper + ratatui + crossterm) |

## Analysis

### Important caveats

- All benchmarks run on **localhost loopback** — kernel bypasses the network stack. Production results over real networks will differ significantly due to TCP round-trip, congestion, and packet loss.
- The server is a **zero-processing mock** — it returns a fixed response immediately. Real application servers have processing time that dwarfs tool overhead differences.
- **Throughput differences between tools reflect tool overhead**, not server capacity. The `resp wait` metric (server-side latency) is consistent across all tools for the same server.
- Results are from a **single machine** (16 cores, 31 GB RAM). Different hardware will produce different absolute numbers, though relative patterns should hold.

### Observations by concurrency range

**c=1-200 (low to moderate):** oha shows highest throughput in this range, particularly with larger payloads. hyper's connection pool and HTTP parsing are well-optimized for moderate connection reuse patterns. hey performs consistently across all scenarios. vastar shows highest single-connection throughput (c=1) due to minimal per-request overhead.

**c=500+ (high concurrency):** vastar shows highest throughput. hey maintains stable performance. oha's throughput decreases significantly — this appears related to per-connection task scheduling overhead, though we have not verified oha's internals to confirm the exact cause.

**100KB payload at c=200:** hey shows slightly higher throughput than vastar and oha. Go's net/http client handles this combination well.

### Memory usage

vastar uses less memory than hey across all concurrency levels (roughly 2-3x less). At c=10,000 with 100KB payload, oha uses significantly more memory (1,477 MB) compared to vastar (329 MB) and hey (581 MB). Memory differences are primarily due to per-connection buffer allocation strategies.

### vastar architecture notes

vastar's approach differs from hey and oha in several ways:

1. **Raw TCP** — no HTTP framework. Request bytes are hand-crafted and written directly to TcpStream. This reduces per-request overhead but means vastar only supports HTTP/1.1 (no HTTP/2).
2. **Adaptive worker topology** — workers scale as `clamp(C/128, 1, cpus*2)`. Each worker manages connections via FuturesUnordered. This trades complexity for reduced scheduler overhead at high concurrency.
3. **Pre-connect** — all connections established before timing starts, rate-limited to 256 concurrent. This measures sustained throughput, not connection establishment.
4. **Zero-copy request** — `Bytes::clone()` is an Arc increment. Request bytes built once and shared.
5. **Drain-in-place** — response bodies consumed via `fill_buf()` + `consume()` through BufReader's existing buffer.

These choices optimize for high-concurrency throughput but mean vastar is **not the best tool for every scenario**. For HTTP/2 testing, use oha or h2load. For scripted multi-step scenarios, use k6 or Gatling. For moderate concurrency with large payloads, hey or oha may produce more representative results.

## Reproducing

```bash
# Build
cd vastar
cargo build --release
cd bench-server && cargo build --release && cd ..

# Start mock server (port 9977, 1KB response)
./bench-server/target/release/bench-server 9977 1024 &

# Run benchmark
./target/release/vastar -n 20000 -c 1000 http://localhost:9977/
hey -n 20000 -c 1000 http://localhost:9977/
oha -n 20000 -c 1000 --no-tui http://localhost:9977/
```
