# Vastar Roadmap

## Current (v0.1.x) — HTTP/1.1 Load Generator

vastar currently supports HTTP/1.1 with raw TCP. This roadmap outlines planned protocol support, missing features compared to hey/oha, and the long-term vision as a **multi-protocol load generator**.

---

## Phase 1: HTTP Feature Parity

Missing features that hey and/or oha already support.

| Feature | hey | oha | vastar | Priority |
|---|---|---|---|---|
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

---

## Non-Goals

- **Browser simulation** — use Playwright/Puppeteer for real browser rendering
- **API functional testing** — use Hurl, Bruno, or Postman for assertion-based testing
- **Traffic replay** — use GoReplay or tcpreplay for production traffic reproduction
- **APM/monitoring** — use VIL Observer, Grafana, or Datadog for ongoing monitoring

---

## Contributing

We welcome contributions for any roadmap item. Start with Phase 1 (HTTP feature parity) as these are the most immediately useful. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
