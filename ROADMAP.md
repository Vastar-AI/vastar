# Vastar Roadmap

## Current (v0.1.x) — HTTP/1.1 Load Generator

vastar currently supports HTTP/1.1 with raw TCP and SSE streaming. This roadmap outlines the evolution from HTTP load generator to a **universal benchmark tool** for modern infrastructure — databases, message queues, AI inference, storage, edge compute, and every protocol in between.

All features are subcommands sharing the same core engine (adaptive worker topology, FuturesUnordered, progress bar, SLO Insight).

---

## Known Bugs

| Bug | Description | Workaround | Priority |
|---|---|---|---|
| `-H` does not override `-T` default | `-H "Content-Type: application/json"` adds a second content-type header instead of overriding the `-T` default (`text/html`). Server receives both headers — some servers pick the wrong one and return 400. | Use `-T "application/json"` instead of `-H "Content-Type: ..."` | **high** |
| `read_chunk_size` premature EOF | Under high concurrency with chunked transfer-encoding, `read_chunk_size` returns 0 when `\n` is not in the current buffer, causing premature chunk drain termination. Next request on same keep-alive connection reads stale data → 400. | Increase BufReader capacity or disable keep-alive (`--disable-keepalive`) | **high** |

**Root cause for both**: vastar uses raw TCP with manual HTTP/1.1 parsing. `-H` headers are appended after `-T` default without dedup. Chunked parser doesn't accumulate across buffer boundaries.

**Fix plan**: Deduplicate headers (later `-H` overrides earlier same-name header). Fix `read_chunk_size` to accumulate line across fill_buf calls before parsing hex.

---

## Phase 0: Concurrency Sweet-Spot Sweep (`vastar sweep`) — **SHIPPED in v0.2.0**

Benchmark users today have to hand-tune `-c` per endpoint: too low and they under-report throughput, too high and queueing explodes the tail — and the right value differs by workload (sub-ms echo vs I/O-bound SQL vs streaming LLM). Every driver script (VIL testsuite, CI harnesses) ends up embedding its own ad-hoc sweep loop.

`vastar sweep` is a **domain-agnostic** subcommand that runs an adaptive concurrency sweep against any endpoint vastar already supports and emits the empirically best `c` (plus the full curve) as text and JSON. Script-callable, cache-friendly, zero workload assumptions.

### Design principles

- **Domain-agnostic** — no hardcoded workload classes. Caller passes URL + method + payload, algorithm treats every endpoint identically.
- **Evidence-based** — knee detected empirically from measured `rps` / `p99` curve, not from CPU-core heuristics or preset tables.
- **Noise-robust** — multi-repeat with median aggregation; disqualification gates for unstable runs.
- **Script-friendly** — first-class JSON output with stable schema so downstream tools (CI gates, bench drivers, dashboards) can consume without parsing text.
- **Reuses core engine** — no refactor needed; `sweep` orchestrates multiple `engine::run()` invocations with different `-c` values and aggregates results.

### Invocation

```
vastar sweep [OPTIONS] <URL>

  # Concurrency plan
  --conc <SPEC>             "10,50,100,500" | "10..1000:log=6" | "10..200:step=20" | "auto" (default)
  --refine                  After coarse sweep, bracket ±50% around winner and sweep 4 more points
  --repeats <N>             Repeat each c-level N times, take median (default: 1)

  # Picking strategy
  --pick <knee|score>       Selection algorithm (default: knee)
  --knee-ratio <0.95>       Smallest c reaching this fraction of peak rps
  --baseline-c <1>          Concurrency used as reference for tail-degradation check

  # Disqualification gates
  --max-spread <4.0>        DQ if p99/p50 > this
  --max-p999-ratio <8.0>    DQ if p99.9/p50 > this
  --max-errors <0.01>       DQ if error_rate > this
  --max-tail-mult <3.0>     DQ if p99 > baseline_p99 × this

  # Output
  -o, --output <FMT>        text | json | ndjson | csv (default: text)
  --json-path <FILE>        Also write JSON to file (text still prints to stdout)

  # Pass-through to each sub-benchmark (reuses existing vastar flags)
  -n, -z, -m, -d, -D, -T, -H, -A, -a, -t, --disable-keepalive, --disable-compression
```

### Algorithm

1. **Calibrate baseline** — run once at `--baseline-c` (default 1) to capture uncontended `p50`/`p99`. Defines "healthy tail" per-endpoint instead of relying on absolute thresholds.
2. **Coarse sweep** — resolve `--conc` spec to concrete levels, run each (with optional repeats + median), tag each point `pass` or `DQ(reason)`.
3. **Refine (optional)** — pick current winner, bracket `[winner × 0.5, winner × 1.5]`, sweep 4 more points, merge.
4. **Pick sweet spot** —
    - **knee mode (default)**: smallest `c` where `rps ≥ knee_ratio × peak_rps` **and** `p99 ≤ baseline_p99 × max_tail_mult`. Falls back to `argmax(rps)` if neither gate met.
    - **score mode**: `argmax(rps / (p99/p50)²)` — throughput weighted by consistency² (original VIL testsuite formula).
5. **Emit** — pretty table + highlighted sweet spot to stdout; structured JSON to file/stdout for downstream consumption.

### Output JSON contract (`schema_version: "1.0"`)

```json
{
  "schema_version": "1.0",
  "params": { "url": "...", "method": "POST", "baseline_c": 1, "pick": "knee", ... },
  "machine": { "cpu_cores_physical": 8, "cpu_cores_logical": 16, "ram_mb": 20000 },
  "baseline": { "concurrency": 1, "rps": 3200, "p50_ms": 0.31, "p99_ms": 0.45 },
  "sweep_points": [
    { "concurrency": 10, "repeats": 3, "rps": 6420, "p50_ms": 1.55, "p95_ms": 2.30,
      "p99_ms": 3.10, "p999_ms": 3.80, "error_rate": 0.0, "disqualified": null, "score": 2660 },
    { "concurrency": 1000, "disqualified": "spread=2.8" }
  ],
  "sweet_spot": {
    "concurrency": 180, "rps": 35800, "p50_ms": 4.55, "p99_ms": 10.2,
    "method": "knee",
    "reasoning": "smallest c reaching 93.7% of peak (38200 @ c=400), p99 within tail gate",
    "peak_rps": 38200, "peak_concurrency": 400
  },
  "notes": ["refine=on", "repeats=3"]
}
```

### Text output (sample)

```
━━━ vastar sweep — POST http://localhost:10003/api/fx/convert ━━━

  Calibration (c=1):      rps=3200     p50=0.31ms   p99=0.45ms
  Machine:                8 phys / 16 log cores, 20 GB RAM

  Coarse sweep (6 points, 3 repeats each, median):
    c       rps          p50       p95       p99       p99.9     score    verdict
    ─────   ─────────    ──────    ──────    ──────    ──────    ─────    ───────
    10      6420         1.55ms    2.30ms    3.10ms    3.80ms    2660
    50      18900        2.64ms    4.20ms    5.80ms    7.20ms    4420
    150     34100        4.39ms    7.10ms    9.80ms    12.3ms    6840
    400     38200        10.4ms    28.0ms    41.0ms    58.0ms    590      high tail
    1000    31500        31.8ms    68.0ms    89.0ms    125ms     —        DISQ (spread=2.8)

  Refine around c=150 (bracket c=75..225):
    c       rps          p50       p95       p99       p99.9
    100     29200        3.42ms    5.80ms    7.90ms    10.1ms
    120     32100        3.75ms    6.30ms    8.40ms    11.0ms
    180     35800        4.55ms    7.40ms    10.2ms    13.1ms   ← best
    250     37500        5.89ms    10.5ms    14.3ms    17.8ms

  ━━━ Sweet spot: c=180 ━━━
  Throughput:   35800 req/s   (93.7% of peak 38200 @ c=400)
  Latency p99:  10.2ms        (22× baseline c=1, within gates)
  Strategy:     knee@95%
  Reasoning:    smallest c reaching ≥95% of peak throughput with healthy tail
```

### CLI backward compatibility

Introduces the first subcommand into the CLI. Existing flat-form invocations (`vastar -c 100 -n 2000 URL`) remain supported via clap's `subcommand_negates_reqs` + optional subcommand pattern — no breakage for existing callers (VIL testsuite, docs, CI pipelines).

### Downstream integration example

```bash
SWEEP=$(vastar sweep -o json --repeats 3 -n 2000 \
    -m POST -T application/json -d '{"prompt":"bench"}' \
    http://localhost:3080/trigger)

BENCH_C=$(echo "$SWEEP" | jq -r .sweet_spot.concurrency)
```

### Explicit non-goals

- **Thermal / CPU-governor / FD-limit probing** — OS/mesin-specific; stays out of a domain-agnostic bench tool
- **Workload auto-classification** — caller knows the domain; `--pick` is the only knob
- **Per-category presets** — shell out multiple `vastar sweep` invocations instead; keeps the tool lean
- **Result cache persistence** — cache is the caller's concern (dump `--json-path` and source it later)

### Paired sweep — platform-overhead mode

Single-endpoint sweep answers "what c saturates *this* URL". For platforms that front an upstream (API gateways, service meshes, sidecars, provision servers fronting simulators), that number can be misleading: the target looks healthy at high c simply because the upstream is doing the heavy lifting, while the platform itself has already become the bottleneck. Paired sweep catches that explicitly.

```
vastar sweep \
  --vs http://localhost:4545/v1/chat/completions \     # reference (upstream)
  --max-overhead-pct 25 \                              # DQ when target p99 >25% of ref
  -m POST -T application/json \
  -d '{"prompt":"bench"}' \
  http://localhost:3080/trigger                        # target (gateway)
```

At each concurrency level the engine runs both endpoints (reference first for stable warm-up, then target) and computes:

- `overhead_pct = (target_p99 - ref_p99) / ref_p99 × 100` — how much extra latency the platform adds at this load
- `rps_deficit_pct = (ref_rps - target_rps) / ref_rps × 100` — whether the platform keeps up with the upstream's own throughput

Points failing either gate (`--max-overhead-pct`, default 25%; `--max-rps-deficit-pct`, default 50%) are DQ'd. Sweet spot picker then chooses among qualified points — typically surfacing a meaningfully *lower* `c` than a pure single-endpoint sweep, because the overhead gate exposes where the platform transitions from "transparent" to "bottleneck".

**Reference caching** — `--ref-from-json <FILE>` loads a reference curve from a prior `vastar sweep -o json` result, skipping all reference measurements. Useful for sweeping many gateway endpoints against the same upstream:

```
# Once: cache upstream curve
vastar sweep -o json --conc auto -n 2000 --repeats 3 \
  -m POST -T application/json -d '{"prompt":"bench"}' \
  http://localhost:4545/v1/chat/completions > /tmp/upstream.json

# Many times: reuse for each gateway test, no re-sweep
vastar sweep --ref-from-json /tmp/upstream.json --max-overhead-pct 20 \
  ... http://localhost:3080/trigger
vastar sweep --ref-from-json /tmp/upstream.json --max-overhead-pct 20 \
  ... http://localhost:3081/api/gw/trigger
```

JSON output schema v1.0 extends with a top-level `paired` block (reference URL/method/source, baseline, gate thresholds) and per-sweep-point `reference` / `overhead_pct` / `rps_deficit_pct` fields. Single-endpoint runs remain backward-compatible — no `paired` block emitted.

### Why Phase 0 (before Phase 1)

Every other bench feature — HTTP/2, TLS, gRPC, AI inference, SQL — compounds value only when the operator knows how to drive it correctly. Fixing `-c` as an operator guess is the most leveraged improvement: one feature that upgrades every existing and future subcommand. This is also the feature that unlocks clean CI gates (stable sweet spot → stable SLO threshold).

---

## Phase 1: HTTP Feature Parity

Missing features that hey and/or oha already support.

| Feature | hey | oha | vastar | Priority |
|---|---|---|---|---|
| **-H override -T** | **yes** | **yes** | **no (bug)** | **critical** |
| HTTP/2 | yes | yes | no | high |
| TLS/HTTPS | yes | yes | no | high |
| HTTP proxy | yes | yes | no | medium |
| Follow redirects | yes (default) | yes (configurable) | no | medium |
| Disable compression | yes | yes | no | low |
| Disable keep-alive | yes | yes | yes | done |
| Custom timeout | yes | yes | yes | done |
| Request body from file (-D) | yes | yes | yes | done |
| Basic auth | yes | yes | yes | done |
| Rate limiting (QPS) | yes | yes | partial | medium |
| Duration mode (-z) | yes | yes | yes | done |
| Output format (JSON/CSV) | csv | json/csv | no | medium |
| Latency correction (coordinated omission) | no | yes | no | high |
| Unix socket | no | yes | no | low |
| Connect-to (host override) | no | yes | no | low |
| AWS SigV4 auth | no | yes | no | low |
| Random URL generation | no | yes | no | low |
| Multiple URLs from file | no | yes | no | medium |

## Phase 2: HTTPS + HTTP/2

| Feature | Description | Approach |
|---|---|---|
| TLS support | HTTPS endpoints | rustls (no OpenSSL dependency) |
| HTTP/2 | Multiplexed streams | h2 crate, maintain raw TCP philosophy |
| ALPN negotiation | Auto HTTP/1.1 vs HTTP/2 | Based on TLS ALPN |
| Certificate verification | System CA + custom certs | rustls-native-certs |
| Client certificates | mTLS support | rustls |

## Phase 3: Multi-Protocol Load Generator

Expand beyond HTTP to become a universal high-throughput protocol tester.

### gRPC
| Feature | Description |
|---|---|
| Unary RPC | Single request-response |
| Server streaming | Server sends stream of messages |
| Client streaming | Client sends stream of messages |
| Bidirectional streaming | Both sides stream |
| Protobuf payload | Load .proto files, generate requests |
| Reflection | Auto-discover services without .proto |

### WebSocket
| Feature | Description |
|---|---|
| Connection load | Open N concurrent WebSocket connections |
| Message throughput | Send M messages per second across connections |
| Echo benchmark | Measure round-trip latency |
| Binary + text frames | Support both frame types |
| Ping/pong latency | Measure keep-alive overhead |

### QUIC / HTTP/3
| Feature | Description |
|---|---|
| QUIC transport | UDP-based, 0-RTT connection |
| HTTP/3 requests | Over QUIC streams |
| Migration testing | Connection migration under load |

### Server-Sent Events (SSE) — Supported

vastar already handles chunked transfer encoding used by SSE endpoints. Tested against ai-endpoint-simulator (OpenAI, Anthropic, Ollama, Cohere, Gemini SSE dialects) at up to 10K concurrent connections.

| Feature | Status |
|---|---|
| SSE connection load | done (chunked drain) |
| Event throughput | done (measures full stream completion) |
| Reconnection testing | planned |
| Last-Event-ID | planned |

### Message Queue Protocols
| Feature | Description |
|---|---|
| MQTT | Publish/subscribe throughput, QoS levels |
| NATS | Pub/sub and request/reply benchmarks |
| Kafka | Producer throughput, consumer lag |
| AMQP (RabbitMQ) | Publish/consume benchmarks |

### Other Protocols
| Feature | Description |
|---|---|
| RSocket | Request-response, fire-and-forget, streaming |
| GraphQL | Query/mutation load testing with variable payloads |
| TCP raw | Generic TCP echo/throughput benchmark |
| UDP | Datagram throughput measurement |

## Phase 4: Advanced Analysis

| Feature | Description |
|---|---|
| Coordinated omission correction | Gil Tene's HdrHistogram-style correction |
| Comparative mode | Run vastar vs hey vs oha automatically, produce comparison report |
| Flamegraph integration | CPU profile of the tool itself during benchmark |
| Distributed mode | Coordinator + agent across multiple machines |
| Scenario scripting | Multi-step workflows (login → browse → checkout) |
| Custom SLO definitions | User-defined absolute thresholds (--slo-p99=200ms) |
| Prometheus push | Push benchmark results to Prometheus pushgateway |
| CI/CD integration | Exit code based on SLO pass/fail for pipeline gates |

## Phase 5: Ecosystem

| Feature | Description |
|---|---|
| vastar-cloud | Hosted distributed load generation |
| vastar-report | HTML report generator from benchmark output |
| vastar-compare | Side-by-side comparison tool (vastar vs hey vs oha) |
| IDE plugin | VS Code extension with inline benchmark results |
| GitHub Action | Run benchmarks in CI, comment results on PR |

## Phase 6: AI Engineering (`vastar ai-bench`)

AI inference has metrics that generic HTTP tools cannot measure — time to first token, tokens per second, inter-token latency, cost estimation. vastar already handles SSE streaming; this phase parses the stream content to extract AI-specific metrics.

All AI features will be subcommands under `vastar ai-bench` — keeping the binary small and the core HTTP engine unchanged.

### LLM Inference Metrics

```
vastar ai-bench -c 50 -n 1000 \
  --model gpt-4o \
  --prompt "Explain quantum computing" \
  http://localhost:4545/v1/chat/completions
```

| Metric | Description | Status |
|---|---|---|
| Time to First Token (TTFT) | Latency from request to first SSE chunk | planned |
| Tokens per Second (TPS) | Token throughput during streaming | planned |
| Inter-Token Latency (ITL) | Time between consecutive tokens | planned |
| Total Tokens | Token count per response | planned |
| Total Stream Time | End-to-end SSE stream duration | done (existing) |
| SSE Chunk Drain | Chunked transfer decode | done (existing) |

### AI-Specific SLO & Insight

```
AI Inference Insight:

  TTFT p50 = 12ms, p99 = 45ms -- within 100ms target
  TPS  p50 = 85 tok/s -- above 50 tok/s minimum
  ITL  p50 = 11.7ms -- smooth streaming

  Token cost: ~$0.0034/request (est. gpt-4o pricing)
  Estimated hourly cost at current RPS: $12.24/hr
```

| Feature | Description | Status |
|---|---|---|
| TTFT SLO | Configurable TTFT target (e.g. --slo-ttft=100ms) | planned |
| TPS SLO | Minimum token throughput target | planned |
| Cost estimation | Per-request and hourly cost based on model pricing | planned |
| Token counting | Count tokens from SSE stream content | planned |

### Multi-Model Comparison

```
vastar ai-bench --compare \
  --model gpt-4o --model claude-3.5 --model llama-3 \
  --prompt "Explain quantum computing" \
  http://localhost:4545/v1/chat/completions
```

Side-by-side output: TTFT, TPS, total tokens, cost per model. Useful for model selection decisions.

### Prompt Stress Testing

```
vastar ai-bench --prompt-sweep 10,100,1000,5000 \
  -c 50 http://localhost:4545/v1/chat/completions
```

Measure how latency and TPS scale with input prompt length. Identifies context window performance cliffs.

### AI Gateway Overhead

```
vastar ai-bench --overhead \
  --upstream http://localhost:4545/v1/chat/completions \
  --gateway http://localhost:3081/api/gw/trigger \
  -c 300 -n 3000
```

Measures gateway overhead at token level — not just HTTP latency but TTFT overhead, TPS degradation, and token pass-through accuracy.

### Guardrail/Safety Layer Benchmarking

Measure the cost of safety layers (prompt shields, guardrails, content filters) on inference performance:

| Metric | Without guardrail | With guardrail | Overhead |
|---|---|---|---|
| TTFT | 12ms | 28ms | +16ms |
| TPS | 85 tok/s | 78 tok/s | -8% |
| Total latency | 4.02s | 4.38s | +9% |

### RAG Pipeline Benchmark

```
vastar ai-bench --rag \
  --query-file queries.jsonl \
  http://localhost:8080/api/rag/query
```

Measures: retrieval latency, generation latency, total latency, context window utilization.

### Landscape: vastar vs existing AI benchmark tools

| Capability | hey/oha | LLMPerf | vLLM bench | GenAI-Perf | vastar ai-bench |
|---|---|---|---|---|---|
| HTTP load | fast | slow (Python) | no | no | fast (raw TCP) |
| TTFT measurement | no | yes | yes | yes | planned |
| TPS measurement | no | yes | yes | yes | planned |
| SSE streaming | no | yes | yes | yes | done |
| Multi-model compare | no | yes | no | no | planned |
| Cost estimation | no | yes | no | no | planned |
| High concurrency | varies | poor | moderate | moderate | strong |
| Generic + AI in one tool | no | no | no | no | yes |
| Binary size | 1-20 MB | Python env | Python env | Python env | ~1.2 MB |

---

## Phase 7: Data Layer (`vastar sql`, `vastar redis`, `vastar search`)

Benchmark databases, key-value stores, and search engines using their native wire protocols — not HTTP wrappers.

### SQL Databases (`vastar sql`)

Target: PostgreSQL, MySQL, CockroachDB, TiDB

```
vastar sql --dsn postgres://localhost:5432/mydb \
  --query "SELECT * FROM orders WHERE status = 'pending'" \
  -c 100 -n 10000
```

| Metric | Description |
|---|---|
| Queries/sec (QPS) | Total query throughput |
| Query latency (p50/p95/p99) | Per-query timing |
| Transaction throughput | BEGIN/COMMIT/ROLLBACK cycles per second |
| Connection pool saturation | Time waiting for pool slot |
| Read vs write split | Separate metrics for SELECT vs INSERT/UPDATE |

### Key-Value Stores (`vastar redis`)

Target: Redis, Memcached, DragonflyDB, etcd, FoundationDB

```
vastar redis --addr localhost:6379 \
  --pattern get-set --key-space 100000 --value-size 256 \
  -c 200 -n 100000
```

| Metric | Description |
|---|---|
| Ops/sec | GET, SET, pipeline throughput |
| Pipeline depth impact | Ops/sec vs pipeline batch size |
| Key-space pressure | Performance under large key count |
| Cluster failover latency | Time to recover after node failure |
| Memory overhead per key | Bytes used vs payload size |

### Vector Databases (`vastar vector`)

Target: Qdrant, Milvus, Weaviate, Pinecone, pgvector, ChromaDB

```
vastar vector --endpoint http://localhost:6333 \
  --dimensions 1536 --top-k 10 \
  -c 50 -n 5000
```

| Metric | Description |
|---|---|
| Insert throughput | Vectors/sec ingestion |
| Query latency vs recall | Accuracy tradeoff at speed |
| Dimension scaling | Performance vs embedding dimensions |
| Index build time | Time to index N vectors |
| Filtered search overhead | Metadata filter impact on latency |

### Time Series Databases (`vastar tsdb`)

Target: InfluxDB, TimescaleDB, QuestDB, ClickHouse

| Metric | Description |
|---|---|
| Write ingest rate | Points/sec write throughput |
| Query over time range | Latency vs range width |
| Downsampling speed | Aggregation query throughput |
| Cardinality impact | Performance vs tag cardinality |

### Search Engines (`vastar search`)

Target: Elasticsearch, OpenSearch, Meilisearch, Typesense

| Metric | Description |
|---|---|
| Index throughput | Documents/sec bulk indexing |
| Search latency | Query p50/p99 |
| Facet overhead | Aggregation cost |
| Autocomplete latency | Prefix search responsiveness |

### Graph Databases (`vastar graph`)

Target: Neo4j, ArangoDB, DGraph

| Metric | Description |
|---|---|
| Traversal depth vs latency | How deep before performance degrades |
| Relationship density impact | Dense vs sparse graph performance |
| Path-finding throughput | Shortest path queries/sec |

## Phase 8: Storage & Cache (`vastar s3`, `vastar cache`)

### Object Storage (`vastar s3`)

Target: S3, MinIO, GCS, Azure Blob

```
vastar s3 --endpoint http://localhost:9000 \
  --bucket bench --object-size 1MB \
  --pattern put-get -c 50 -n 1000
```

| Metric | Description |
|---|---|
| Upload throughput | MB/sec PUT operations |
| Download throughput | MB/sec GET operations |
| Multipart overhead | Chunked upload vs single PUT |
| List latency | Bucket listing at scale |
| First byte latency | Time to first byte on GET |

### Cache Systems (`vastar cache`)

Target: Redis, Memcached, Hazelcast, Varnish

| Metric | Description |
|---|---|
| Hit/miss ratio under load | Cache effectiveness at concurrency |
| Eviction rate | Items evicted/sec under memory pressure |
| Cluster replication lag | Primary → replica sync delay |
| Warm-up time | Time to reach target hit ratio |

### Distributed File Systems

Target: HDFS, Ceph, GlusterFS, SeaweedFS

| Metric | Description |
|---|---|
| Sequential read/write | Throughput MB/sec |
| Random IOPS | Small block random access |
| Replication latency | Write confirmation across replicas |

## Phase 9: Infrastructure (`vastar dns`, `vastar mesh`, `vastar edge`)

### API Gateway Overhead (`vastar gateway`)

Target: Kong, Envoy, Nginx, Traefik, VIL Gateway

```
vastar gateway --overhead \
  --upstream http://backend:8080 \
  --gateway http://kong:8000 \
  -c 300 -n 10000
```

| Metric | Description |
|---|---|
| Proxy overhead (ms) | Gateway latency - upstream latency |
| Max RPS before degradation | Throughput ceiling |
| Connection limit | Max concurrent through gateway |
| Plugin/middleware cost | Per-plugin latency contribution |

### Service Mesh (`vastar mesh`)

Target: Istio, Linkerd sidecar

| Metric | Description |
|---|---|
| Sidecar latency overhead | With vs without mesh |
| mTLS handshake cost | TLS overhead per connection |
| Control plane impact | Config propagation delay |

### DNS (`vastar dns`)

Target: CoreDNS, Route53, Cloudflare DNS

```
vastar dns --server 8.8.8.8 --domain api.example.com \
  -c 100 -n 10000
```

| Metric | Description |
|---|---|
| Resolution latency | DNS lookup time p50/p99 |
| Cache effectiveness | Cached vs uncached query time |
| NXDOMAIN rate | Failed resolution percentage |

### Serverless / Cold Start (`vastar serverless`)

Target: Lambda, Cloud Functions, Cloudflare Workers, Deno Deploy

| Metric | Description |
|---|---|
| Cold start latency | First invoke after idle |
| Warm invoke latency | Subsequent invoke |
| Concurrency scaling | Latency vs concurrent invocations |
| Memory size impact | Performance vs allocated memory |

### Load Balancer (`vastar lb`)

Target: HAProxy, Nginx, Envoy, ALB

| Metric | Description |
|---|---|
| Distribution fairness | Request spread across backends |
| Failover time | Detection + reroute latency |
| Health check overhead | Probe impact on throughput |

### Edge Compute (`vastar edge`)

Target: Cloudflare Workers, Fly.io, Deno Deploy, Vercel Edge

| Metric | Description |
|---|---|
| Cold start by region | Geographic cold start variance |
| Global latency distribution | P50/P99 per region |
| Edge cache hit ratio | Cache vs origin fetch |

## Phase 10: Emerging Systems (`vastar blockchain`, `vastar realtime`, `vastar wasm`)

### Blockchain RPC (`vastar blockchain`)

Target: Ethereum, Solana, Polygon, Avalanche nodes

| Metric | Description |
|---|---|
| RPC call latency | eth_call, eth_getBalance timing |
| Block subscription throughput | Events/sec on newHeads |
| Transaction submission rate | Pending tx/sec |
| Node sync status impact | Performance vs sync state |

### Realtime Sync (`vastar realtime`)

Target: Firebase, Supabase Realtime, Liveblocks, PartyKit

| Metric | Description |
|---|---|
| Sync latency | Write → observe on other client |
| Conflict resolution time | Concurrent write handling |
| Fan-out throughput | Broadcast to N subscribers |
| Reconnection recovery | Time to sync after disconnect |

### WASM Runtime (`vastar wasm`)

Target: Wasmtime, Wasmer, V8 isolates, Spin

| Metric | Description |
|---|---|
| Module startup time | Instantiation latency |
| Compute throughput | Operations/sec for CPU-bound tasks |
| Memory overhead | Per-instance memory cost |
| Cold vs warm instance | Pre-warmed pool benefit |

### ML Model Serving (non-LLM) (`vastar ml`)

Target: TorchServe, TFServing, Triton, ONNX Runtime, BentoML

| Metric | Description |
|---|---|
| Inference latency | Per-request model execution time |
| Batch throughput | Requests/sec with dynamic batching |
| GPU utilization | Compute saturation under load |
| Model switching overhead | Hot-swap cost between models |

### Image/Video Processing (`vastar media`)

Target: Image resize services, video transcoding, CLIP inference

| Metric | Description |
|---|---|
| Frames/sec | Processing throughput |
| Resolution scaling | Latency vs input resolution |
| Format conversion | Encode/decode overhead |

### Speech/Audio (`vastar audio`)

Target: Whisper, TTS engines, speech-to-text services

| Metric | Description |
|---|---|
| Real-time factor | Processing time vs audio duration |
| Concurrent stream limit | Max simultaneous transcriptions |
| Word error rate under load | Accuracy degradation at scale |

---

## Subcommand Summary

```
vastar sweep       Adaptive concurrency sweep — finds sweet-spot c (Phase 0)
vastar http        HTTP/1.1 load generator (current)
vastar ai-bench    LLM inference: TTFT, TPS, cost, multi-model
vastar grpc        gRPC unary + streaming
vastar ws          WebSocket connection + message load
vastar mqtt        MQTT pub/sub throughput
vastar kafka       Kafka producer/consumer bench
vastar nats        NATS pub/sub and request/reply
vastar amqp        RabbitMQ publish/consume
vastar quic        QUIC/HTTP/3 transport
vastar sql         PostgreSQL/MySQL wire protocol queries
vastar redis       Redis/Memcached key-value operations
vastar vector      Vector database insert + search
vastar tsdb        Time series write + range query
vastar search      Elasticsearch/Meilisearch index + search
vastar graph       Graph traversal + path-finding
vastar s3          Object storage upload/download
vastar cache       Cache hit/miss ratio under load
vastar dns         DNS resolution latency
vastar gateway     API gateway overhead measurement
vastar mesh        Service mesh sidecar overhead
vastar serverless  Cold start + warm invoke
vastar edge        Edge compute latency by region
vastar lb          Load balancer fairness + failover
vastar blockchain  RPC node latency + tx throughput
vastar realtime    Realtime sync latency
vastar wasm        WASM runtime startup + compute
vastar ml          ML model serving inference
vastar media       Image/video processing throughput
vastar audio       Speech/audio processing bench
vastar tcp         Raw TCP echo throughput
vastar udp         UDP datagram throughput
```

All subcommands share the same core engine: adaptive FuturesUnordered topology, colored progress bar, SLO Insight, percentile distribution, and histogram.

---

## Non-Goals

- **Browser simulation** — use Playwright/Puppeteer for real browser rendering
- **API functional testing** — use Hurl, Bruno, or Postman for assertion-based testing
- **Traffic replay** — use GoReplay or tcpreplay for production traffic reproduction
- **APM/monitoring** — use VIL Observer, Grafana, or Datadog for ongoing monitoring

---

## Contributing

We welcome contributions for any roadmap item. Start with Phase 1 (HTTP feature parity) as these are the most immediately useful. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
