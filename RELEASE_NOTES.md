# Release Notes - v1.0.3

**Release Date**: December 26, 2025

## Production-Ready Features

### State Persistence
- Resume from last position after restart
- No certificate loss during maintenance or updates
- JSON-based state file with atomic writes
- Configurable via `ct_log.state_file` or `CERTSTREAM_CT_LOG_STATE_FILE`

```yaml
ct_log:
  state_file: "/data/state.json"
```

### Rate Limiting
- Configurable requests per second with burst allowance
- Uses tower-governor for efficient rate limiting
- Per-client tracking

```yaml
rate_limit:
  enabled: true
  per_second: 10
  burst_size: 20
```

### Connection Limiting
- Maximum total connections across all protocols
- Per-IP connection limits to prevent abuse
- DashMap-based concurrent tracking

```yaml
connection_limit:
  enabled: true
  max_connections: 10000
  per_ip_limit: 100
```

### Token Authentication
- Bearer token based authentication
- Multiple tokens supported
- Configurable header name

```yaml
auth:
  enabled: true
  tokens:
    - "secret-token-1"
    - "secret-token-2"
  header_name: "Authorization"
```

### Hot Reload Configuration
- Config changes apply without restart
- File system watcher detects updates
- Rate limit, connection limit, and auth settings reloadable

```yaml
hot_reload:
  enabled: true
```

### CT Log Health Management
- Automatic retry with exponential backoff (backon crate)
- Circuit breaker pattern for unhealthy logs
- Configurable healthy/unhealthy thresholds
- Health check intervals for recovery

```yaml
ct_log:
  retry_max_attempts: 3
  retry_initial_delay_ms: 100
  retry_max_delay_ms: 5000
  request_timeout_secs: 30
  healthy_threshold: 3
  unhealthy_threshold: 5
  health_check_interval_secs: 60
```

---

## Dependencies Added

- `tower_governor` 0.8 - Rate limiting middleware
- `backon` 1 - Retry with exponential backoff
- `dashmap` 6 - Concurrent hash maps
- `notify` 7 - File system watching for hot reload

---

## Breaking Changes

None. Full backward compatibility with v1.0.1.

---

## Migration Guide

1. Update Docker image to v1.0.3
2. Optionally enable new features via config or environment variables
3. For state persistence, mount a volume for the state file

```bash
docker run -d \
  -p 8080:8080 \
  -v certstream-state:/data \
  -e CERTSTREAM_CT_LOG_STATE_FILE=/data/state.json \
  reloading01/certstream-server-rust:1.0.3
```

---
---

# Release Notes - v1.0.1

**Release Date**: December 26, 2025

## Major Performance Improvements

### Pre-Serialization Architecture
- Messages are now serialized once and shared across all clients using `Arc<PreSerializedMessage>`
- **Before**: N clients = N serializations per message
- **After**: N clients = 1 serialization + N cheap Arc clones
- **Result**: ~80% CPU reduction under high client load

### Zero-Copy String Handling
- Static strings use `Cow<'static, str>` to avoid allocations
- Source information shared via `Arc<Source>` across all messages from same CT log
- Domain lists use `SmallVec<[String; 4]>` to avoid heap allocation for small lists

### Optimized Data Structures
- `Arc<str>` for log names and URLs (shared, immutable)
- Pre-allocated capacities for HashMaps and Vecs
- Efficient fingerprint and serial number formatting with pre-sized buffers

---

## New Features

### Multi-Protocol Support

#### SSE (Server-Sent Events) - `/sse`
- Lightweight HTTP-based streaming
- Native browser support without WebSocket libraries
- Automatic reconnection via browser EventSource
- Lower overhead than WebSocket for read-only streams

```bash
curl -N "http://localhost:8080/sse?stream=lite"
```

#### Raw TCP Streaming
- Minimal protocol overhead (JSON + newline)
- Ideal for high-performance consumers
- Stream type selection via first byte: `f` = full, `d` = domains, default = lite

```bash
nc localhost 8081
```

### Enhanced Metrics

New Prometheus metrics for monitoring:
- `certstream_ws_connections_total` - Total WebSocket connections
- `certstream_ws_connections_full` - Full stream connections
- `certstream_ws_connections_lite` - Lite stream connections
- `certstream_ws_connections_domains` - Domains-only connections
- `certstream_sse_connections` - SSE connections
- `certstream_tcp_connections` - TCP connections
- `certstream_ct_logs_count` - Active CT logs
- `certstream_messages_sent` - Total messages sent
- `certstream_ws_messages_lagged` - Lagged/skipped messages

---

## Configuration Changes

### New Protocol Configuration

```yaml
protocols:
  websocket: true    # Default: true
  sse: true          # Default: false
  tcp: true          # Default: false
  tcp_port: 8081     # Default: HTTP_PORT + 1
```

### Environment Variables

```bash
CERTSTREAM_WS_ENABLED=true
CERTSTREAM_SSE_ENABLED=true
CERTSTREAM_TCP_ENABLED=true
CERTSTREAM_TCP_PORT=8081
```

---

## API Endpoints

| Endpoint | Protocol | Description |
|----------|----------|-------------|
| `/` | WebSocket | Lite stream (no DER/chain) |
| `/full-stream` | WebSocket | Full stream with DER + chain |
| `/domains-only` | WebSocket | Domains only |
| `/sse` | SSE | SSE stream (query: `stream=full\|lite\|domains`) |
| `/health` | HTTP | Health check |
| `/metrics` | HTTP | Prometheus metrics |
| `/example.json` | HTTP | Example message format |

---

## Breaking Changes

None. This release maintains full backward compatibility with v1.0.0.

---

## Dependencies Updated

- Added: `bytes` 1.10 - Zero-copy buffer handling
- Added: `parking_lot` 0.12 - Faster mutexes
- Added: `smallvec` 1.15 - Stack-allocated small vectors
- Added: `tokio-stream` 0.1 - Async stream utilities
- Updated: `serde` with `rc` feature for Arc serialization

---

## Performance Benchmarks

### vs Go (1,000 concurrent clients, same machine)

| Metric | Rust | Go |
|--------|------|-----|
| Memory (idle) | ~12 MB | ~100 MB |
| Memory (avg under load) | 22 MB | 254 MB |
| CPU (avg under load) | ~15% | ~34% |
| Latency (avg) | 3.4ms | 31ms |
| Latency (min) | 0.16ms | 1.7ms |
| Throughput | 677K msg | 267K msg |

**Result**: 12x less memory, 9x faster latency, 2.5x higher throughput.

### v1.0.0 vs v1.0.1

| Metric | v1.0.0 | v1.0.1 | Improvement |
|--------|--------|--------|-------------|
| Memory (idle) | ~15MB | ~12MB | 20% ↓ |
| Latency | ~5ms | 0.16ms | 97% ↓ |
| Max concurrent clients | ~5000 | ~50000+ | 10x ↑ |

---

## Migration Guide

1. Update your `config.yaml` to include new protocol settings if needed
2. No code changes required for existing WebSocket clients
3. Consider switching to SSE for browser-based monitoring dashboards
4. Use TCP for highest-performance data pipelines

---

## Docker

```bash
docker build -t certstream-server-rust .
docker run -p 8080:8080 -p 8081:8081 \
  -e CERTSTREAM_SSE_ENABLED=true \
  -e CERTSTREAM_TCP_ENABLED=true \
  certstream-server-rust
```

---
---

# Release Notes - v1.0.0

**Release Date**: December 26, 2025

## Initial Release

First public release of certstream-server-rust - a high-performance Certificate Transparency log streaming server written in Rust.

## Features

- **Real-time CT Log Streaming**: Stream certificates from 60+ CT logs globally via WebSocket
- **Three Streaming Modes**:
  - `/` - Lite stream (default, no DER/chain data)
  - `/full-stream` - Full certificate data with DER and chain
  - `/domains-only` - Domain names only
- **Configuration**:
  - YAML configuration file support
  - Environment variable override
  - Custom CT logs support
- **Monitoring**:
  - Prometheus metrics endpoint (`/metrics`)
  - Health check endpoint (`/health`)
  - Example message format (`/example.json`)
- **Security**: Optional TLS/HTTPS support
- **Deployment**: Docker ready with multi-stage build

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `ws://host:8080/` | Lite stream |
| `ws://host:8080/full-stream` | Full stream |
| `ws://host:8080/domains-only` | Domains only |
| `http://host:8080/health` | Health check |
| `http://host:8080/metrics` | Prometheus metrics |

## Performance

| Metric | Value |
|--------|-------|
| Memory (idle) | ~15 MB |
| Binary size | ~5.4 MB |
| Startup time | <100ms |

## Supported CT Logs

Monitors all major Certificate Transparency logs including:
- Google (Argon, Xenon, Solera, Submariner)
- Cloudflare (Nimbus)
- DigiCert (Wyvern, Sphinx)
- Sectigo (Elephant, Tiger, Dodo)
- Let's Encrypt (Sapling, Clicky)
- TrustAsia, Nordu, and more...

## Compatibility

Drop-in replacement for:
- [certstream-server](https://github.com/CaliDog/certstream-server) (Elixir)
- [certstream-server-go](https://github.com/d-Rickyy-b/certstream-server-go) (Go)

Works with existing certstream clients (Python, JavaScript, Go).

---