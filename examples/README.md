# certstream-server-rust Examples

Demo scripts for connecting to certstream-server-rust.

## Scripts

### websocket_client.py
Basic WebSocket client for streaming certificates.

```bash
# Default (localhost:8080)
python3 websocket_client.py

# Custom URL
python3 websocket_client.py ws://certstream.example.com/
```

### sse_client.py
Server-Sent Events (SSE) client.

```bash
# Default (localhost:8080/sse)
python3 sse_client.py

# Custom URL
python3 sse_client.py http://localhost:8080/sse
```

### tcp_client.py
Raw TCP client for newline-delimited JSON stream.

```bash
# Default (localhost:8081)
python3 tcp_client.py

# Custom host/port
python3 tcp_client.py localhost 8081
```

### benchmark.py
Performance benchmark tool with statistics.

```bash
# Run benchmark
python3 benchmark.py ws://localhost:8080/

# Stats printed every 5 seconds:
# - Message count
# - Messages per second
# - Latency (avg/min/max)
```

### docker_examples.sh
Docker run examples for various configurations.

```bash
# View all examples
bash docker_examples.sh
```

## Endpoints

| Endpoint | Protocol | Description |
|----------|----------|-------------|
| `/` | WebSocket | Lite stream (no certificate chain) |
| `/full-stream` | WebSocket | Full stream with certificate chain |
| `/domains-only` | WebSocket | Only domain names, minimal data |
| `/sse` | SSE | Server-Sent Events stream |
| `:8081` | TCP | Raw newline-delimited JSON |

## Requirements

```bash
pip install websocket-client requests
```
