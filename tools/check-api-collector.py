#!/usr/bin/env python3
"""Smoke the actual ARM64 collector against a local Lambda API, without AWS.

Run from the repository root:
  docker buildx build --platform linux/arm64 --target otel-extension-smoke \
    -f apps/backend/aws/api/Dockerfile .
The Docker target disables networking during this check and uses dummy credentials.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import urllib.request
import urllib.parse
from datetime import datetime, timezone

paths = []
errors = []
transport_mode = False
started = threading.Event()
telemetry_endpoint = None
invoked = False


class Runtime(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self, obj, headers=None):
        data = json.dumps(obj).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        paths.append(self.path)
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        if "error" in self.path:
            errors.append((self.path, body.decode(), dict(self.headers)))
        if self.path.endswith("/register"):
            self.respond(
                {"functionName": "collector-smoke", "functionVersion": "$LATEST",
                 "handler": "bootstrap", "accountId": "000000000000"},
                {"Lambda-Extension-Identifier": "local-smoke-extension"},
            )
        else:
            self.respond({})

    def do_PUT(self):
        global telemetry_endpoint
        paths.append(self.path)
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        address = urllib.parse.urlsplit(json.loads(body)["destination"]["URI"])
        telemetry_endpoint = f"http://127.0.0.1:{address.port}"
        self.respond({})

    def do_GET(self):
        global invoked
        paths.append(self.path)
        if transport_mode and not invoked:
            invoked = True
            self.respond({"eventType": "INVOKE", "requestId": "smoke-invocation",
                          "deadlineMs": int(time.time() * 1000) + 10000})
            started.set()
            return
        # The extension only requests its next event after successful startup.
        self.respond({"eventType": "SHUTDOWN", "shutdownReason": "SPINDOWN",
                      "deadlineMs": int(time.time() * 1000) + 2000})


server = http.server.ThreadingHTTPServer(("127.0.0.1", 9001), Runtime)
threading.Thread(target=server.serve_forever, daemon=True).start()
env = os.environ.copy()
env.update(
    AWS_LAMBDA_RUNTIME_API="127.0.0.1:9001",
    AWS_SAM_LOCAL="true",  # Local telemetry listener needs no Lambda DNS name.
    AWS_REGION="eu-west-1",
    AWS_LAMBDA_FUNCTION_NAME="collector-smoke",
    AWS_LAMBDA_FUNCTION_VERSION="$LATEST",
    AWS_LAMBDA_INITIALIZATION_TYPE="on-demand",
    AWS_EC2_METADATA_DISABLED="true",
    AWS_ACCESS_KEY_ID="smoke-only",
    AWS_SECRET_ACCESS_KEY="smoke-only",
    OPENTELEMETRY_COLLECTOR_CONFIG_URI="/otel-collector.yaml",
    OPENTELEMETRY_EXTENSION_LOG_LEVEL="warn",
)
result = subprocess.run(["/collector"], env=env, capture_output=True, text=True,
                        timeout=25)
print(result.stdout + result.stderr)
assert result.returncode == 0, result.returncode
assert not errors, errors
assert "/2020-01-01/extension/register" in paths, paths
assert "/2022-07-01/telemetry" in paths, paths
assert "/2020-01-01/extension/event/next" in paths, paths
assert "Failed to start" not in result.stdout + result.stderr
print("PASS: ARM64 ADOT collector parsed production config, registered, started "
      "and shut down against a local Lambda API with networking disabled.")

# Repeat with only the X-Ray endpoint redirected to loopback. The production
# receiver, processors, exporter, SigV4 signing and resource conversion stay intact.
segments = []
delivered = threading.Event()


class XRay(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        assert self.path == "/TraceSegments", self.path
        assert self.headers["Authorization"].startswith("AWS4-HMAC-SHA256 ")
        segments.extend(json.loads(segment) for segment in json.loads(body)["TraceSegmentDocuments"])
        data = b'{"UnprocessedTraceSegments":[]}'
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        delivered.set()


xray = http.server.ThreadingHTTPServer(("127.0.0.1", 2000), XRay)
threading.Thread(target=xray.serve_forever, daemon=True).start()
config = Path("/otel-collector.yaml").read_text()
assert config.count("  awsxray:\n") == 1
Path("/tmp/otel-collector-local.yaml").write_text(config.replace(
    "  awsxray:\n", "  awsxray:\n    endpoint: http://127.0.0.1:2000\n"))
env["OPENTELEMETRY_COLLECTOR_CONFIG_URI"] = "/tmp/otel-collector-local.yaml"
transport_mode = True
process = subprocess.Popen(["/collector"], env=env, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True)
try:
    assert started.wait(10), "collector did not start"
    client = subprocess.run(["/otel-client"], capture_output=True, text=True, timeout=5)
    assert client.returncode == 0, client.stderr
    assert delivered.wait(5), "collector did not export to the local X-Ray endpoint"
    assert len(segments) == 1, segments
    segment = segments[0]
    assert segment["trace_id"] == client.stdout.strip(), segment
    assert segment["id"] == "8899aabbccddeeff", segment
    assert segment["parent_id"] == "0011223344556677", segment
    assert segment["name"] == "GET /api/v1/smoke", segment
    assert segment["end_time"] > segment["start_time"], segment
finally:
    # Complete the invocation through the actual Lambda Telemetry API listener,
    # allowing decouple to drain and the extension to request its next event.
    event = [{"time": datetime.now(timezone.utc).isoformat(), "type": "platform.runtimeDone",
              "record": {"requestId": "smoke-invocation", "status": "success",
                         "metrics": {"durationMs": 1, "producedBytes": 1}}}]
    request = urllib.request.Request(telemetry_endpoint, data=json.dumps(event).encode(),
                                     headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=3) as response:
        assert response.status == 200
    output, _ = process.communicate(timeout=25)
    print(output)
assert process.returncode == 0, process.returncode
assert not errors, errors
print("PASS: OTLP/gRPC span exported as signed X-Ray PutTraceSegments with trace "
      "ID, span ID, parent ID and timing preserved; endpoint was loopback only.")
