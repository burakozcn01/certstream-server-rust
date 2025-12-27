#!/usr/bin/env python3
import json
import socket
import sys

def connect_tcp(host, port):
    print(f"Connecting to TCP stream: {host}:{port}")

    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.connect((host, port))
    print("Connected! Streaming certificates...\n")

    buffer = ""
    try:
        while True:
            data = sock.recv(4096).decode("utf-8")
            if not data:
                break

            buffer += data
            while "\n" in buffer:
                line, buffer = buffer.split("\n", 1)
                if not line.strip():
                    continue

                try:
                    msg = json.loads(line)
                    if msg.get("message_type") == "certificate_update":
                        cert = msg["data"]["leaf_cert"]
                        domains = cert.get("all_domains", [])
                        issuer = cert.get("issuer", {}).get("CN", "Unknown")
                        print(f"[CERT] {', '.join(domains[:3])} | Issuer: {issuer}")
                    elif msg.get("message_type") == "heartbeat":
                        print("[HEARTBEAT]")
                except json.JSONDecodeError:
                    pass
    finally:
        sock.close()

if __name__ == "__main__":
    host = sys.argv[1] if len(sys.argv) > 1 else "localhost"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8081
    connect_tcp(host, port)
