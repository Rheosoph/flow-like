"""Validate benchmark process topology and complete request admission evidence."""

from collections import Counter, defaultdict
import math


POOLS = {"api_frontend": "apiFrontendReplicas", "api_callback": "apiCallbackReplicas",
         "runtime": "executorReplicas"}
CALLBACK_CATEGORIES = {"execution_quota", "execution_jwks", "execution_progress",
                       "execution_events", "execution_widgets", "execution_channel"}
FRONTEND_CATEGORIES = {"frontend_invoke", "frontend_bootstrap", "frontend_prerun"}
EXECUTOR_CATEGORIES = {"executor_sse", "executor_stream", "executor_execute"}
OPTIONAL_RESPONSE_POLICY = [{"method": "GET", "pathCategory": "frontend_telemetry_config", "status": 404}]


def positive_integer(value):
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def finite(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def shared_topology(worker_threads, executor_slots):
    """Old reports used one multi-thread Tokio API and executor, without HTTP admission."""
    if not positive_integer(worker_threads) or not positive_integer(executor_slots):
        raise ValueError("shared topology requires positive worker_threads and executor_slots")
    return {"kind": "shared-process", "apiFrontendReplicas": 1, "apiCallbackReplicas": 0,
            "executorReplicas": 1, "requestsPerProcess": None, "workerThreads": worker_threads,
            "executorSlotsPerProcess": executor_slots, "callbackRouting": "shared-api",
            "leaseBoundary": "unbounded", "runtimeModel": "multi-thread",
            "cpuLimitEnforced": False, "memoryLimitEnforced": False}


def validate_topology(value):
    if not isinstance(value, dict):
        raise ValueError("execution_topology must be an object")
    kind = value.get("kind")
    expected = shared_topology(1, 1)
    if set(value) != set(expected):
        raise ValueError("execution_topology has missing or unknown fields")
    if kind == "shared-process":
        expected = shared_topology(value["workerThreads"], value["executorSlotsPerProcess"])
    elif kind == "replicated-local":
        expected.update({"kind": kind, "requestsPerProcess": 1, "workerThreads": 1,
                         "executorSlotsPerProcess": 1, "callbackRouting": "reserved-pool",
                         "leaseBoundary": "upstream-eof", "runtimeModel": "current-thread"})
        for field in POOLS.values():
            if not positive_integer(value[field]):
                raise ValueError(f"execution_topology.{field} must be a positive integer")
            expected[field] = value[field]
    else:
        raise ValueError("unknown execution_topology kind")
    # Equality alone would accept True for an integer and zero for False.
    if any(type(value[key]) is not type(wanted) or value[key] != wanted
           for key, wanted in expected.items()):
        raise ValueError(f"invalid {kind} execution_topology configuration")
    return dict(value)


def normalize_topology(manifest):
    """Only absent topology means the legacy shared arrangement; null is invalid."""
    if "execution_topology" not in manifest:
        return shared_topology(manifest.get("worker_threads"), manifest.get("executor_slots"))
    result = validate_topology(manifest["execution_topology"])
    for old, field in (("worker_threads", "workerThreads"), ("executor_slots", "executorSlotsPerProcess")):
        if old in manifest and (type(manifest[old]) is not int or manifest[old] != result[field]):
            raise ValueError(f"execution_topology.{field} disagrees with {old}")
    return result


def instance_identity(value):
    """Replica and generation identifiers retain their JSON types."""
    if not isinstance(value, dict) or not isinstance(value.get("pool"), str) or value["pool"] not in POOLS:
        raise ValueError("unknown or missing pool")
    identifiers = [value.get("replica"), value.get("generation")]
    if any(not isinstance(item, (str, int)) or isinstance(item, bool) or item == ""
           or isinstance(item, int) and item < 0
           for item in identifiers):
        raise ValueError("replica and generation must be nonempty strings or nonnegative integers")
    return value["pool"], *identifiers


def instance_record(identity):
    return dict(zip(("pool", "replica", "generation"), identity))


def monotonic_ns(value):
    if not isinstance(value, str) or not value.isascii() or not value.isdigit():
        raise ValueError("monotonic timestamps must be decimal nanosecond strings")
    return int(value)


def validate_admissions(logrecords, processes, topology, *, allowed_frontend_disconnects=None, allowed_optional_responses=None):
    """Validate all admissions, including failures; never discard a failed fast request.

    A lease lasts until upstream EOF, including draining after a disconnected client.
    Inventory may include several generations when a cold trial replaces the pool.
    Idle instances are reported; callers check warmup and cold-burst coverage separately.
    """
    issues, inventory, process_ids = [], {}, set()
    allowed_frontend_disconnects = set(allowed_frontend_disconnects or ())
    allowed_optional_responses = set(allowed_optional_responses or ())
    try:
        topology = validate_topology(topology)
        if topology["kind"] != "replicated-local":
            raise ValueError("admission evidence requires replicated-local topology")
    except ValueError as error:
        return {"status": "invalid", "issues": [str(error)], "instances": [], "requests": []}
    if not isinstance(processes, list) or not processes:
        issues.append("process inventory is missing")
        processes = []
    for index, process in enumerate(processes):
        try:
            identity = instance_identity(process)
        except ValueError as error:
            issues.append(f"process {index}: {error}")
            continue
        if identity in inventory:
            issues.append(f"duplicate process instance {identity}")
        inventory[identity] = process
        pid, started = process.get("pid"), process.get("started")
        if not positive_integer(pid) or not isinstance(started, str) or not started:
            issues.append(f"process {identity}: missing PID/start identity")
        elif (pid, started) in process_ids:
            issues.append(f"process {identity}: reused PID/start identity")
        else:
            process_ids.add((pid, started))
        sha = process.get("binarySha256")
        if not isinstance(sha, str) or len(sha) != 64 or any(char not in "0123456789abcdef" for char in sha):
            issues.append(f"process {identity}: missing binary SHA256")
        if process.get("runtimeModel") != topology["runtimeModel"]:
            issues.append(f"process {identity}: runtime model differs from topology")
        if type(process.get("workerThreads")) is not int or process["workerThreads"] != topology["workerThreads"]:
            issues.append(f"process {identity}: worker count differs from topology")
    generations = {identity[2] for identity in inventory}
    if len({process.get("binarySha256") for process in inventory.values()
            if isinstance(process.get("binarySha256"), str)}) > 1:
        issues.append("process inventory mixes backend binaries within one trial")
    for generation in generations:
        counts = Counter(identity[0] for identity in inventory if identity[2] == generation)
        for pool, field in POOLS.items():
            if counts[pool] != topology[field]:
                issues.append(f"generation {generation}: {pool} inventory is {counts[pool]}, expected {topology[field]}")

    requests = defaultdict(dict)
    if not isinstance(logrecords, list) or not logrecords:
        issues.append("admission log is missing")
        logrecords = []
    for index, record in enumerate(logrecords):
        if not isinstance(record, dict):
            issues.append(f"admission record {index}: must be an object")
            continue
        request_id, event = record.get("requestId"), record.get("event")
        if not isinstance(request_id, str) or not request_id:
            issues.append(f"admission record {index}: missing requestId")
            continue
        if event == "rejected":
            issues.append(f"request {request_id}: admission rejected ({record.get('failureReason', 'unknown')})")
            continue
        if event not in ("admitted", "released"):
            issues.append(f"request {request_id}: unknown admission event")
            continue
        if event in requests[request_id]:
            issues.append(f"request {request_id}: duplicate {event} event")
        requests[request_id][event] = record

    completed, intervals, observed, clock_modes = [], defaultdict(list), Counter(), set()
    common = ("pool", "replica", "generation", "method", "routeKey", "pathCategory",
              "queuedAtUnixMs", "admittedAtUnixMs")
    mono_fields = ("queuedAtMonotonicNs", "admittedAtMonotonicNs", "releasedAtMonotonicNs")
    for request_id, events in requests.items():
        label = f"request {request_id}"
        if set(events) != {"admitted", "released"}:
            issues.append(f"{label}: admission/release pair is incomplete")
            continue
        admitted, released = events["admitted"], events["released"]
        for record in (admitted, released):
            if any(field not in record for field in (*common, "releasedAtUnixMs", "status",
                                                     "upstreamComplete", "downstreamDisconnected")):
                issues.append(f"{label}: {record['event']} event has missing fields")
        try:
            identity = instance_identity(admitted)
        except ValueError as error:
            issues.append(f"{label}: {error}")
            continue
        if identity not in inventory:
            issues.append(f"{label}: instance {identity} is absent from process inventory")
        observed[identity] += 1
        for field in common:
            if field not in admitted or field not in released or type(admitted[field]) is not type(released[field]) or admitted[field] != released[field]:
                issues.append(f"{label}: changed or missing {field} across admission/release")
        for field in ("method", "routeKey", "pathCategory"):
            if not isinstance(admitted.get(field), str) or not admitted[field]:
                issues.append(f"{label}: missing {field}")
        category, pool = admitted.get("pathCategory"), identity[0]
        expected_pool = ("api_callback" if category in CALLBACK_CATEGORIES else
                         "api_frontend" if category in FRONTEND_CATEGORIES else
                         "runtime" if category in EXECUTOR_CATEGORIES else None)
        if expected_pool is not None and pool != expected_pool:
            issues.append(f"{label}: {category} used {pool}, expected {expected_pool}")
        if (admitted.get("releasedAtUnixMs") is not None or admitted.get("status") is not None
                or admitted.get("upstreamComplete") is not False or admitted.get("downstreamDisconnected") is not False):
            issues.append(f"{label}: invalid admitted state")
        unix = [released.get(field) for field in ("queuedAtUnixMs", "admittedAtUnixMs", "releasedAtUnixMs")]
        unix_valid = all(finite(value) for value in unix) and unix[0] <= unix[1] <= unix[2]
        if not unix_valid:
            issues.append(f"{label}: invalid Unix timestamp ordering")
        has_mono = any(field in record for record in (admitted, released) for field in mono_fields)
        clock_modes.add(has_mono)
        timeline = unix if unix_valid else None
        if has_mono:
            try:
                mono = [monotonic_ns(released.get(field)) for field in mono_fields]
                if not mono[0] <= mono[1] <= mono[2]:
                    raise ValueError("invalid monotonic timestamp ordering")
                if (any(admitted.get(field) != released.get(field) for field in mono_fields[:2])
                        or admitted.get(mono_fields[2]) is not None):
                    raise ValueError("monotonic timestamps changed across admission/release")
                timeline = mono
            except ValueError as error:
                issues.append(f"{label}: {error}")
                timeline = None
        status = released.get("status")
        verified_cancellation = (request_id in allowed_frontend_disconnects and pool == "api_frontend"
                                 and category == "frontend_invoke" and admitted.get("method") == "POST")
        success = (isinstance(status, int) and not isinstance(status, bool) and 200 <= status < 300
                   and released.get("upstreamComplete") is True
                   and (released.get("downstreamDisconnected") is False
                        or released.get("downstreamDisconnected") is True and verified_cancellation)
                   and released.get("failureReason") is None)
        optional = (request_id in allowed_optional_responses and pool == "api_frontend"
                    and category == "frontend_telemetry_config" and admitted.get("method") == "GET" and type(status) is int and status == 404
                    and released.get("upstreamComplete") is True and released.get("downstreamDisconnected") is False
                    and released.get("failureReason") is None)
        if not success and not optional:
            issues.append(f"{label}: failed, disconnected or incomplete upstream response")
        summary = {**instance_record(identity), "requestId": request_id,
                   "method": admitted.get("method"), "routeKey": admitted.get("routeKey"),
                   "pathCategory": category, "queuedAtUnixMs": unix[0], "admittedAtUnixMs": unix[1],
                   "releasedAtUnixMs": unix[2], "status": status, "successful": success,
                   "downstreamDisconnected": released.get("downstreamDisconnected"),
                   "verifiedTerminalCancellation": verified_cancellation and released.get("downstreamDisconnected") is True,
                   "optionalResponse": optional}
        if timeline is not None:
            scale = 1_000_000 if has_mono else 1
            summary.update({"admissionWaitMs": (timeline[1] - timeline[0]) / scale,
                            "leaseMs": (timeline[2] - timeline[1]) / scale})
            intervals[identity].append((timeline[1], timeline[2], request_id))
        completed.append(summary)
    eligible_ids = {request["requestId"] for request in completed if request["pool"] == "api_frontend"
                    and request["pathCategory"] == "frontend_invoke" and request["method"] == "POST"}
    if not allowed_frontend_disconnects <= eligible_ids:
        issues.append("verified browser invocation IDs are absent from frontend admission evidence")
    optional_ids = {request["requestId"] for request in completed if request["optionalResponse"]}
    if allowed_optional_responses != optional_ids:
        issues.append("optional browser response IDs do not match the exact allowed frontend response")
    if len(clock_modes) > 1:
        issues.append("admission log mixes monotonic and wall-clock-only timestamps")
    instances = []
    for identity, process in inventory.items():
        events = []
        for start, end, request_id in intervals[identity]:
            if end > start:
                events.extend(((start, 1, request_id), (end, -1, request_id)))
        active, peak = 0, 0
        for _, change, _ in sorted(events):
            active += change
            peak = max(peak, active)
        if peak > topology["requestsPerProcess"]:
            issues.append(f"instance {identity}: overlapping leases, peak active {peak}")
        instances.append({**instance_record(identity), "pid": process.get("pid"),
                          "started": process.get("started"), "observedRequests": observed[identity],
                          "peakActive": peak})
    return {"status": "invalid" if issues else "passed", "issues": issues,
            "instances": instances, "requests": completed,
            "clock": "monotonic" if clock_modes == {True} else "wall-clock"}


def verified_frontend_invocations(report, accounting):
    """Bind intentional terminal cancellation to proxy request IDs and persisted runs."""
    issues, identifiers = [], set()
    if not isinstance(accounting, dict) or accounting.get("status") != "confirmed":
        return identifiers, ["frontend cancellation requires confirmed persisted accounting"]
    accounted = accounting.get("runIds", [])
    if not isinstance(accounted, list) or not all(isinstance(value, str) and value for value in accounted):
        return identifiers, ["frontend cancellation requires persisted run identifiers"]
    for record in report.get("network", []):
        if record.get("category") != "invoke":
            continue
        execution = record.get("execution") or {}
        request_id = record.get("benchmarkRequestId")
        if (not isinstance(request_id, str) or not request_id or request_id in identifiers
                or record.get("method") != "POST" or type(record.get("status")) is not int
                or not 200 <= record["status"] < 300 or execution.get("success") is not True
                or execution.get("terminalObserved") is not True or execution.get("runId") not in accounted):
            issues.append("browser invocation lacks a unique proxy ID, terminal success or persisted accounting")
            continue
        identifiers.add(request_id)
    return identifiers, issues


def verified_optional_responses(report, policy, first_measured_click_unix_ms):
    """Old backends may lack the optional telemetry-config endpoint during bootstrap."""
    if policy == []:
        return set(), []
    if policy != OPTIONAL_RESPONSE_POLICY:
        return set(), ["unknown optional-response policy"]
    identifiers, issues = set(), []
    for record in report.get("network", []):
        if record.get("status") != 404:
            continue
        request_id = record.get("benchmarkRequestId")
        start, elapsed = record.get("startedAtUnixMs"), record.get("totalMs")
        if (not isinstance(request_id, str) or not request_id or request_id in identifiers
                or record.get("method") != "GET" or record.get("category") != "other_api"
                or record.get("phase") != "bootstrap" or record.get("networkError")
                or not finite(start) or not finite(elapsed) or not finite(first_measured_click_unix_ms)
                or start + elapsed > first_measured_click_unix_ms):
            issues.append("optional response lacks complete bootstrap timing or a unique proxy request ID")
            continue
        identifiers.add(request_id)
    return identifiers, issues


def expected_instance_set(instances):
    return {instance_identity(instance) for instance in instances}


def validate_warm_coverage(validation, expected_instances, warmup_request_ids, before_unix_ms):
    """The caller selects verified warmup request IDs, excluding readiness traffic."""
    issues = []
    try:
        expected = expected_instance_set(expected_instances)
    except (ValueError, TypeError) as error:
        return [f"invalid warmup instances: {error}"]
    if not expected or not finite(before_unix_ms):
        return ["warmup requires expected instances and a finite measurement boundary"]
    if validation.get("status") != "passed":
        issues.append("warmup admission evidence is invalid")
    requested = set(warmup_request_ids)
    covered, found = set(), set()
    for request in validation.get("requests", []):
        if request["requestId"] not in requested:
            continue
        found.add(request["requestId"])
        if (request["successful"] and request["pathCategory"] != "health"
                and finite(request["releasedAtUnixMs"]) and request["releasedAtUnixMs"] <= before_unix_ms):
            covered.add(instance_identity(request))
        else:
            issues.append(f"request {request['requestId']}: warmup was not successful before measurement")
    if requested != found:
        issues.append("warmup request IDs are missing from admission evidence")
    if covered != expected:
        issues.append(f"warmup instance coverage differs: missing {len(expected - covered)}, unexpected {len(covered - expected)}")
    return issues


def validate_cold_window(validation, expected_instances, ready_unix_ms, first_click_unix_ms):
    """A fresh generation may receive health probes before its first measured click."""
    try:
        expected = expected_instance_set(expected_instances)
    except (ValueError, TypeError) as error:
        return [f"invalid cold instances: {error}"]
    if (not expected or not finite(ready_unix_ms) or not finite(first_click_unix_ms)
            or ready_unix_ms > first_click_unix_ms):
        return ["cold window requires expected instances and ordered readiness/click boundaries"]
    issues = [] if validation.get("status") == "passed" else ["cold admission evidence is invalid"]
    known = expected_instance_set(validation.get("instances", []))
    if not expected <= known:
        issues.append("cold instances are missing from process inventory")
    for request in validation.get("requests", []):
        if (instance_identity(request) in expected and request["pathCategory"] != "health"
                and finite(request["queuedAtUnixMs"]) and request["queuedAtUnixMs"] < first_click_unix_ms):
            issues.append(f"request {request['requestId']}: fresh instance received traffic before the cold click")
    return issues
