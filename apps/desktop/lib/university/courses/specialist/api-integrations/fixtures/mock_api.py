from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
import json
import threading

records = {}
dropped = set()
lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, status, value, headers=None):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        for key, val in (headers or {}).items():
            self.send_header(key, val)
        self.end_headers()
        self.wfile.write(body)

    def authorized(self):
        if self.headers.get("Authorization") != "Bearer practice-token":
            self.reply(401, {"error": "practice token required"})
            return False
        return True

    def do_GET(self):
        if not self.authorized():
            return
        url = urlparse(self.path)
        params = parse_qs(url.query)
        if url.path == "/customers":
            if "email" in params:
                if params["email"][0] != "learner@example.invalid":
                    return self.reply(404, {"error": "customer not found"})
                return self.reply(200, {"id": "C-001", "plan": "practice"})
            page = params.get("page", ["1"])[0]
            if page == "1":
                return self.reply(200, {"items": [{"id": "C-001"}], "next_page": 2})
            return self.reply(200, {"items": [{"id": "C-002"}], "next_page": None})
        if url.path.startswith("/status/"):
            try:
                code = int(url.path.rsplit("/", 1)[1])
            except ValueError:
                return self.reply(400, {"error": "invalid status"})
            if code not in (401, 404, 429, 503):
                return self.reply(400, {"error": "unsupported practice status"})
            return self.reply(
                code, {"status": code}, {"Retry-After": "1"} if code == 429 else {}
            )
        if url.path == "/notes":
            with lock:
                values = list(records.values())
            return self.reply(200, {"count": len(values), "notes": values})
        return self.reply(404, {"error": "unknown path"})

    def do_POST(self):
        if not self.authorized():
            return
        url = urlparse(self.path)
        if url.path != "/notes":
            return self.reply(404, {"error": "unknown path"})
        key = self.headers.get("Idempotency-Key", "")
        if not key:
            return self.reply(400, {"error": "Idempotency-Key required"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length > 10000:
                return self.reply(413, {"error": "practice body too large"})
            value = json.loads(self.rfile.read(length))
        except (ValueError, json.JSONDecodeError):
            return self.reply(400, {"error": "invalid JSON"})
        with lock:
            if key in records and records[key] != value:
                conflict = True
            else:
                conflict = False
                records[key] = value
            drop = (
                not conflict
                and parse_qs(url.query).get("drop_once") == ["1"]
                and key not in dropped
            )
            if drop:
                dropped.add(key)
        if conflict:
            return self.reply(409, {"error": "key reused with different body"})
        if drop:
            self.close_connection = True
            return
        self.reply(200, {"operation_id": key, "stored": True})


if __name__ == "__main__":
    print("Practice API on http://127.0.0.1:8765; stop with Ctrl-C", flush=True)
    ThreadingHTTPServer(("127.0.0.1", 8765), Handler).serve_forever()
