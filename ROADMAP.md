# vastar Roadmap

vastar is experimental. The roadmap below lists what we intend to build,
ordered by priority. No dates. An item ships when it is ready and when
we have validated it against real workloads; the ordering is what we
work on first when we have time, not a delivery schedule.

The long-term vision is a **universal benchmark tool** — one binary
covering HTTP, streaming, gRPC, WebSockets, databases, message queues,
storage, edge, AI inference, and more — all sharing the same core
engine, progress UX, and SLO analysis. That scope is intentional, and
we know it is ambitious. We will not rush to cover surface area at the
cost of correctness. If a subcommand is listed here but not yet shipped,
it does not exist yet.

---

## Priority 1 — Fix what we already ship

Before expanding scope, vastar needs to close the gaps that cross-tool
benchmarks exposed. These are listed ahead of any new-protocol work.

### Known bugs

| Bug | Description | Workaround | Status |
|---|---|---|---|
| `-H` does not override `-T` default | `-H "Content-Type: application/json"` adds a second header instead of overriding the default; some servers pick wrong one → 400. | Use `-T` instead of overriding via `-H`. | open |
| p99 tail degradation at ≥ 1 MB streaming | vastar matches or beats `wrk` on throughput at 1 – 2 MB chunked streaming, but p99 blows out (1.2 s at 1 MB, 2.7 s at 2 MB) compared to `oha` (190 ms, 260 ms). Likely root cause: BufReader capacity (32 KB), lack of flow control, per-chunk `Bytes::copy_from_slice` allocation. | None; for p99-sensitive 1 MB+ streaming, `oha` is currently a better choice. | open |
| 100 KB non-streaming regression vs `hey` | At 100 KB `Content-Length` responses, vastar's throughput falls behind `hey`. Likely the same reader path as above. | Use `oha` or `hey` for this workload size. | open |
| Duration-mode deadline behavior (3 gaps) | (A) Pre-connect time ate into the requested duration window (5-10% skew at high c); (B) drain time past the deadline had no accounting; (C) a single slow in-flight request could hijack elapsed up to `timeout` (30s default). | — | **fixed (post-v0.3.0)** — Timer now starts after pre-connect; `Drain: Xms past deadline` surfaced in report; new `--drain-cap` flag (default 2s) bounds overshoot and counts aborted in-flight as `[drain-aborted]`. Matches `oha`'s reporting shape. |

### Missing HTTP features

| Feature | Priority | Notes |
|---|---|---|
| `-H` override semantics | critical | Deduplicate headers; later `-H` wins over earlier `-T` / `-H`. |
| **TLS / HTTPS** | high | Via `rustls`. Non-optional for production-endpoint benchmarking. |
| **HTTP/2** | high | Via `h2` crate. Preserve raw-TCP philosophy for HTTP/1.1 path. |
| ALPN negotiation | high | HTTP/1.1 ↔ HTTP/2 auto-select. |
| Certificate verification | high | System CA + custom certs via `rustls-native-certs`. |
| Client certificates (mTLS) | medium | |
| **Coordinated-omission correction** | high | Gil Tene / HDR-style correction. `wrk2`'s killer feature; necessary for true SLO testing. |
| Follow redirects | medium | Configurable. |
| HTTP proxy | medium | `HTTP_PROXY` / `HTTPS_PROXY`. |
| Output formats: JSON, CSV, NDJSON | medium | Stable schema for CI pipelines. |
| Unix socket | low | `oha` has this. |
| Connect-to (host override) | low | |
| AWS SigV4 auth | low | |
| Random URL generation | low | |
| Multiple URLs from file | medium | |
| Disable compression | low | |
| Rate limiting (QPS) | medium | Partial support exists; needs polish. |

---

## Priority 2 — `vastar sweep` (shipped, v0.2.0+)

Already released. Adaptive concurrency sweep with knee detection,
disqualification gates, JSON output, paired-mode overhead measurement.
Documented in the README.

Improvements still planned:

- Reference-curve caching with TTL so a sweep can reuse upstream
  measurements captured on a different host.
- Multi-concurrency comparison view (overlay curves from different
  builds / versions).
- Auto-warm on first run (first point's results discarded to avoid
  cold-start skew).

---

## Priority 3 — AI Insight via CLI

**New feature class.** Integrate large-language-model analysis of
benchmark results directly into vastar's CLI. After a run completes,
the user can request an AI interpretation of the distribution, tail,
phase breakdown, and SLO status — in natural language, with concrete
investigation suggestions.

### Sketch

```bash
# One-time: configure provider + key + workload context + baseline
vastar insight config \
  --provider anthropic \
  --api-key-env ANTHROPIC_API_KEY \
  --model claude-3-5-sonnet \
  --context-docs ./docs/architecture.md,./docs/slo.md,./CHANGELOG.md \
  --baseline ./baselines/v1.2.0.json \
  --slo-target "p99<200ms,error<0.1%" \
  --service-name "payment-gateway" \
  --notes "3-node VFlow cluster behind Nginx, PostgreSQL 15, Redis 7"

# Per-run: attach AI analysis to the standard report
vastar -z 30s -c 200 --insight http://localhost:8080/api

# Override context per-run (layered on top of config)
vastar -z 30s -c 200 \
  --insight \
  --context-docs ./docs/recent-change.md \
  --baseline ./baselines/last-good.json \
  http://localhost:8080/api
```

or pipe-style:

```bash
vastar -z 30s -c 200 http://localhost:8080/api -o json \
  | vastar insight analyze \
      --context-docs ./docs/architecture.md \
      --baseline ./baselines/v1.2.0.json \
      --notes "AI gateway, SLO p99<200ms"
```

### Context inputs

The insight command accepts multiple context sources. All are optional;
more context produces better analysis.

| Input | Flag | Purpose |
|---|---|---|
| Context documents | `--context-docs <paths>` | Architecture diagrams, SLO docs, runbooks, recent CHANGELOG entries, incident post-mortems. Comma-separated paths or glob patterns. Markdown, plain text, PDF supported. |
| Baseline result | `--baseline <path>` | Prior vastar JSON output (e.g. last known-good run). Enables regression diagnosis and diff commentary. |
| Multiple baselines | `--baseline-dir <path>` | Directory of historical runs; AI picks relevant ones based on configuration similarity. |
| Service metadata | `--service-name`, `--environment`, `--version` | Identifies what is under test across runs. |
| SLO definition | `--slo-target` | Explicit SLO used for pass/fail verdict (same syntax as the existing `--slo-target` flag). |
| Free-form notes | `--notes <text>` | Short paragraph on recent changes, suspected issues, or what the operator wants the AI to focus on. |
| Topology hints | `--topology <path>` | Optional machine-readable infra description (YAML/JSON): hop count, proxies, datastores, regions. |

Context is assembled at analysis time, token-budgeted, and sent to the
configured provider alongside the run's numeric report. Oversized
documents are summarized locally before being sent; full bytes never
leave the machine unless they fit the budget.

### Capabilities

| Capability | Description |
|---|---|
| Provider-agnostic | OpenAI, Anthropic, Gemini, local (Ollama) — user brings their own API key. |
| Context-aware | Reads architecture docs, SLO definitions, topology hints, operator notes. |
| Baseline-aware | Compares current run against a named baseline and explains regressions line-by-line. |
| Distribution analysis | Interpret histogram shape, bimodality, long-tail characteristics. |
| Phase diagnosis | Identify whether bottleneck is connect, write, wait, or read phase. |
| SLO verdict | Pass/fail/at-risk relative to user-defined or auto-inferred SLO. |
| Investigation suggestions | Concrete next steps grounded in the supplied docs ("section 4.2 of architecture.md mentions a 200ms budget for this hop — current p99 is 340ms"). |
| Comparison mode | Diff two runs, explain regression with reference to CHANGELOG entries between the two versions. |
| Offline mode | Store last N results locally for later `vastar insight review`. |
| Redaction | `--redact` strips hostnames, IPs, and custom patterns before sending context to the provider. |

### Non-goals

- Vastar does not host a model. Users bring their own provider.
- Vastar does not train on user bench data.
- AI insight is strictly additive — never replaces the numeric report.

---

## Priority 4 — Own the streaming niche

This is where vastar's raw-TCP design has a natural fit. Deep metrics
beyond "RPS + percentiles".

### SSE / NDJSON / chunked streaming

| Feature | Status |
|---|---|
| Chunked-transfer drain | done |
| **Per-chunk timing** (inter-chunk latency) | planned |
| **Time to first byte (TTFB)** | partial |
| **Final-chunk latency** | planned |
| Chunk count distribution (p50/p90/p99) | planned |
| Stream abandonment modelling | planned |
| Reconnection + `Last-Event-ID` behavior | planned |
| SSE dialect detection (OpenAI, Anthropic, Cohere, Gemini, W3C) | done |

### LLM inference metrics

Building on streaming. `vastar ai-bench` subcommand.

| Metric | Description |
|---|---|
| **Time to First Token (TTFT)** | Latency from request to first SSE token chunk. |
| **Tokens per Second (TPS)** | Steady-state token throughput. |
| **Inter-Token Latency (ITL)** | Time between consecutive tokens. |
| Total tokens per response | Parsed from SSE content. |
| Token cost estimation | Per-request and hourly, based on published provider pricing. |
| Multi-model comparison | Side-by-side TTFT/TPS/cost across models. |
| Prompt-length sweep | Latency/TPS scaling vs input prompt size. |
| Gateway overhead (token-level) | TTFT / TPS degradation introduced by a gateway hop. |
| Guardrail / safety-layer cost | Same, but specifically for content-filter middleware. |

---

## Priority 5 — gRPC + WebSocket

Natural extensions of streaming focus. Both share the "long-lived
connection, many messages" shape where vastar's engine has a fit.

### gRPC (`vastar grpc`)

| Feature | Description |
|---|---|
| Unary RPC | Single request-response. |
| Server streaming | Server sends a stream of messages. |
| Client streaming | Client sends a stream of messages. |
| Bidirectional streaming | Both sides stream. |
| Protobuf payload | Load `.proto` files, generate messages. |
| Server reflection | Discover services without `.proto`. |

### WebSocket (`vastar ws`)

| Feature | Description |
|---|---|
| Connection load | N concurrent WebSocket connections. |
| Message throughput | M messages/sec across the fleet. |
| Echo benchmark | Round-trip latency. |
| Binary + text frames | Both frame types. |
| Ping/pong overhead | Keep-alive cost. |

---

## Priority 6 — QUIC / HTTP/3

| Feature | Description |
|---|---|
| QUIC transport | UDP 0-RTT. |
| HTTP/3 request stream | Over QUIC streams. |
| Connection migration testing | Migrate under load. |

---

## Priority 7 — Observability + CI integration

| Feature | Description |
|---|---|
| Prometheus push | Push benchmark metrics to Pushgateway. |
| OpenTelemetry export | Emit OTLP for downstream dashboards. |
| **CI/CD exit codes** | Return non-zero on SLO failure for pipeline gates. |
| Custom absolute SLOs | Already shipped (`--slo-target`). |
| Scenario scripting | Multi-step workflows (login → browse → checkout). |
| Flamegraph of the tool itself | Sanity-check vastar's own CPU during runs. |

---

## Priority 8 — Data layer benchmarks

These exist in dedicated tools already (`sysbench`, `redis-benchmark`,
`ann-benchmarks`, etc). Our opportunity is consistency — one CLI, one
output schema, one SLO model. If any of these ship in vastar, they need
to be at least as accurate as the dedicated tool for that class.

### SQL (`vastar sql`)

Targets: PostgreSQL, MySQL, CockroachDB, TiDB. Queries/sec, per-query
latency, transaction throughput, connection-pool saturation, read/write
split.

### Key-Value (`vastar redis`)

Targets: Redis, Memcached, DragonflyDB, etcd, FoundationDB. Ops/sec,
pipeline depth impact, key-space pressure, cluster failover, memory
overhead.

### Vector DB (`vastar vector`)

Targets: Qdrant, Milvus, Weaviate, Pinecone, pgvector, ChromaDB. Insert
throughput, recall-vs-latency tradeoff, dimension scaling, filtered
search overhead.

### Time-Series (`vastar tsdb`)

Targets: InfluxDB, TimescaleDB, QuestDB, ClickHouse. Write ingest
rate, query-over-range latency, downsampling throughput, cardinality
impact.

### Search (`vastar search`)

Targets: Elasticsearch, OpenSearch, Meilisearch, Typesense. Index
throughput, query latency, facet overhead, autocomplete latency.

### Graph (`vastar graph`)

Targets: Neo4j, ArangoDB, DGraph. Traversal depth, relationship
density, path-finding throughput.

---

## Priority 9 — Message queues + eventing

### Target classes

| Protocol | Subcommand | Description |
|---|---|---|
| MQTT | `vastar mqtt` | pub/sub throughput, QoS levels. |
| NATS | `vastar nats` | pub/sub + request/reply. |
| Kafka | `vastar kafka` | producer throughput, consumer lag. |
| AMQP (RabbitMQ) | `vastar amqp` | publish/consume. |
| RSocket | `vastar rsocket` | request-response, fire-and-forget, streaming. |
| GraphQL | `vastar graphql` | query/mutation load with variable payloads. |

---

## Priority 10 — Storage + cache

### Object storage (`vastar s3`)

Targets: S3, MinIO, GCS, Azure Blob. Upload/download throughput,
multipart overhead, list latency, first-byte latency.

### Cache (`vastar cache`)

Targets: Redis, Memcached, Hazelcast, Varnish. Hit/miss under load,
eviction rate, cluster replication lag, warm-up time.

### Distributed file systems

Targets: HDFS, Ceph, GlusterFS, SeaweedFS. Sequential throughput,
random IOPS, replication latency.

---

## Priority 11 — Infrastructure

### API gateway / service mesh (`vastar gateway`, `vastar mesh`)

Proxy overhead, max RPS before degradation, plugin/middleware cost,
sidecar latency, mTLS handshake cost, control-plane impact.

### DNS (`vastar dns`)

Resolution latency, cache effectiveness, NXDOMAIN rate.

### Serverless / cold start (`vastar serverless`)

Lambda, Cloud Functions, Cloudflare Workers, Deno Deploy. Cold start,
warm invoke, concurrency scaling, memory-size impact.

### Load balancer (`vastar lb`)

HAProxy, Nginx, Envoy, ALB. Distribution fairness, failover time,
health-check overhead.

### Edge compute (`vastar edge`)

Cloudflare Workers, Fly.io, Deno Deploy, Vercel Edge. Geographic cold
start variance, per-region P99, edge cache hit ratio.

---

## Priority 12 — Emerging systems

### Blockchain RPC (`vastar blockchain`)

Ethereum, Solana, Polygon, Avalanche. RPC call latency, subscription
throughput, tx submission rate, sync-state impact.

### Realtime sync (`vastar realtime`)

Firebase, Supabase Realtime, Liveblocks, PartyKit. Sync latency,
conflict resolution, fan-out throughput, reconnection recovery.

### WASM runtime (`vastar wasm`)

Wasmtime, Wasmer, V8 isolates, Spin. Module startup, compute
throughput, memory overhead, cold-vs-warm instance.

### Non-LLM ML serving (`vastar ml`)

TorchServe, TFServing, Triton, ONNX Runtime, BentoML. Inference
latency, batch throughput, GPU utilization, model-switch overhead.

### Media processing (`vastar media`)

Image resize, video transcoding, CLIP inference. Frames/sec,
resolution scaling, format conversion overhead.

### Audio (`vastar audio`)

Whisper, TTS engines, speech-to-text. Real-time factor, concurrent
stream limit, WER degradation under load.

### Raw transport

`vastar tcp` — generic TCP echo/throughput.
`vastar udp` — datagram throughput measurement.

---

## Priority 13 — Ecosystem

Only worth investing in once the core tool is stable enough that
ecosystem additions are not papering over gaps.

| Feature | Description |
|---|---|
| `vastar-report` | HTML report generator from JSON output. |
| `vastar-compare` | Diff two JSON runs, highlight regressions. |
| Distributed mode | Coordinator + agents across multiple machines. |
| IDE plugin | VS Code extension with inline results. |
| GitHub Action | Run benchmarks in CI, comment results on PR. |

---

## Subcommand summary

All planned subcommands, ordered by priority category. Items without
a status label are planned but not shipped.

```
vastar sweep       Adaptive concurrency sweep            (P2 — shipped)
vastar <flat>      HTTP/1.1 load generator               (current, v0.3.0)
vastar insight     AI-assisted report interpretation     (P3)
vastar ai-bench    LLM inference: TTFT, TPS, cost        (P4)
vastar grpc        gRPC unary + streaming                (P5)
vastar ws          WebSocket connection + message load   (P5)
vastar quic        QUIC / HTTP/3                         (P6)
vastar sql         SQL wire protocol                     (P8)
vastar redis       Redis / KV ops                        (P8)
vastar vector      Vector DB insert + search             (P8)
vastar tsdb        Time-series write + query             (P8)
vastar search      Search-engine index + query           (P8)
vastar graph       Graph traversal                       (P8)
vastar mqtt        MQTT pub/sub                          (P9)
vastar kafka       Kafka producer/consumer               (P9)
vastar nats        NATS pub/sub + req/reply              (P9)
vastar amqp        AMQP publish/consume                  (P9)
vastar rsocket     RSocket patterns                      (P9)
vastar graphql     GraphQL query/mutation load           (P9)
vastar s3          Object storage upload/download        (P10)
vastar cache       Cache behaviour under load            (P10)
vastar gateway     API-gateway overhead                  (P11)
vastar mesh        Service-mesh sidecar overhead         (P11)
vastar dns         DNS resolution                        (P11)
vastar serverless  Cold start + warm invoke              (P11)
vastar lb          Load-balancer fairness + failover     (P11)
vastar edge        Edge-compute latency by region        (P11)
vastar blockchain  RPC latency + tx throughput           (P12)
vastar realtime    Realtime sync latency                 (P12)
vastar wasm        WASM runtime startup + compute        (P12)
vastar ml          ML model serving                      (P12)
vastar media       Image / video processing              (P12)
vastar audio       Speech / audio processing             (P12)
vastar tcp         Raw TCP echo throughput               (P12)
vastar udp         UDP datagram throughput               (P12)
```

All subcommands will share the same core engine: adaptive
`FuturesUnordered` topology, colour progress bar, SLO Insight,
percentile distribution, histogram, and (via `vastar insight`)
AI-assisted analysis.

---

## Non-goals

- **Browser simulation** — Playwright / Puppeteer for real browser rendering.
- **API functional testing** — Hurl, Bruno, Postman for assertion-based testing.
- **Traffic replay** — GoReplay / tcpreplay.
- **APM / monitoring** — Grafana, Datadog, your Observer of choice.
- **Model training** — vastar's AI insight is inference-only; users
  bring their own provider.

---

## Contributing

The roadmap is long and the core team is small. We welcome
contributions — the priorities above indicate where the core team's
attention goes first, not the only places work is welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
