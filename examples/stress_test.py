#!/usr/bin/env python3
import sys
import time
import threading
import signal
from collections import defaultdict

try:
    import websocket
except ImportError:
    print("Installing websocket-client...")
    import subprocess
    subprocess.check_call([sys.executable, "-m", "pip", "install", "websocket-client"])
    import websocket

class StressTest:
    def __init__(self, url, num_clients):
        self.url = url
        self.num_clients = num_clients
        self.connected = 0
        self.disconnected = 0
        self.errors = 0
        self.messages = 0
        self.lock = threading.Lock()
        self.running = True
        self.clients = []

    def client_thread(self, client_id):
        def on_open(ws):
            with self.lock:
                self.connected += 1

        def on_message(ws, message):
            with self.lock:
                self.messages += 1

        def on_error(ws, error):
            with self.lock:
                self.errors += 1

        def on_close(ws, code, msg):
            with self.lock:
                self.disconnected += 1

        while self.running:
            try:
                ws = websocket.WebSocketApp(
                    self.url,
                    on_open=on_open,
                    on_message=on_message,
                    on_error=on_error,
                    on_close=on_close
                )
                self.clients.append(ws)
                ws.run_forever()
            except:
                pass
            if self.running:
                time.sleep(1)

    def print_stats(self):
        start_time = time.time()
        last_messages = 0
        while self.running:
            time.sleep(5)
            with self.lock:
                elapsed = time.time() - start_time
                msg_rate = (self.messages - last_messages) / 5
                last_messages = self.messages
                print(f"\r[{elapsed:.0f}s] Connected: {self.connected} | Disconnected: {self.disconnected} | Errors: {self.errors} | Messages: {self.messages} | Rate: {msg_rate:.0f}/s", flush=True)

    def run(self):
        print(f"Starting stress test with {self.num_clients} clients")
        print(f"URL: {self.url}")
        print("")

        stats_thread = threading.Thread(target=self.print_stats, daemon=True)
        stats_thread.start()

        threads = []
        for i in range(self.num_clients):
            t = threading.Thread(target=self.client_thread, args=(i,), daemon=True)
            t.start()
            threads.append(t)
            if (i + 1) % 50 == 0:
                print(f"Spawned {i + 1}/{self.num_clients} clients...")
            time.sleep(0.02)

        print(f"\nAll {self.num_clients} clients spawned. Press Ctrl+C to stop.\n")

        try:
            while self.running:
                time.sleep(1)
        except KeyboardInterrupt:
            print("\n\nStopping...")
            self.running = False
            for ws in self.clients:
                try:
                    ws.close()
                except:
                    pass

        print(f"\n=== Final Stats ===")
        print(f"Total Connected: {self.connected}")
        print(f"Total Disconnected: {self.disconnected}")
        print(f"Total Errors: {self.errors}")
        print(f"Total Messages: {self.messages}")

if __name__ == "__main__":
    url = sys.argv[1] if len(sys.argv) > 1 else "ws://localhost:8080/"
    num_clients = int(sys.argv[2]) if len(sys.argv) > 2 else 500

    test = StressTest(url, num_clients)
    test.run()
