#!/usr/bin/env python3
import json
import sys
import time
import threading
from collections import deque

try:
    import websocket
except ImportError:
    print("Installing websocket-client...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "websocket-client"])
    import websocket

class CertBenchmark:
    def __init__(self):
        self.message_count = 0
        self.start_time = None
        self.latencies = deque(maxlen=1000)
        self.lock = threading.Lock()
        self.running = True

    def on_message(self, ws, message):
        now = time.time()
        if self.start_time is None:
            self.start_time = now

        with self.lock:
            self.message_count += 1

            try:
                data = json.loads(message)
                if data.get("message_type") == "certificate_update":
                    seen = data["data"].get("seen", 0)
                    if seen > 0:
                        latency = (now - seen) * 1000
                        self.latencies.append(latency)
            except:
                pass

    def print_stats(self):
        while self.running:
            time.sleep(5)
            with self.lock:
                if self.start_time is None:
                    continue

                elapsed = time.time() - self.start_time
                rate = self.message_count / elapsed if elapsed > 0 else 0

                if self.latencies:
                    avg_latency = sum(self.latencies) / len(self.latencies)
                    min_latency = min(self.latencies)
                    max_latency = max(self.latencies)
                else:
                    avg_latency = min_latency = max_latency = 0

                print(f"\n--- Stats (elapsed: {elapsed:.1f}s) ---")
                print(f"Messages: {self.message_count}")
                print(f"Rate: {rate:.1f} msg/s")
                print(f"Latency (ms): avg={avg_latency:.1f}, min={min_latency:.1f}, max={max_latency:.1f}")

def run_benchmark(url):
    bench = CertBenchmark()

    stats_thread = threading.Thread(target=bench.print_stats, daemon=True)
    stats_thread.start()

    print(f"Connecting to {url}")
    print("Stats will print every 5 seconds...\n")

    def on_open(ws):
        print("Connected! Benchmarking...\n")

    def on_error(ws, error):
        print(f"Error: {error}")

    def on_close(ws, code, msg):
        bench.running = False
        print("\nConnection closed")

    ws = websocket.WebSocketApp(
        url,
        on_open=on_open,
        on_message=bench.on_message,
        on_error=on_error,
        on_close=on_close,
    )

    try:
        ws.run_forever()
    except KeyboardInterrupt:
        bench.running = False
        print("\nBenchmark stopped")

if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "ws://localhost:8080/"
    run_benchmark(url)
