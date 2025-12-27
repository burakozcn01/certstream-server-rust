# Release Notes - v1.0.5

**Release Date**: December 28, 2025

## Changes

### Complete Certificate Field Compatibility with certstream-server-go

This release adds full compatibility with certstream-server-go certificate output format.

### New Certificate Fields

**LeafCert/ChainCert:**
- `sha1` - SHA1 fingerprint (separate field, colon-separated hex)
- `sha256` - SHA256 fingerprint (separate field, colon-separated hex)
- `signature_algorithm` - Algorithm used to sign the certificate (e.g., "sha256, rsa", "ecdsa, sha256")
- `is_ca` - Boolean indicating if certificate is a CA

**Subject/Issuer (now a struct instead of HashMap):**
- `C` - Country
- `CN` - Common Name
- `L` - Locality
- `O` - Organization
- `OU` - Organizational Unit
- `ST` - State/Province
- `aggregated` - Combined string format (e.g., "/C=US/CN=example.com/O=Example Inc")
- `email_address` - Email address from certificate

**Extensions (fully parsed):**
- `authorityInfoAccess` - CA info and OCSP URLs
- `authorityKeyIdentifier` - Authority key ID
- `basicConstraints` - "CA:TRUE" or "CA:FALSE"
- `certificatePolicies` - Policy OIDs
- `extendedKeyUsage` - "serverAuth", "clientAuth", etc.
- `keyUsage` - "Digital Signature", "Key Encipherment", etc.
- `subjectAltName` - "DNS:example.com, IP Address:1.2.3.4, email:x@y.com"
- `subjectKeyIdentifier` - Subject key ID
- `ctlPoisonByte` - Boolean for precert detection

**Source:**
- `operator` - CT log operator name (Google, Cloudflare, DigiCert, etc.)

### Example Output

```json
{
  "message_type": "certificate_update",
  "data": {
    "leaf_cert": {
      "subject": {
        "CN": "example.com",
        "O": "Example Inc",
        "C": "US",
        "aggregated": "/C=US/CN=example.com/O=Example Inc"
      },
      "issuer": {
        "CN": "R3",
        "O": "Let's Encrypt",
        "aggregated": "/CN=R3/O=Let's Encrypt"
      },
      "serial_number": "0123456789ABCDEF",
      "not_before": 1704067200,
      "not_after": 1735689600,
      "fingerprint": "AB:CD:EF:01:23:45:...",
      "sha1": "AB:CD:EF:01:23:45:...",
      "sha256": "AB:CD:EF:01:23:45:...",
      "signature_algorithm": "sha256, rsa",
      "is_ca": false,
      "all_domains": ["example.com", "www.example.com"],
      "extensions": {
        "keyUsage": "Digital Signature, Key Encipherment",
        "extendedKeyUsage": "serverAuth, clientAuth",
        "basicConstraints": "CA:FALSE",
        "subjectAltName": "DNS:example.com, DNS:www.example.com"
      }
    },
    "source": {
      "name": "Google 'Argon2025h2' log",
      "url": "https://ct.googleapis.com/logs/us1/argon2025h2"
    }
  }
}
```

---

## Dependencies

- Added `sha1` crate for SHA1 fingerprint calculation

---

## Migration Guide

No breaking changes. Existing clients will continue to work. New fields are additive.

```bash
docker pull reloading01/certstream-server-rust:1.0.5
```

---

# Previous Releases

## v1.0.4 (December 27, 2025)

### Connection Limiting Fix
- **Critical Fix**: Connection limiting now works correctly for WebSocket, SSE, and TCP connections
- Previous behavior: Connection limits were immediately released after HTTP upgrade, making limits ineffective
- New behavior: Connection limits are properly tracked throughout the entire connection lifecycle

### Rate Limiting Removed
- Rate limiting has been removed as it's not useful for streaming protocols
- Connection limiting is the appropriate mechanism for WebSocket/SSE/TCP servers

---

## v1.0.3 (December 26, 2025)

- Production-ready release
- Initial stable version
