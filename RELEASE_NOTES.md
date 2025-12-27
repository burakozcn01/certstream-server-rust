# Release Notes - v1.0.4

**Release Date**: December 27, 2025

## Changes

### Connection Limiting Fix
- **Critical Fix**: Connection limiting now works correctly for WebSocket, SSE, and TCP connections
- Previous behavior: Connection limits were immediately released after HTTP upgrade, making limits ineffective
- New behavior: Connection limits are properly tracked throughout the entire connection lifecycle

### Rate Limiting Removed
- Rate limiting has been removed as it's not useful for streaming protocols
- Connection limiting is the appropriate mechanism for WebSocket/SSE/TCP servers

### What Changed
- Connection limiting moved from HTTP middleware to protocol handlers
- WebSocket: Limit acquired on upgrade, released when socket closes
- SSE: Limit acquired on stream start, released when stream ends (via Drop trait)
- TCP: Limit acquired on accept, released when connection closes

### Configuration
```yaml
connection_limit:
  enabled: true
  max_connections: 10000
  per_ip_limit: 20
```

---

## Breaking Changes

- `rate_limit` configuration section removed (was not functional for streaming)
- `tower_governor` dependency removed

---

## Migration Guide

Remove any `rate_limit` configuration from your config.yaml. Connection limiting handles abuse prevention.

```bash
docker pull reloading01/certstream-server-rust:1.0.4
```

---
