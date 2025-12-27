#!/bin/bash

echo "=== certstream-server-rust Docker Examples ==="
echo ""

basic_run() {
    echo "1. Basic Run (WebSocket only)"
    echo "   docker run -d -p 8080:8080 certstream-server-rust"
    echo ""
}

all_protocols() {
    echo "2. All Protocols (WebSocket + SSE + TCP)"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -p 8081:8081 \\"
    echo "     -e CERTSTREAM_SSE_ENABLED=true \\"
    echo "     -e CERTSTREAM_TCP_ENABLED=true \\"
    echo "     -e CERTSTREAM_TCP_PORT=8081 \\"
    echo "     certstream-server-rust"
    echo ""
}

persistence() {
    echo "3. With State Persistence (resume after restart)"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -v certstream-state:/data \\"
    echo "     -e CERTSTREAM_STATE_FILE=/data/state.json \\"
    echo "     certstream-server-rust"
    echo ""
}

rate_limit() {
    echo "4. With Rate Limiting"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -e CERTSTREAM_RATE_LIMIT_ENABLED=true \\"
    echo "     -e CERTSTREAM_RATE_LIMIT_PER_SECOND=10 \\"
    echo "     -e CERTSTREAM_RATE_LIMIT_BURST_SIZE=20 \\"
    echo "     certstream-server-rust"
    echo ""
}

auth() {
    echo "5. With Token Authentication"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -e CERTSTREAM_AUTH_ENABLED=true \\"
    echo "     -e CERTSTREAM_AUTH_TOKENS=secret-token-1,secret-token-2 \\"
    echo "     certstream-server-rust"
    echo ""
    echo "   Client usage:"
    echo "   wscat -c ws://localhost:8080/ -H 'Authorization: Bearer secret-token-1'"
    echo ""
}

connection_limit() {
    echo "6. With Connection Limits"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -e CERTSTREAM_CONNECTION_LIMIT_ENABLED=true \\"
    echo "     -e CERTSTREAM_MAX_CONNECTIONS=1000 \\"
    echo "     -e CERTSTREAM_PER_IP_LIMIT=10 \\"
    echo "     certstream-server-rust"
    echo ""
}

config_file() {
    echo "7. With Config File"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -v ./config.yaml:/app/config.yaml \\"
    echo "     -e CERTSTREAM_CONFIG=/app/config.yaml \\"
    echo "     certstream-server-rust"
    echo ""
}

hot_reload() {
    echo "8. With Hot Reload (config changes without restart)"
    echo "   docker run -d \\"
    echo "     -p 8080:8080 \\"
    echo "     -v ./config.yaml:/app/config.yaml \\"
    echo "     -e CERTSTREAM_CONFIG=/app/config.yaml \\"
    echo "     -e CERTSTREAM_HOT_RELOAD_ENABLED=true \\"
    echo "     certstream-server-rust"
    echo ""
    echo "   Edit config.yaml and changes apply automatically!"
    echo ""
}

tls() {
    echo "9. With TLS"
    echo "   docker run -d \\"
    echo "     -p 443:8080 \\"
    echo "     -v ./cert.pem:/app/cert.pem \\"
    echo "     -v ./key.pem:/app/key.pem \\"
    echo "     -e CERTSTREAM_TLS_CERT=/app/cert.pem \\"
    echo "     -e CERTSTREAM_TLS_KEY=/app/key.pem \\"
    echo "     certstream-server-rust"
    echo ""
}

production() {
    echo "10. Production Setup (all features)"
    echo "    docker run -d \\"
    echo "      --name certstream \\"
    echo "      --restart unless-stopped \\"
    echo "      -p 8080:8080 \\"
    echo "      -p 8081:8081 \\"
    echo "      -v certstream-state:/data \\"
    echo "      -v ./config.yaml:/app/config.yaml \\"
    echo "      -e CERTSTREAM_CONFIG=/app/config.yaml \\"
    echo "      -e CERTSTREAM_STATE_FILE=/data/state.json \\"
    echo "      -e CERTSTREAM_SSE_ENABLED=true \\"
    echo "      -e CERTSTREAM_TCP_ENABLED=true \\"
    echo "      -e CERTSTREAM_HOT_RELOAD_ENABLED=true \\"
    echo "      -e CERTSTREAM_RATE_LIMIT_ENABLED=true \\"
    echo "      -e CERTSTREAM_CONNECTION_LIMIT_ENABLED=true \\"
    echo "      -e RUST_LOG=info \\"
    echo "      certstream-server-rust"
    echo ""
}

compose() {
    echo "11. Docker Compose"
    echo "    See docker-compose.yml in repository root"
    echo ""
}

basic_run
all_protocols
persistence
rate_limit
auth
connection_limit
config_file
hot_reload
tls
production
compose

echo "=== Testing Commands ==="
echo ""
echo "Health check:     curl http://localhost:8080/health"
echo "Metrics:          curl http://localhost:8080/metrics"
echo "Example JSON:     curl http://localhost:8080/example.json"
echo "WebSocket:        wscat -c ws://localhost:8080/"
echo "SSE:              curl -N http://localhost:8080/sse"
echo "TCP:              nc localhost 8081"
echo ""
