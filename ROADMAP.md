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
