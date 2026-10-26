import copy
import unittest

import topology


def replicated(replicas=1):
    return {"kind": "replicated-local", "apiFrontendReplicas": replicas, "apiCallbackReplicas": replicas,
            "executorReplicas": replicas, "requestsPerProcess": 1, "workerThreads": 1,
            "executorSlotsPerProcess": 1, "callbackRouting": "reserved-pool", "leaseBoundary": "upstream-eof",
            "runtimeModel": "current-thread", "cpuLimitEnforced": False, "memoryLimitEnforced": False}


def process_inventory(replicas=1, generation=0, first_pid=100):
    return [{"pool": pool, "replica": replica, "generation": generation,
             "pid": first_pid + role * replicas + replica, "started": "2026-10-26T12:00:00Z",
             "binarySha256": "a" * 64, "runtimeModel": "current-thread", "workerThreads": 1}
            for role, pool in enumerate(topology.POOLS) for replica in range(replicas)]


def request(request_id, process, *, queued=0, admitted=10, released=20, category=None, monotonic=True):
    category = category or {"api_frontend": "frontend_invoke", "api_callback": "execution_quota",
                            "runtime": "executor_sse"}[process["pool"]]
    base = {key: process[key] for key in ("pool", "replica", "generation")}
    base.update({"requestId": request_id, "method": "POST", "routeKey": "route-hash", "pathCategory": category,
                 "queuedAtUnixMs": queued, "admittedAtUnixMs": admitted,
                 "releasedAtUnixMs": None, "status": None, "upstreamComplete": False,
                 "downstreamDisconnected": False, "failureReason": None})
    if monotonic:
        base.update({"queuedAtMonotonicNs": str(int(queued * 1_000_000)),
                     "admittedAtMonotonicNs": str(int(admitted * 1_000_000)), "releasedAtMonotonicNs": None})
    start = {**base, "event": "admitted"}
    end = {**base, "event": "released", "releasedAtUnixMs": released, "status": 200, "upstreamComplete": True}
    if monotonic:
        end["releasedAtMonotonicNs"] = str(int(released * 1_000_000))
    return [start, end]


def fixture(replicas=1):
    processes = process_inventory(replicas)
    records = [event for index, process in enumerate(processes)
               for event in request(f"request-{index}", process)]
    return records, processes, replicated(replicas)


class TopologyConfiguration(unittest.TestCase):
    def test_absent_topology_is_the_known_legacy_arrangement(self):
        legacy = {"worker_threads": 1, "executor_slots": 8}
        explicit = {**legacy, "execution_topology": topology.shared_topology(1, 8)}
        self.assertEqual(topology.normalize_topology(legacy), topology.normalize_topology(explicit))
        self.assertEqual(topology.normalize_topology(legacy)["apiCallbackReplicas"], 0)

    def test_explicit_null_or_malformed_topology_cannot_fall_back_to_legacy(self):
        invalid = [None, [], {}, {**replicated(), "requestsPerProcess": True},
                   {**replicated(), "cpuLimitEnforced": 0}, {**replicated(), "kind": "lambda"},
                   {**replicated(), "apiFrontendReplicas": 0}, {**replicated(), "newSetting": True}]
        for value in invalid:
            with self.subTest(value=value), self.assertRaises(ValueError):
                topology.normalize_topology({"worker_threads": 1, "executor_slots": 1, "execution_topology": value})

    def test_manifest_cannot_disagree_with_its_topology(self):
        for field in ("worker_threads", "executor_slots"):
            manifest = {"worker_threads": 1, "executor_slots": 1, "execution_topology": replicated()}
            manifest[field] = 8
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "disagrees"):
                topology.normalize_topology(manifest)

    def test_missing_legacy_capacity_is_not_evidence_of_equal_topology(self):
        for manifest in ({}, {"worker_threads": 1}, {"worker_threads": True, "executor_slots": 8}):
            with self.subTest(manifest=manifest), self.assertRaises(ValueError):
                topology.normalize_topology(manifest)


class AdmissionEvidence(unittest.TestCase):
    def assess(self, records=None, processes=None, arrangement=None):
        default_records, default_processes, default_topology = fixture()
        return topology.validate_admissions(default_records if records is None else records,
                                            default_processes if processes is None else processes,
                                            default_topology if arrangement is None else arrangement)

    def test_complete_interleaved_evidence_recomputes_peak_and_wait(self):
        records, processes, arrangement = fixture(8)
        result = self.assess(list(reversed(records)), processes, arrangement)
        self.assertEqual(result["issues"], [])
        self.assertEqual(len(result["instances"]), 24)
        self.assertEqual({item["peakActive"] for item in result["instances"]}, {1})
        self.assertEqual({item["admissionWaitMs"] for item in result["requests"]}, {10})
        self.assertEqual(result["clock"], "monotonic")

    def test_adjacent_leases_are_valid_but_overlap_is_not(self):
        records, processes, _ = fixture()
        adjacent = request("next", processes[0], queued=5, admitted=20, released=40)
        self.assertEqual(self.assess(records + adjacent)["issues"], [])
        overlapping = request("next", processes[0], queued=5, admitted=19, released=40)
        result = self.assess(records + overlapping)
        self.assertTrue(any("overlapping leases" in issue for issue in result["issues"]))
        self.assertEqual(result["instances"][0]["peakActive"], 2)

    def test_missing_duplicate_and_reused_request_events_are_invalid(self):
        records, processes, _ = fixture()
        cases = [records[:-1], records + [records[0]], records + [records[1]],
                 records + request("request-0", processes[1], admitted=40, released=50)]
        for value in cases:
            with self.subTest(records=value):
                self.assertEqual(self.assess(value)["status"], "invalid")

    def test_pause_rejections_and_missing_admission_fields_cannot_disappear(self):
        records, _, _ = fixture()
        rejected = {"event": "rejected", "requestId": "paused-request", "failureReason": "admission_paused"}
        self.assertTrue(any("admission rejected" in issue for issue in self.assess(records + [rejected])["issues"]))
        records[0].pop("status")
        self.assertEqual(self.assess(records)["status"], "invalid")

    def test_response_headers_or_completed_sse_event_do_not_release_a_lease(self):
        for changes in ({"upstreamComplete": False}, {"status": 403}, {"status": None},
                        {"downstreamDisconnected": True}, {"failureReason": "upstream_error"}):
            records, _, _ = fixture()
            records[1].update(changes)
            with self.subTest(changes=changes):
                result = self.assess(records)
                self.assertEqual(result["status"], "invalid")
                self.assertEqual(len(result["requests"]), 3)
                self.assertFalse(result["requests"][0]["successful"])

    def test_proven_frontend_terminal_cancellation_still_requires_full_upstream_eof(self):
        records, processes, arrangement = fixture()
        records[1]["downstreamDisconnected"] = True
        result = topology.validate_admissions(records, processes, arrangement, allowed_frontend_disconnects={"request-0"})
        self.assertEqual(result["issues"], [])
        self.assertTrue(result["requests"][0]["verifiedTerminalCancellation"])
        self.assertTrue(result["requests"][0]["downstreamDisconnected"])
        records[1]["upstreamComplete"] = False
        self.assertEqual(topology.validate_admissions(records, processes, arrangement,
                         allowed_frontend_disconnects={"request-0"})["status"], "invalid")
        records[1]["upstreamComplete"] = True
        self.assertEqual(topology.validate_admissions(records, processes, arrangement,
                         allowed_frontend_disconnects={"request-1"})["status"], "invalid")

    def test_optional_telemetry_fallback_is_narrow_and_cannot_warm_an_instance(self):
        records, processes, arrangement = fixture()
        for record in records[:2]:
            record.update(pathCategory="frontend_telemetry_config", method="GET")
        records[1]["status"] = 404
        result = topology.validate_admissions(records, processes, arrangement, allowed_optional_responses={"request-0"})
        self.assertEqual(result["issues"], [])
        self.assertFalse(result["requests"][0]["successful"])
        self.assertTrue(result["requests"][0]["optionalResponse"])
        self.assertTrue(topology.validate_warm_coverage(result, processes, {"request-0", "request-1", "request-2"}, 30))
        for record in records[:2]:
            record["pathCategory"] = "other"
        self.assertEqual(topology.validate_admissions(records, processes, arrangement,
                         allowed_optional_responses={"request-0"})["status"], "invalid")

    def test_changed_instance_or_unknown_generation_is_invalid(self):
        for field, value in (("pool", "runtime"), ("replica", 5), ("generation", 1)):
            records, _, _ = fixture()
            records[1][field] = value
            with self.subTest(field=field):
                self.assertTrue(any("across admission/release" in issue for issue in self.assess(records)["issues"]))
            records[0][field] = value
            self.assertEqual(self.assess(records)["status"], "invalid")

    def test_callbacks_cannot_consume_the_frontend_pool(self):
        for category in topology.CALLBACK_CATEGORIES:
            records, _, _ = fixture()
            for record in records[:2]:
                record["pathCategory"] = category
            with self.subTest(category=category):
                self.assertTrue(any("expected api_callback" in issue for issue in self.assess(records)["issues"]))

    def test_process_inventory_requires_counts_unique_processes_and_configuration(self):
        _, processes, _ = fixture()
        cases = [processes[:-1], processes + [processes[0]]]
        for field, value in (("pid", 0), ("started", ""), ("binarySha256", ""),
                             ("runtimeModel", "multi-thread"), ("workerThreads", True)):
            changed = copy.deepcopy(processes)
            changed[0][field] = value
            cases.append(changed)
        reused = copy.deepcopy(processes)
        reused[1]["pid"] = reused[0]["pid"]
        cases.append(reused)
        mixed = copy.deepcopy(processes)
        mixed[1]["binarySha256"] = "b" * 64
        cases.append(mixed)
        for value in cases:
            with self.subTest(processes=value):
                self.assertEqual(self.assess(processes=value)["status"], "invalid")

    def test_each_generation_requires_its_own_complete_inventory(self):
        records, processes, arrangement = fixture(2)
        replacement = process_inventory(2, generation=1, first_pid=1000)
        records += [event for index, process in enumerate(replacement)
                    for event in request(f"new-{index}", process, queued=50, admitted=60, released=70)]
        self.assertEqual(self.assess(records, processes + replacement, arrangement)["issues"], [])
        self.assertEqual(self.assess(records, processes + replacement[:-1], arrangement)["status"], "invalid")

    def test_timestamp_ordering_and_clock_consistency_are_required(self):
        cases = []
        for field, value in (("releasedAtUnixMs", 5), ("releasedAtMonotonicNs", "5000000"),
                             ("releasedAtMonotonicNs", 20000000), ("releasedAtUnixMs", float("nan"))):
            records, _, _ = fixture()
            records[1][field] = value
            cases.append(records)
        records, processes, _ = fixture()
        cases.append(records + request("wall-clock", processes[0], admitted=40, released=50, monotonic=False))
        for value in cases:
            with self.subTest(records=value):
                self.assertEqual(self.assess(value)["status"], "invalid")

    def test_monotonic_timestamps_determine_overlap(self):
        records, processes, _ = fixture()
        later_wall_clock = request("next", processes[0], queued=100, admitted=110, released=120)
        for event in later_wall_clock:
            event["queuedAtMonotonicNs"] = "5000000"
            event["admittedAtMonotonicNs"] = "19000000"
        later_wall_clock[1]["releasedAtMonotonicNs"] = "25000000"
        self.assertTrue(any("overlapping leases" in issue for issue in self.assess(records + later_wall_clock)["issues"]))

    def test_all_wall_clock_evidence_is_supported_and_labeled(self):
        _, processes, _ = fixture()
        records = [event for index, process in enumerate(processes)
                   for event in request(f"request-{index}", process, monotonic=False)]
        result = self.assess(records)
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["clock"], "wall-clock")

    def test_malformed_log_or_inventory_is_not_accepted(self):
        for value in (None, [], [None], [{"requestId": "x", "event": "queued"}]):
            with self.subTest(records=value):
                self.assertEqual(topology.validate_admissions(value, process_inventory(), replicated())["status"], "invalid")
        for value in (None, [], [None], [{"pool": []}]):
            with self.subTest(processes=value):
                self.assertEqual(topology.validate_admissions(fixture()[0], value, replicated())["status"], "invalid")


class MeasurementBoundaries(unittest.TestCase):
    def test_frontend_cancellation_needs_exact_browser_and_accounting_evidence(self):
        report = {"network": [{"category": "invoke", "benchmarkRequestId": "request-1", "method": "POST", "status": 200,
                               "execution": {"runId": "run-1", "success": True, "terminalObserved": True}}]}
        accounting = {"status": "confirmed", "runIds": ["run-1"]}
        self.assertEqual(topology.verified_frontend_invocations(report, accounting), ({"request-1"}, []))
        self.assertTrue(topology.verified_frontend_invocations(report, {"status": "confirmed", "runIds": []})[1])
        report["network"].append(copy.deepcopy(report["network"][0]))
        self.assertTrue(topology.verified_frontend_invocations(report, accounting)[1])

    def test_every_required_instance_must_finish_verified_warmup(self):
        records, processes, arrangement = fixture(2)
        validation = topology.validate_admissions(records, processes, arrangement)
        identifiers = [record["requestId"] for record in records if record["event"] == "released"]
        self.assertEqual(topology.validate_warm_coverage(validation, processes, identifiers, 30), [])
        self.assertTrue(topology.validate_warm_coverage(validation, processes, identifiers[:-1], 30))
        self.assertTrue(topology.validate_warm_coverage(validation, processes, identifiers, 19))
        self.assertTrue(topology.validate_warm_coverage(validation, processes, identifiers + ["missing"], 30))

    def test_readiness_health_request_cannot_count_as_warmup(self):
        records, processes, arrangement = fixture()
        for record in records:
            record["pathCategory"] = "health"
        validation = topology.validate_admissions(records, processes, arrangement)
        self.assertTrue(topology.validate_warm_coverage(validation, processes, ["request-0", "request-1", "request-2"], 30))

    def test_cold_pool_allows_health_only_before_earliest_click(self):
        records, processes, arrangement = fixture()
        health = request("health", processes[0], queued=0, admitted=1, released=2, category="health")
        for record in records:
            record["queuedAtUnixMs"] += 100
            record["admittedAtUnixMs"] += 100
            if record["releasedAtUnixMs"] is not None:
                record["releasedAtUnixMs"] += 100
            for field in ("queuedAtMonotonicNs", "admittedAtMonotonicNs", "releasedAtMonotonicNs"):
                if record[field] is not None:
                    record[field] = str(int(record[field]) + 100_000_000)
        validation = topology.validate_admissions(health + records, processes, arrangement)
        self.assertEqual(topology.validate_cold_window(validation, processes, 50, 100), [])
        self.assertTrue(topology.validate_cold_window(validation, processes, 50, 101))
        self.assertTrue(topology.validate_cold_window(validation, processes, 101, 100))

    def test_pre_readiness_work_also_warms_a_cold_generation(self):
        records, processes, arrangement = fixture()
        validation = topology.validate_admissions(records, processes, arrangement)
        self.assertTrue(topology.validate_cold_window(validation, processes, 50, 100))
        unknown = process_inventory(generation=1)
        self.assertTrue(topology.validate_cold_window(validation, unknown, 50, 100))


if __name__ == "__main__":
    unittest.main()
