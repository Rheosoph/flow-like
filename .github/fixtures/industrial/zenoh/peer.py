"""Reference Python peer reached through a separate official Zenoh router."""

import collections
import json
import os
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import zenoh


def sample_json(sample):
    return {
        "key": str(sample.key_expr),
        "payload": list(sample.payload.to_bytes()),
        "encoding": str(sample.encoding),
        "deleted": sample.kind == zenoh.SampleKind.DELETE,
        "timestamp": str(sample.timestamp) if sample.timestamp else None,
    }


class Peer:
    def __init__(self):
        config = zenoh.Config.from_json5(json.dumps({
            "mode": "client",
            "connect": {"endpoints": [os.environ["ZENOH_ENDPOINT"]]},
            "scouting": {"multicast": {"enabled": False}},
        }))
        self.session = zenoh.open(config)
        self.samples = collections.deque(maxlen=512)
        self.lock = threading.Lock()
        self.subscriber = self.session.declare_subscriber(
            "flow-like/e2e/adapter/**", self.observe
        )
        self.queryable = self.session.declare_queryable(
            "flow-like/e2e/oracle/**", self.answer, complete=True
        )

    def observe(self, sample):
        with self.lock:
            self.samples.append(sample_json(sample))

    def answer(self, query):
        with query:
            key = str(query.key_expr)
            if key == "flow-like/e2e/oracle/error":
                query.reply_err("fixture denied query", encoding="text/plain")
            elif key == "flow-like/e2e/oracle/slow":
                time.sleep(2)
                query.reply(key, "late", encoding="text/plain")
            elif key.startswith("flow-like/e2e/oracle/known/"):
                query.reply(
                    "flow-like/e2e/oracle/known/a", bytes([0, 255, 128]),
                    encoding="application/octet-stream", timestamp=self.session.new_timestamp(),
                )
                query.reply(
                    "flow-like/e2e/oracle/known/b", '{"temperature":21.5}',
                    encoding="application/json", timestamp=self.session.new_timestamp(),
                )
            elif key.startswith("flow-like/e2e/oracle/many/"):
                for index in range(4):
                    query.reply(f"flow-like/e2e/oracle/many/{index}", bytes([index]))

    def publish(self, body, deleted=False):
        key = body["key"]
        if not key.startswith("flow-like/e2e/peer/"):
            raise ValueError("Only the peer test namespace can be published")
        with self.session.declare_publisher(key) as publisher:
            deadline = time.monotonic() + 5
            while not publisher.matching_status:
                if time.monotonic() >= deadline:
                    raise TimeoutError("The Rust subscription did not reach the router")
                time.sleep(0.01)
            if deleted:
                publisher.delete(timestamp=self.session.new_timestamp())
            else:
                publisher.put(
                    bytes(body["payload"]), encoding=body["encoding"],
                    timestamp=self.session.new_timestamp(),
                )


class Handler(BaseHTTPRequestHandler):
    def respond(self, value, status=200):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        args = urllib.parse.parse_qs(parsed.query)
        try:
            if parsed.path == "/health":
                self.respond({"ready": True, "implementation": "eclipse-zenoh-python-1.6.2"})
            elif parsed.path == "/samples":
                key = args["key"][0]
                with self.server.peer.lock:
                    samples = [s for s in self.server.peer.samples if s["key"] == key]
                self.respond(samples)
            elif parsed.path == "/query":
                values = []
                for reply in self.server.peer.session.get(args["selector"][0], timeout=2):
                    if reply.err is not None:
                        raise ValueError(reply.err.payload.to_string())
                    values.append(sample_json(reply.ok))
                self.respond(values)
            else:
                self.respond({"error": "unknown endpoint"}, 404)
        except Exception as error:
            self.respond({"error": str(error)}, 500)

    def do_POST(self):
        try:
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 < size <= 1024 * 1024:
                raise ValueError("Expected a bounded JSON body")
            body = json.loads(self.rfile.read(size))
            if self.path not in ("/publish", "/delete"):
                self.respond({"error": "unknown endpoint"}, 404)
                return
            self.server.peer.publish(body, deleted=self.path == "/delete")
            self.respond({"published": True})
        except Exception as error:
            self.respond({"error": str(error)}, 500)


if __name__ == "__main__":
    if sys.argv[1:] == ["--healthcheck"]:
        with urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=3) as response:
            assert json.load(response)["ready"]
    else:
        peer = Peer()
        server = ThreadingHTTPServer(("0.0.0.0", 8080), Handler)
        server.peer = peer
        try:
            server.serve_forever()
        finally:
            peer.session.close()
