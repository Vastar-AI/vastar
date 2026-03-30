# Measuring Load Generator Accuracy: A Comprehensive Analysis

**Author:** Vastar Engineering
**Date:** 2026-03-31
**Classification:** Internal — Engineering Reference
**Context:** Built during development of vastar HTTP load generator

---

## 1. The Fundamental Problem

A load generator is a **measuring instrument**. Like any instrument, it has inherent error. The question is not "how fast is the tool?" but "how accurately does the tool measure the system under test?"

If the tool itself is the bottleneck, the numbers you see reflect **the tool's limitations**, not the server's capacity.

```
Observed RPS = min(Tool Capacity, Server Capacity)
```

If Tool Capacity < Server Capacity → you are measuring the tool, not the server.

---

## 2. Sources of Inaccuracy

### 2.1 Coordinated Omission

**The most critical accuracy problem in load testing.**

Most load generators (including hey, oha, wrk) measure latency as:
```
latency = time_response_received - time_request_sent
```

This seems correct but **misses the queuing time**. When the server is overloaded and the tool is waiting for a connection slot to become available, that waiting time is NOT recorded.

**Example:**
- Target: 1000 RPS
- Server can handle: 500 RPS
- Requests 501-1000 queue inside the tool, waiting for connections
- When they finally execute, their measured latency is low (server responded fast)
- But the USER experienced: queue_wait + server_latency

**Impact:** p99 is artificially low. The tool reports the server is healthy when users are experiencing multi-second delays.

**Detection:** Compare `Total Time / (Requests / Concurrency)` with `Average Latency`. If Total Time >> Average * (Requests/Concurrency), coordinated omission is occurring.

**vastar's approach:** vastar measures from `Instant::now()` at the moment the request future is created, not when the TCP write begins. This captures some queuing but not all (pre-connect queuing is separate from benchmark timing).

### 2.2 Tool Overhead Per Request

Each request has overhead from the tool itself:

| Component | hey (Go) | oha (Rust/hyper) | vastar (Rust/raw TCP) |
|---|---|---|---|
| Request clone | Full HTTP struct clone | HeaderMap clone | Bytes::clone (Arc increment) |
| Connection mgmt | Go net/http pool | hyper-util pool | Manual BufReader reuse |
| Stats recording | Channel send per request | Channel send per request | Local Vec push |
| Async yield | Goroutine scheduler | tokio task per conn | FuturesUnordered per worker |

**Measurement:** Run tool against an infinitely-fast server (raw TCP echo that returns immediately). The RPS you see is the tool's maximum throughput — its ceiling.

### 2.3 Connection Establishment Bias

How and when connections are established affects results:

| Strategy | Behavior | Bias |
|---|---|---|
| **Lazy connect** | Connections created as needed during benchmark | Includes connect time in early request latencies |
| **Pre-connect** | All connections established before timing starts | Excludes connect overhead (more accurate for sustained load) |
| **Connection storm** | All connections created simultaneously | May overwhelm server's TCP backlog, causing artificial errors |

**vastar's approach:** Pre-connect with semaphore (max 256 concurrent). Benchmark timer starts after all connections established. This is most representative of sustained production traffic where connections are already warm.

### 2.4 Client-Side Resource Exhaustion

At high concurrency, the tool itself consumes resources that affect measurement:

| Resource | Effect |
|---|---|
| **File descriptors** | c=10000 needs 10000 FDs. Default ulimit is often 1024. |
| **Memory** | Each connection has buffers. hey: ~45KB/conn, vastar: ~32KB/conn |
| **CPU** | Parsing responses, computing stats, rendering progress |
| **Ephemeral ports** | OS has ~28000 ephemeral ports. TIME_WAIT connections reduce available ports. |

**Detection:** Monitor tool's own CPU and memory during benchmark. If tool CPU > 80%, results are suspect.

### 2.5 Network Stack Interference

| Factor | Impact |
|---|---|
| **TCP Nagle** | Delays small writes. Tool should set TCP_NODELAY. |
| **SO_RCVBUF / SO_SNDBUF** | Default buffer sizes may bottleneck at high throughput. |
| **Connection reuse** | Keep-alive vs close affects throughput dramatically. |
| **Localhost loopback** | Different kernel path than real network. Results don't transfer to production. |

### 2.6 Statistical Sampling Error

| Issue | Description |
|---|---|
| **Too few requests** | n=100 gives unreliable percentiles. p99 is literally 1 data point. |
| **Too short duration** | 1-second test doesn't capture GC pauses, background jobs, cron effects. |
| **Warmup ignored** | First N requests include JIT compilation, cache warmup, connection pool initialization. |
| **Outlier sensitivity** | One 10-second timeout skews average by 100x but doesn't affect p50. |

**Rule of thumb:**
- Minimum 10,000 requests for reliable p99
- Minimum 100,000 requests for reliable p99.9
- At least 30 seconds duration to capture system-level variance

---

## 3. How to Validate a Load Generator

### 3.1 The Null Test

Benchmark the tool against a **do-nothing server** that returns a fixed response instantly.

```bash
# Start null server (returns "HTTP/1.1 200 OK\r\n\r\n" with zero processing)
./bench-server 9977 0

# Measure tool ceiling
vastar -n 100000 -c 100 http://localhost:9977/
hey -n 100000 -c 100 http://localhost:9977/
```

**Expected:** Tool should report very high RPS. If tool A reports 400K and tool B reports 100K, tool B has 4x more overhead per request.

**What this tells you:** The maximum RPS the tool can generate. Any benchmark result close to this number means the tool is the bottleneck.

### 3.2 The Known Latency Test

Server that sleeps for exactly N milliseconds before responding.

```python
# Server that sleeps 50ms per request
async def handler(request):
    await asyncio.sleep(0.050)
    return Response("OK")
```

**Expected results:**
- p50 should be ~50ms (± 1ms)
- p99 should be ~50ms (± 2ms)
- RPS should be ~concurrency / 0.050 = c * 20

If tool reports p50=50ms but RPS < c*20, the tool is introducing delay.
If tool reports p50 > 55ms consistently, the tool's per-request overhead is ~5ms.

### 3.3 The Cross-Validation Test

Run two different tools against the same server, same parameters.

```bash
vastar -n 10000 -c 300 -m POST -d '...' http://server:8080/api
hey -n 10000 -c 300 -m POST -d '...' http://server:8080/api
```

**Compare:**
- `resp wait` should be nearly identical (server latency is constant)
- `RPS` difference = tool overhead difference
- `p99` difference at high concurrency reveals scheduling efficiency

**From our data (VIL gateway, c=300):**
| Metric | hey | vastar |
|---|---|---|
| resp wait | 44.2ms | 44.4ms |
| RPS | 5,466 | 6,374 |
| p99 | 95.8ms | 73.0ms |

Server latency identical (44ms). vastar 17% higher RPS and tighter p99 — the difference is pure tool overhead.

### 3.4 The Saturation Test

Gradually increase concurrency until server breaks. Compare the **inflection point** across tools.

```bash
for c in 50 100 200 300 400 500 600; do
    vastar -n 5000 -c $c http://server/api
done
```

**Expected:** All tools should find the same server saturation point. If tool A shows errors at c=500 but tool B at c=600, tool A is causing premature saturation (e.g., connection storm, slow connect).

**From our data (VIL gateway):**
| Concurrency | vastar | hey |
|---|---|---|
| c=500 | 100% success | 71% success (29% 502) |
| c=600 | 12% success | 26% success |

hey generated more errors at c=500 because its connection management is more aggressive — overwhelming the gateway's connection pool.

---

## 4. Accuracy Checklist for Production Benchmarks

### Before the benchmark:
- [ ] Server and tool on **separate machines** (or at minimum separate CPU cores)
- [ ] `ulimit -n` set to at least 2x concurrency
- [ ] TCP TIME_WAIT recycling enabled (`net.ipv4.tcp_tw_reuse=1`)
- [ ] Monitoring running on the server (CPU, memory, network, disk I/O)
- [ ] Server warmed up (run a small load first, discard results)

### During the benchmark:
- [ ] Tool CPU usage < 50% of available cores
- [ ] Tool memory not growing unbounded
- [ ] Network is not saturated (check `sar -n DEV 1`)
- [ ] No other significant processes competing for resources

### After the benchmark:
- [ ] Compare `resp wait` across tools (should be consistent)
- [ ] Check error rate — non-zero errors invalidate latency percentiles
- [ ] Verify total time makes sense: `Total ≈ Requests / RPS`
- [ ] Run 3+ times and check variance. >10% variance = unstable measurement

### Red flags that indicate tool is the bottleneck:
- [ ] RPS is close to the tool's null-test ceiling
- [ ] p99 is much higher than p95 (queuing inside the tool)
- [ ] Different tools show very different RPS for the same server
- [ ] Adding more concurrency doesn't increase RPS (tool saturated)

---

## 5. Vastar's Accuracy Design Decisions

| Decision | Rationale |
|---|---|
| **Pre-connect before timing** | Connection establishment is not part of sustained load measurement |
| **Adaptive worker topology** | Prevents tool's own scheduler from becoming bottleneck at high C |
| **Per-worker local stats** | No channel contention on hot path (Vec push vs channel send) |
| **AtomicU64 for progress** | Lock-free live progress doesn't interfere with measurement |
| **Raw TCP (no hyper)** | Eliminates framework overhead from the measurement instrument |
| **fill_buf + consume body drain** | Minimal per-response allocation, doesn't distort memory readings |
| **Nanosecond timing** | `Instant::now()` per request, not per-batch |
| **Sort once at end** | Percentile computation is O(n log n) once, not per-request overhead |

### Known limitations:
1. **No coordinated omission correction** — vastar does not apply Gil Tene's correction. Reported percentiles are "service time" not "response time from user perspective."
2. **Pre-connect hides connection cost** — real production traffic includes connection establishment. vastar's numbers are best-case for warm connections.
3. **Single-machine only** — vastar runs on one machine. Distributed load generation (like k6 cloud, Locust distributed) can generate more load from multiple sources.
4. **HTTP/1.1 only** — no HTTP/2 multiplexing. Servers that benefit from H2 multiplexing will show lower numbers with vastar.

---

## 6. Industry Reference: Gil Tene's "How NOT to Measure Latency"

The seminal talk by Gil Tene (Azul Systems) identifies key pitfalls:

1. **Coordinated omission** — the #1 mistake. Almost all tools suffer from this.
2. **Averaging averages** — mathematically invalid for latency distributions.
3. **Ignoring high percentiles** — p50 is useless for SLO; p99.9 reveals system behavior.
4. **Testing in steady state only** — real systems experience GC pauses, compaction, cache eviction.
5. **Insufficient duration** — 10-second tests miss hourly/daily patterns.

**Recommendation:** For production capacity planning, run benchmarks for at least 10 minutes with vastar/hey, then **also** use a sustained-load tool (k6, Gatling) for multi-hour soak tests.

---

## 7. When to Use Which Tool

| Scenario | Recommended Tool | Why |
|---|---|---|
| Quick smoke test | `curl` | Single request, see response |
| API latency baseline | `vastar -n 1000 -c 1` | Single connection, pure server latency |
| Throughput ceiling | `vastar -n 50000 -c 500` | High concurrency, vastar's sweet spot |
| Sustained soak test | k6, Gatling, Locust | Multi-hour, scripted scenarios |
| Distributed load | k6 cloud, Locust distributed | Load from multiple geographic locations |
| HTTP/2 testing | h2load, oha | vastar is HTTP/1.1 only |
| Browser simulation | Playwright, Puppeteer | Real browser rendering, not just HTTP |
| Production traffic replay | GoReplay, tcpreplay | Exact production pattern reproduction |

---

## 8. Reproducing Our Benchmark Methodology

All vastar benchmarks use this methodology:

```bash
# 1. Build mock server (zero processing time)
cd vastar/bench-server && cargo build --release

# 2. Start server with specific payload size
./target/release/bench-server 9977 1024  # 1KB response

# 3. Warmup (discard results)
vastar -n 5000 -c 100 http://localhost:9977/ > /dev/null

# 4. Run benchmark (5 rounds, take median)
for i in 1 2 3 4 5; do
    vastar -n 20000 -c $CONCURRENCY http://localhost:9977/
done

# 5. Compare with other tools (same parameters)
hey -n 20000 -c $CONCURRENCY http://localhost:9977/
oha -n 20000 -c $CONCURRENCY --no-tui http://localhost:9977/
```

**Critical:** bench-server is a raw TCP server with zero processing time. This measures pure tool+network overhead. For application benchmarks, replace with your actual server.

---

## 9. Summary

| Principle | Application |
|---|---|
| Tool is an instrument, not the subject | Always verify tool is not the bottleneck |
| Cross-validate with multiple tools | If tools disagree, investigate why |
| resp_wait should be consistent | Server latency is constant regardless of tool |
| Pre-connect for sustained load | Connection cost is separate from throughput measurement |
| Minimum 10K requests for p99 | Statistical significance requires sample size |
| Report the methodology | Without methodology, numbers are meaningless |
| Be honest about limitations | No tool wins every scenario |

---

*This document is based on experience building vastar (raw TCP HTTP load generator) and benchmarking it against hey (Go) and oha (Rust/hyper) across 40 test configurations (10 concurrency levels × 4 payload sizes).*
