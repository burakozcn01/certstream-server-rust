#!/usr/bin/env python3
import json
import sys

try:
    import websocket
except ImportError:
    print("Installing websocket-client...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "websocket-client"])
    import websocket

def on_message(ws, message):
    data = json.loads(message)
    if data.get("message_type") == "certificate_update":
        cert = data["data"]["leaf_cert"]
        domains = cert.get("all_domains", [])
        issuer = cert.get("issuer", {}).get("CN", "Unknown")
        print(f"[CERT] {', '.join(domains[:3])} | Issuer: {issuer}")
    elif data.get("message_type") == "heartbeat":
        print("[HEARTBEAT]")

def on_error(ws, error):
    print(f"Error: {error}")

def on_close(ws, close_status_code, close_msg):
    print("Connection closed")

def on_open(ws):
    print("Connected to certstream server")

if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "ws://localhost:8080/"
    print(f"Connecting to {url}")

    ws = websocket.WebSocketApp(
        url,
        on_open=on_open,
        on_message=on_message,
        on_error=on_error,
        on_close=on_close
    )
    ws.run_forever()
