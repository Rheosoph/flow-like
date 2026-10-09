"""Tahu protobuf/lifecycle peer and a bounded Sparkplug host oracle over MQTT."""

import collections
import copy
import json
import os
import socket
import sys
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from google.protobuf.json_format import MessageToDict
import paho.mqtt.client as mqtt
import sparkplug_b as tahu


def now():
    return int(time.time() * 1000)


def mqtt_client(client_id):
    return mqtt.Client(
        mqtt.CallbackAPIVersion.VERSION2, client_id=client_id,
        clean_session=True, reconnect_on_failure=False,
    )


def connect(client):
    ready = threading.Event()
    result = []

    def connected(_client, _userdata, _flags, reason, _properties):
        result.append(reason.is_failure)
        ready.set()

    client.on_connect = connected
    client.connect(os.environ["SPARKPLUG_MQTT_HOST"], int(os.environ.get("SPARKPLUG_MQTT_PORT", "1883")), 5)
    client.loop_start()
    if not ready.wait(5) or result != [False]:
        raise RuntimeError("MQTT fixture connection failed")


class Oracle:
    def __init__(self):
        self.lock = threading.Lock()
        self.nodes = {}
        self.references = {}
        self.reference_lock = threading.Lock()
        self.client = mqtt_client("flow-like-tahu-oracle")
        self.client.on_message = self.observe
        ready = threading.Event()
        self.client.on_subscribe = lambda *_args: ready.set()
        connect(self.client)
        self.client.subscribe("spBv1.0/#", qos=1)
        if not ready.wait(5):
            raise RuntimeError("Tahu observer subscription was not acknowledged")

    def state(self, group, node):
        return self.nodes.setdefault((group, node), {
            "online": False, "bd_seq": None, "next_seq": None,
            "births": 0, "deaths": 0, "stale_deaths": 0,
            "metrics": {}, "devices": {}, "aliases": {},
            "events": collections.deque(maxlen=1024), "errors": [],
        })

    def snapshot(self, group, node):
        with self.lock:
            state = copy.deepcopy(self.state(group, node))
            state["events"] = list(state["events"])
            return state

    def observe(self, _client, _userdata, message):
        parts = message.topic.split("/")
        if len(parts) not in (4, 5):
            return
        group, kind, node = parts[1:4]
        if not group.startswith("flow-like-e2e-"):
            return
        with self.lock:
            state = self.state(group, node)
            try:
                payload = tahu.Payload()
                payload.ParseFromString(message.payload)
                document = MessageToDict(payload)
                event = {"kind": kind, "payload": document, "qos": message.qos, "retain": message.retain}
                state["events"].append(event)
                if kind in ("NCMD", "DCMD"):
                    return
                assert not message.retain, "lifecycle/data messages must not be retained"
                if kind == "NDEATH":
                    assert message.qos == 1, "NDEATH must use QoS 1"
                    assert not payload.HasField("seq"), "NDEATH must omit the sequence field"
                    bd_seq = self.bd_seq(payload)
                    state["deaths"] += 1
                    if state["bd_seq"] == bd_seq:
                        state["online"] = False
                        state["next_seq"] = None
                    else:
                        state["stale_deaths"] += 1
                    return
                assert message.qos == 0, "birth/data/device-death messages must use QoS 0"
                assert payload.HasField("timestamp") and payload.timestamp > 0
                assert payload.HasField("seq") and 0 <= payload.seq <= 255
                if kind == "NBIRTH":
                    assert payload.seq == 0, "NBIRTH must reset sequence to zero"
                    state["bd_seq"] = self.bd_seq(payload)
                    state["online"] = True
                    state["births"] += 1
                    state["metrics"] = {}
                    state["devices"] = {}
                    state["aliases"] = {}
                    self.birth_metrics(state, state["metrics"], payload, "")
                else:
                    assert state["online"], "node data/device lifecycle arrived before NBIRTH"
                    assert payload.seq == state["next_seq"], f"sequence gap: expected {state['next_seq']}, got {payload.seq}"
                    device = parts[4] if len(parts) == 5 else None
                    if kind == "DBIRTH":
                        state["devices"][device] = {"online": True, "metrics": {}}
                        self.birth_metrics(state, state["devices"][device]["metrics"], payload, device)
                    elif kind in ("NDATA", "DDATA"):
                        target = state["metrics"] if device is None else state["devices"][device]["metrics"]
                        for metric in payload.metrics:
                            if metric.HasField("alias"):
                                assert state["aliases"][str(metric.alias)][0] == (device or ""), "alias belongs to a different device"
                            name = metric.name if metric.HasField("name") else state["aliases"][str(metric.alias)][1]
                            previous = target[name]
                            if metric.HasField("datatype"):
                                assert metric.datatype == previous["datatype"], "metric datatype changed without rebirth"
                            update = MessageToDict(metric)
                            if not metric.is_historical:
                                previous.update(update)
                    elif kind == "DDEATH":
                        assert state["devices"][device]["online"]
                        state["devices"][device]["online"] = False
                    else:
                        raise AssertionError(f"unexpected message kind {kind}")
                state["next_seq"] = (payload.seq + 1) % 256
            except Exception as error:
                state["errors"].append(f"{kind}: {type(error).__name__}: {error}")

    @staticmethod
    def bd_seq(payload):
        metrics = [metric for metric in payload.metrics if metric.name == "bdSeq"]
        assert len(metrics) == 1
        metric = metrics[0]
        assert metric.datatype == tahu.MetricDataType.Int64
        assert metric.WhichOneof("value") == "long_value" and metric.long_value <= 255
        return metric.long_value

    @staticmethod
    def birth_metrics(state, target, payload, device):
        for metric in payload.metrics:
            assert metric.HasField("name") and metric.name and metric.name not in target
            assert metric.HasField("datatype") and metric.datatype > 0
            assert metric.HasField("timestamp"), "birth metrics require timestamps"
            assert metric.is_null or metric.WhichOneof("value") is not None
            expected = {
                tahu.MetricDataType.Double: "double_value",
                tahu.MetricDataType.Boolean: "boolean_value",
                tahu.MetricDataType.String: "string_value",
                tahu.MetricDataType.Int64: "long_value",
                tahu.MetricDataType.UInt64: "long_value",
                tahu.MetricDataType.DataSet: "dataset_value",
                tahu.MetricDataType.Bytes: "bytes_value",
                tahu.MetricDataType.File: "bytes_value",
            }
            if not metric.is_null and metric.datatype in expected:
                assert metric.WhichOneof("value") == expected[metric.datatype], "metric value field disagrees with the Tahu datatype"
            if metric.HasField("alias"):
                alias = str(metric.alias)
                assert alias not in state["aliases"], "birth aliases must be unique within a node"
                state["aliases"][alias] = [device, metric.name]
            target[metric.name] = MessageToDict(metric)

    def publish(self, client, group, kind, node, payload, device=None):
        topic = f"spBv1.0/{group}/{kind}/{node}" + (f"/{device}" if device else "")
        sent = client.publish(topic, payload.SerializeToString(), qos=1 if kind == "NDEATH" else 0, retain=False)
        sent.wait_for_publish(timeout=5)
        if not sent.is_published():
            raise TimeoutError("Tahu publication was not sent")

    def rebirth_command(self, group, node):
        payload = tahu.Payload(timestamp=now())
        tahu.addMetric(payload, "Node Control/Rebirth", None, tahu.MetricDataType.Boolean, True, now())
        self.publish(self.client, group, "NCMD", node, payload)

    def reference(self, action, group, node):
        # Tahu's helper owns global sequence counters. Serialize its reference-node actions.
        with self.reference_lock:
            key = (group, node)
            if action == "start":
                if self.references:
                    raise ValueError("Only one Tahu reference edge may run at a time")
                tahu.bdSeq = 41
                death = tahu.getNodeDeathPayload()
                client = mqtt_client(f"tahu-reference-{node}")
                client.will_set(f"spBv1.0/{group}/NDEATH/{node}", death.SerializeToString(), qos=1, retain=False)
                connect(client)
                self.references[key] = (client, death)
                self.reference_birth(client, group, node, 21.5, True)
                data = tahu.getDdataPayload()
                tahu.addMetric(data, None, 10, tahu.MetricDataType.Double, 22.75, now())
                self.publish(client, group, "NDATA", node, data)
                data = tahu.getDdataPayload()
                tahu.addMetric(data, None, 20, tahu.MetricDataType.Boolean, False, now())
                self.publish(client, group, "DDATA", node, data, "device")
            else:
                client, death = self.references[key]
                if action == "gap":
                    tahu.getSeqNum()
                    data = tahu.getDdataPayload()
                    tahu.addMetric(data, None, 10, tahu.MetricDataType.Double, 23.5, now())
                    self.publish(client, group, "NDATA", node, data)
                elif action == "rebirth":
                    self.reference_birth(client, group, node, 23.5, False)
                elif action == "stale-death":
                    stale = tahu.Payload()
                    tahu.addMetric(stale, "bdSeq", None, tahu.MetricDataType.Int64, 40, now())
                    self.publish(client, group, "NDEATH", node, stale)
                elif action == "abort":
                    client.loop_stop()
                    connection = client.socket()
                    connection.shutdown(socket.SHUT_RDWR)
                    connection.close()
                    del self.references[key]
                else:
                    raise ValueError("Unknown Tahu edge action")

    def reference_birth(self, client, group, node, temperature, running):
        birth = tahu.getNodeBirthPayload()
        tahu.addMetric(birth, "temperature", 10, tahu.MetricDataType.Double, temperature, now())
        tahu.addMetric(birth, "label", 11, tahu.MetricDataType.String, "温度センサー", now())
        tahu.addMetric(birth, "raw", 12, tahu.MetricDataType.Bytes, bytes([0, 255, 128]), now())
        tahu.addMetric(birth, "Node Control/Rebirth", None, tahu.MetricDataType.Boolean, False, now())
        self.publish(client, group, "NBIRTH", node, birth)
        birth = tahu.getDeviceBirthPayload()
        tahu.addMetric(birth, "running", 20, tahu.MetricDataType.Boolean, running, now())
        self.publish(client, group, "DBIRTH", node, birth, "device")


class Handler(BaseHTTPRequestHandler):
    def respond(self, body, status=200):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        request = urllib.parse.urlparse(self.path)
        if request.path == "/health":
            self.respond({"ready": self.server.oracle.client.is_connected(), "implementation": "Eclipse Tahu v1.0.18"})
        elif request.path == "/state":
            args = urllib.parse.parse_qs(request.query)
            self.respond(self.server.oracle.snapshot(args["group"][0], args["node"][0]))
        else:
            self.respond({"error": "unknown endpoint"}, 404)

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 4096:
                raise ValueError("Expected a bounded JSON request")
            args = json.loads(self.rfile.read(length))
            group, node = args["group"], args["node"]
            if not group.startswith("flow-like-e2e-") or any(c in group + node for c in "/+#\0"):
                raise ValueError("Expected test group and node identifiers")
            if self.path == "/rebirth-command":
                self.server.oracle.rebirth_command(group, node)
            elif self.path.startswith("/reference/"):
                self.server.oracle.reference(self.path.rsplit("/", 1)[1], group, node)
            else:
                self.respond({"error": "unknown endpoint"}, 404)
                return
            self.respond({"ok": True})
        except Exception as error:
            self.respond({"error": f"{type(error).__name__}: {error}"}, 500)


if __name__ == "__main__":
    if sys.argv[1:] == ["--healthcheck"]:
        with urllib.request.urlopen("http://127.0.0.1:8080/health", timeout=3) as response:
            assert json.load(response)["ready"]
    else:
        server = ThreadingHTTPServer(("0.0.0.0", 8080), Handler)
        server.oracle = Oracle()
        server.serve_forever()
