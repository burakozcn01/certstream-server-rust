# certstream-server-rust

Certstream server written in Rust. Streams SSL/TLS certificates from Certificate Transparency logs in real-time via WebSocket, SSE, and TCP.

[![Docker Hub](https://img.shields.io/docker/pulls/reloading01/certstream-server-rust.svg)](https://hub.docker.com/r/reloading01/certstream-server-rust)
[![Docker Image Size](https://img.shields.io/docker/image-size/reloading01/certstream-server-rust/latest)](https://hub.docker.com/r/reloading01/certstream-server-rust)
[![Rust](https://img.shields.io/badge/rust-1.86%2B-orange.svg)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Docs](https://img.shields.io/badge/docs-website-blue.svg)](https://burakozcn01.github.io/certstream-server-rust/)

## What is Certstream?

Certstream aggregates certificates from Certificate Transparency logs and streams them in real-time. This is a Rust rewrite that works as a drop-in replacement for [certstream-server](https://github.com/CaliDog/certstream-server) (Elixir) and [certstream-server-go](https://github.com/d-Rickyy-b/certstream-server-go).

### Why Rust?

- ~12MB memory idle, ~19MB under load
- Sub-millisecond latency (tested 0.33ms min)
- Handles 50,000+ concurrent connections
- ~6% CPU with 500+ clients
- Single binary, no dependencies

## Features

- WebSocket, SSE, and raw TCP streaming
- Pre-serialized messages (serialize once, broadcast to all)
- Works with existing certstream clients
- Prometheus metrics
- TLS support
- 60+ CT logs monitored

### v1.0.3 New Features

- **State Persistence**: Resume from last position after restart
- **Rate Limiting**: Configurable requests per second with burst
- **Connection Limiting**: Max total and per-IP connections
- **Token Authentication**: Bearer token based auth
- **Hot Reload**: Config changes without restart
- **CT Log Health**: Automatic retry, circuit breaker, health checks

## Quick Start

```bash
# Basic
docker run -d -p 8080:8080 reloading01/certstream-server-rust:latest

# All protocols enabled
docker run -d -p 8080:8080 -p 8081:8081 \
  -e CERTSTREAM_SSE_ENABLED=true \
  -e CERTSTREAM_TCP_ENABLED=true \
  reloading01/certstream-server-rust:latest

# With state persistence (resume after restart)
docker run -d -p 8080:8080 \
  -v certstream-state:/data \
  -e CERTSTREAM_CT_LOG_STATE_FILE=/data/state.json \
  reloading01/certstream-server-rust:latest

# Production setup
docker run -d \
  --name certstream \
  --restart unless-stopped \
  -p 8080:8080 \
  -p 8081:8081 \
  -v certstream-state:/data \
  -e CERTSTREAM_CT_LOG_STATE_FILE=/data/state.json \
  -e CERTSTREAM_SSE_ENABLED=true \
  -e CERTSTREAM_TCP_ENABLED=true \
  -e CERTSTREAM_RATE_LIMIT_ENABLED=true \
  -e CERTSTREAM_CONNECTION_LIMIT_ENABLED=true \
  reloading01/certstream-server-rust:latest
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_HOST` | 0.0.0.0 | Bind address |
| `CERTSTREAM_PORT` | 8080 | HTTP/WebSocket port |
| `CERTSTREAM_LOG_LEVEL` | info | debug, info, warn, error |
| `CERTSTREAM_BUFFER_SIZE` | 1000 | Broadcast buffer |

**Protocols**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_WS_ENABLED` | true | Enable WebSocket |
| `CERTSTREAM_SSE_ENABLED` | false | Enable SSE |
| `CERTSTREAM_TCP_ENABLED` | false | Enable TCP |
| `CERTSTREAM_TCP_PORT` | 8081 | TCP port |

**Rate Limiting**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_RATE_LIMIT_ENABLED` | false | Enable rate limiting |
| `CERTSTREAM_RATE_LIMIT_PER_SECOND` | 10 | Requests per second |
| `CERTSTREAM_RATE_LIMIT_BURST_SIZE` | 20 | Burst allowance |

**Connection Limiting**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_CONNECTION_LIMIT_ENABLED` | false | Enable connection limits |
| `CERTSTREAM_CONNECTION_LIMIT_MAX_CONNECTIONS` | 10000 | Max total connections |
| `CERTSTREAM_CONNECTION_LIMIT_PER_IP_LIMIT` | 100 | Max per IP |

**Authentication**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_AUTH_ENABLED` | false | Enable token auth |
| `CERTSTREAM_AUTH_TOKENS` | - | Comma-separated tokens |
| `CERTSTREAM_AUTH_HEADER_NAME` | Authorization | Auth header |

**CT Log Settings**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_CT_LOG_STATE_FILE` | - | State file path |
| `CERTSTREAM_CT_LOG_RETRY_MAX_ATTEMPTS` | 3 | Max retry attempts |
| `CERTSTREAM_CT_LOG_REQUEST_TIMEOUT_SECS` | 30 | Request timeout |
| `CERTSTREAM_CT_LOG_BATCH_SIZE` | 256 | Entries per batch |

**Hot Reload**

| Variable | Default | Description |
|----------|---------|-------------|
| `CERTSTREAM_HOT_RELOAD_ENABLED` | false | Enable hot reload |
| `CERTSTREAM_HOT_RELOAD_WATCH_PATH` | - | Config file to watch |

### Build from Source

```bash
# Docker
docker build -t certstream-server-rust .
docker run -d -p 8080:8080 certstream-server-rust

# Cargo
cargo build --release
./target/release/certstream-server-rust

# Docker Compose
docker compose up -d
```

## API

### WebSocket

| Endpoint | Description |
|----------|-------------|
| `ws://host:8080/` | Lite stream (no DER/chain) |
| `ws://host:8080/full-stream` | Full data with DER and chain |
| `ws://host:8080/domains-only` | Just domain names |

### SSE

| Endpoint | Description |
|----------|-------------|
| `http://host:8080/sse` | Lite (default) |
| `http://host:8080/sse?stream=full` | Full |
| `http://host:8080/sse?stream=domains` | Domains only |

### TCP

Connect to port `8081`. Send `f` for full, `d` for domains, or nothing for lite.

### HTTP

| Endpoint | Description |
|----------|-------------|
| `/health` | Health check |
| `/metrics` | Prometheus metrics |
| `/example.json` | Example message |

## Client Examples

See [examples/](examples/) directory for demo scripts.

### Python

```bash
pip install certstream
```

```python
import certstream

def callback(message, context):
    if message['message_type'] == 'certificate_update':
        domains = message['data']['leaf_cert']['all_domains']
        print(domains)

certstream.listen_for_events(callback, url='ws://localhost:8080/')
```

### JavaScript

```javascript
const WebSocket = require('ws');
const ws = new WebSocket('ws://localhost:8080/');

ws.on('message', (data) => {
    const msg = JSON.parse(data);
    if (msg.message_type === 'certificate_update') {
        console.log(msg.data.leaf_cert.all_domains);
    }
});
```

### Go

```go
import "github.com/CaliDog/certstream-go"

stream, _ := certstream.CertStreamEventStream(false)
for event := range stream {
    fmt.Println(event.Data.LeafCert.AllDomains)
}
```

### With Authentication

```bash
# WebSocket with token
wscat -c ws://localhost:8080/ -H "Authorization: Bearer your-token"

# SSE with token
curl -H "Authorization: Bearer your-token" http://localhost:8080/sse
```

## Configuration

### YAML

```yaml
host: "0.0.0.0"
port: 8080
log_level: "info"
buffer_size: 1000
ct_logs_url: "https://www.gstatic.com/ct/log_list/v3/all_logs_list.json"

protocols:
  websocket: true
  sse: true
  tcp: true
  tcp_port: 8081

ct_log:
  state_file: "/data/state.json"
  batch_size: 256
  poll_interval_ms: 500
  retry_max_attempts: 3
  request_timeout_secs: 30

rate_limit:
  enabled: true
  per_second: 10
  burst_size: 20

connection_limit:
  enabled: true
  max_connections: 10000
  per_ip_limit: 100

auth:
  enabled: false
  tokens:
    - "secret-token-1"
    - "secret-token-2"
  header_name: "Authorization"

hot_reload:
  enabled: true

custom_logs:
  - name: "My Custom CT Log"
    url: "https://ct.example.com/log"
```

Config search order:
1. `CERTSTREAM_CONFIG` env var
2. `./config.yaml`
3. `./config.yml`
4. `/etc/certstream/config.yaml`

## Performance

Load tested with 1,000 concurrent WebSocket clients (same machine, same conditions):

| Metric | Rust | Go |
|--------|------|-----|
| Memory (idle) | ~12 MB | ~100 MB |
| Memory (avg under load) | 22 MB | 254 MB |
| CPU (avg under load) | ~15% | ~34% |
| Latency (avg) | 3.4ms | 31ms |
| Latency (min) | 0.16ms | 1.7ms |
| Throughput | 677K msg | 267K msg |

**Result**: 12x less memory, 9x faster latency, 2.5x higher throughput.

## CT Logs

Monitors 60+ logs including:

- **Google**: Argon, Xenon, Solera, Submariner
- **Cloudflare**: Nimbus
- **DigiCert**: Wyvern, Sphinx
- **Sectigo**: Elephant, Tiger, Dodo
- **Let's Encrypt**: Sapling, Clicky
- TrustAsia, Nordu, and others

## Release Notes

See [RELEASE_NOTES.md](RELEASE_NOTES.md) for version history.

## Related

- [certstream-server](https://github.com/CaliDog/certstream-server) - Elixir
- [certstream-server-go](https://github.com/d-Rickyy-b/certstream-server-go) - Go
- [certstream-python](https://github.com/CaliDog/certstream-python) - Python client

## License

MIT - see [LICENSE](LICENSE)
