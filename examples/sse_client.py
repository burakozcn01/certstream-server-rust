#!/usr/bin/env python3
import json
import sys

try:
    import requests
except ImportError:
    print("Installing requests...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "requests"])
    import requests

def stream_sse(url):
    print(f"Connecting to SSE stream: {url}")

    with requests.get(url, stream=True, headers={"Accept": "text/event-stream"}) as response:
        response.raise_for_status()
        print("Connected! Streaming certificates...\n")

        for line in response.iter_lines():
            if not line:
                continue

            line = line.decode("utf-8")
            if not line.startswith("data:"):
                continue

            try:
                data = json.loads(line[5:].strip())
                if data.get("message_type") == "certificate_update":
                    cert = data["data"]["leaf_cert"]
                    domains = cert.get("all_domains", [])
                    issuer = cert.get("issuer", {}).get("CN", "Unknown")
                    print(f"[CERT] {', '.join(domains[:3])} | Issuer: {issuer}")
                elif data.get("message_type") == "heartbeat":
                    print("[HEARTBEAT]")
            except json.JSONDecodeError:
                pass

if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8080/sse"
    stream_sse(url)
