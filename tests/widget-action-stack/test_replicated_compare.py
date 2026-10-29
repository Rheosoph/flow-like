import copy
import json
from pathlib import Path
import tempfile
import unittest

import replicated_compare
import topology
from test_compare import bundle
from test_topology import process_inventory, replicated, request


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def proxy_status(records, processes):
    return {"closing": False, "paused": False, "pools": {
        pool: {"failed": False, "active": 0, "queued": 0, "quarantined": 0, "failures": 0, "rejected": 0,
               "admitted": sum(record["pool"] == pool and record["event"] == "admitted" for record in records),
               "released": sum(record["pool"] == pool and record["event"] == "released" for record in records),
               "targets": [{"id": process["replica"], "generation": process["generation"], "state": "idle", "requestId": None}
                           for process in processes if process["pool"] == pool]}
        for pool in ("api_frontend", "api_callback", "runtime")}}


def fixture(directory, *, pairs=2, replicas=2, modes=("warm", "navigation", "action"), layout_experiment=False):
    suite = {"schemaVersion": 1, "mode": "paired-replicated-process", "status": "passed",
             "configuration": {"trials": pairs, "modes": list(modes), "replicas": replicas, "concurrency": replicas,
                               "warmSamples": replicas * 2, "warmup": 1, "workerThreads": 1, "executorSlots": 1,
                               "readinessPollMs": 20, "trace": False, "order": "alternating-ab-ba"},
             "binaries": {"baseline": {"sha256": "a" * 64}, "candidate": {"sha256": "b" * 64}},
             "frontends": {variant: {"sourceSha256": "c" * 64, "buildId": "build"} for variant in ("baseline", "candidate")},
             "trials": []}
    if layout_experiment:
        suite["binaries"]["candidate"] = copy.deepcopy(suite["binaries"]["baseline"])
        suite["fixtureLayoutExperiment"] = {
            "states": {variant: str(directory / variant) for variant in ("baseline", "candidate")},
            "fixtures": {},
        }
    next_pid = 100
    for mode in modes:
        for pair in range(pairs):
            for position, variant in enumerate(("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline")):
                initial = process_inventory(replicas, first_pid=next_pid)
                next_pid += replicas * 3
                final = process_inventory(replicas, generation=1, first_pid=next_pid) if mode == "action" else initial
                if mode == "action":
                    next_pid += replicas * 3
                processes = initial + final if mode == "action" else initial
                for process in processes:
                    process["binarySha256"] = suite["binaries"][variant]["sha256"]
                    process["name"] = f"{process['pool']}-{process['replica']}"
                value = bundle()
                report, manifest = value["report"], value["manifest"]
                if layout_experiment:
                    compact = variant == "candidate"
                    manifest["fixture"]["database"].update(
                        layout="compact" if compact else "fragmented", fragments=1 if compact else 36,
                        physical_rows=47 if compact else 58, deletion_files=0 if compact else 3,
                        records_sha256="d" * 64, schema_sha256="e" * 64)
                    suite["fixtureLayoutExperiment"]["fixtures"][variant] = copy.deepcopy(manifest["fixture"])
                sample_count = replicas * (2 if mode == "warm" else 1)
                report["configuration"].update(samples=sample_count, warmup=1 if mode == "warm" else 0, concurrency=replicas)
                report["web"]["build"] = {"source": {"sha256": "c" * 64}, "buildId": "build"}
                manifest.update(worker_threads=1, executor_slots=1, execution_topology=replicated(replicas),
                                binary_sha256=suite["binaries"][variant]["sha256"], readiness_poll_ms=20, trace_requested=False,
                                database_pool={"maxConnections": 4, "minConnections": 0},
                                allowedOptionalResponses=topology.OPTIONAL_RESPONSE_POLICY)
                report.update(navigations=[], samples=[], network=[], resets=[])
                records = []
                run_prefix = f"{mode}-{pair}-{variant}"
                nav_ids, all_ids = [], []
                for worker in range(replicas):
                    identifier = f"{run_prefix}-nav-{worker}"
                    nav_ids.append(identifier)
                    report["navigations"].append({"worker": worker, "index": 0, "status": "ok", "cardsDomMs": 100,
                                                  "bootstrapValidatedAtNavigationMs": 200, "browserTimeOriginMs": 50 + worker,
                                                  "executionSuccessVerified": True, "execution": [{"runId": identifier, "success": True}]})
                    for category in ("bootstrap", "prerun", "invoke"):
                        report["network"].append({"worker": worker, "navigation": 0, "phase": "bootstrap", "category": category,
                                                  "routeKey": category, "status": 200, "method": "POST", "totalMs": 10, "startedAtUnixMs": 100,
                                                  **({"benchmarkRequestId": f"bootstrap-{worker}", "execution": {
                                                      "runId": identifier, "success": True, "terminalObserved": True}} if category == "invoke" else {})})
                for index, process in enumerate(initial):
                    records.extend(request(f"bootstrap-{index}", process, queued=100, admitted=110, released=120))
                if mode == "warm":
                    for worker in range(replicas):
                        identifier = f"{run_prefix}-warm-{worker}"
                        all_ids.append(identifier)
                        report["samples"].append({"worker": worker, "navigation": 0, "index": 0, "burst": 0, "warmup": True,
                                                  "status": "ok", "executionSuccessVerified": True,
                                                  "execution": [{"runId": identifier, "success": True}]})
                        report["network"].append({"worker": worker, "navigation": 0, "phase": "warmup", "category": "invoke",
                                                  "method": "POST", "routeKey": "invoke", "status": 200, "totalMs": 10, "startedAtUnixMs": 400,
                                                  "benchmarkRequestId": f"warmup-{worker}", "execution": {
                                                      "runId": identifier, "success": True, "terminalObserved": True}})
                    for index, process in enumerate(initial):
                        records.extend(request(f"warmup-{index}", process, queued=400, admitted=410, released=420))
                for index in range(sample_count):
                    worker, burst = index % replicas, index // replicas
                    clicked = 1000 + burst * 200 + worker
                    identifier = f"{run_prefix}-action-{index}"
                    all_ids.append(identifier)
                    latency = 100 if variant == "baseline" else 90
                    report["samples"].append({"worker": worker, "navigation": 0, "index": index, "burst": burst, "warmup": False,
                                              "status": "ok", "executionSuccessVerified": True,
                                              "execution": [{"runId": identifier, "success": True}],
                                              "browserTimeOriginMs": 50 + worker, "clickAtNavigationMs": clicked - 50 - worker,
                                              "scheduledAtUnixMs": 990 + burst * 200, "scheduledToClickMs": 10 + worker,
                                              "clickToHistoryDomMs": latency, "clickToHistoryFrameOpportunityMs": latency + 10,
                                              "clickToSettledMs": latency + 20})
                    for category in ("prerun", "invoke"):
                        report["network"].append({"worker": worker, "navigation": 0, "phase": "action", "category": category,
                                                  "routeKey": category, "status": 200, "method": "POST", "totalMs": 10, "startedAtUnixMs": clicked + 1,
                                                  **({"benchmarkRequestId": f"action-{index}-api_frontend", "execution": {
                                                      "runId": identifier, "success": True, "terminalObserved": True}} if category == "invoke" else {})})
                    for process in (process for process in final if process["replica"] == worker):
                        records.extend(request(f"action-{index}-{process['pool']}", process, queued=clicked + 2,
                                               admitted=clicked + 3, released=clicked + 30))
                relative = f"{mode}/pair-{pair + 1:03d}-{variant}"
                path = directory / relative
                path.mkdir(parents=True)
                trial = {"mode": mode, "pair": pair, "position": position, "variant": variant, "directory": relative,
                         "status": "passed", "wrapperExitCode": 0, "issues": [], "topology": replicated(replicas),
                         "initialStartup": {"processes": copy.deepcopy(initial), "readyAtUnixMs": 10,
                                            "measurements_ms": {process["name"]: 100 + index for index, process in enumerate(initial)}}, "restart": None,
                         "readyAtUnixMs": 500 if mode == "action" else 10, "firstNavigationUnixMs": 50,
                         "firstMeasuredClickUnixMs": 1000, "warmupCount": replicas if mode == "warm" else 0,
                         "accountingBeforeStop": {"status": "confirmed", "runIds": nav_ids + all_ids, "durationMs": 1, "polls": 1},
                         "proxyStatus": proxy_status(records, final),
                         "cleanup": {"status": "ok", "ownedServiceProcessesExited": True, "processInventoryRemoved": True}}
                if layout_experiment:
                    trial["fixtureState"] = suite["fixtureLayoutExperiment"]["states"][variant]
                if mode == "action":
                    trial.update(restartRequestedAfterBootstrap=True,
                                 restart={"processes_before": copy.deepcopy(initial), "processes_after": copy.deepcopy(final), "readyAtUnixMs": 500,
                                          "measurements_ms": {process["name"]: 200 + index for index, process in enumerate(final)}},
                                 accountingBeforeRestart={"status": "confirmed", "runIds": nav_ids, "durationMs": 1, "polls": 1},
                                 idleBeforeRestart=proxy_status([record for record in records if record["generation"] == 0], initial))
                    report["coldBoundary"] = {"mode": "cold-action", "status": "confirmed", "workerCount": replicas,
                        "bootstrapExecutionSuccessVerified": True, "controllerRestartConfirmed": True, "documentPreserved": True,
                        "requestsCheckedThroughClick": True, "unexpectedApiRequests": 0, "unexpectedExecutionRequests": 0,
                        "startedAtUnixMs": 300, "pendingRequestsBeforeBoundary": {"status": "confirmed", "workerCount": replicas,
                            "pendingApiRequests": 0, "durationMs": 1, "polls": 1},
                        "workers": [{"worker": worker, "navigation": 0, "bootstrapExecutionSuccessVerified": True,
                                     "documentPreserved": True, "requestsCheckedThroughClick": True,
                                     "unexpectedApiRequests": 0, "unexpectedExecutionRequests": 0,
                                     "documentTimeOriginMs": 50 + worker, "checkedThroughUnixMs": 1000 + worker}
                                    for worker in range(replicas)]}
                write_json(path / "results.json", report)
                write_json(path / "run-manifest.json", manifest)
                write_json(path / "processes.json", processes)
                (path / "admissions.jsonl").write_text("".join(json.dumps(record) + "\n" for record in records))
                trial["evidence"] = {"admissionsSha256": replicated_compare.sha256(path / "admissions.jsonl"),
                                     "processesSha256": replicated_compare.sha256(path / "processes.json")}
                write_json(path / "replicated-trial.json", trial)
                suite["trials"].append(trial)
    write_json(directory / "replicated-results.json", suite)
    return suite


class ReplicatedComparison(unittest.TestCase):
    def test_layout_experiment_accepts_only_layout_changes_across_ab_ba_pairs(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=2, modes=("warm", "action"), layout_experiment=True)
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["fixtureLayoutExperiment"], suite["fixtureLayoutExperiment"])
            self.assertEqual(result["metrics"]["action"]["Click to history DOM: trial median"]["n"], 2)
            self.assertIn("physical fixture layout", replicated_compare.render(result))

    def test_normal_code_comparisons_reject_changed_physical_layout(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("warm",))
            path = directory / suite["trials"][1]["directory"] / "run-manifest.json"
            manifest = json.loads(path.read_text())
            manifest["fixture"]["database"]["fragments"] = 1
            write_json(path, manifest)
            result = replicated_compare.analyze(directory)
            self.assert_withheld(result)
            self.assertTrue(any("physical storage layout" in issue for issue in result["issues"]))

    def test_layout_experiment_rejects_confounds_and_changed_frozen_evidence(self):
        for mutation in ("binary", "frontend", "records", "schema", "graph", "trial-layout",
                         "trial-state", "missing-declaration", "invalid-declaration", "invalid-fixtures",
                         "missing-hash", "shared-state"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=("warm",), layout_experiment=True)
                experiment = suite["fixtureLayoutExperiment"]
                trial = suite["trials"][1]
                path = directory / trial["directory"] / "run-manifest.json"
                manifest = json.loads(path.read_text())
                if mutation == "binary":
                    suite["binaries"]["candidate"]["sha256"] = "f" * 64
                elif mutation == "frontend":
                    suite["frontends"]["candidate"]["buildId"] = "different"
                elif mutation in ("records", "schema"):
                    experiment["fixtures"]["candidate"]["database"][mutation + "_sha256"] = "f" * 64
                elif mutation == "graph":
                    experiment["fixtures"]["candidate"]["graph_fixture_sha256"] = "different"
                elif mutation == "trial-layout":
                    manifest["fixture"]["database"]["fragments"] = 2
                elif mutation == "trial-state":
                    trial["fixtureState"] = experiment["states"]["baseline"]
                elif mutation == "missing-declaration":
                    del suite["fixtureLayoutExperiment"]
                elif mutation == "invalid-declaration":
                    suite["fixtureLayoutExperiment"] = True
                elif mutation == "invalid-fixtures":
                    experiment["fixtures"] = []
                elif mutation == "missing-hash":
                    for variant in ("baseline", "candidate"):
                        del experiment["fixtures"][variant]["database"]["schema_sha256"]
                elif mutation == "shared-state":
                    experiment["states"]["candidate"] = experiment["states"]["baseline"]
                write_json(path, manifest)
                self.write_trial(directory, suite, 1)
                self.assert_withheld(replicated_compare.analyze(directory))

    def write_trial(self, directory, suite, index=0):
        write_json(directory / suite["trials"][index]["directory"] / "replicated-trial.json", suite["trials"][index])
        write_json(directory / "replicated-results.json", suite)

    def assert_withheld(self, result):
        self.assertEqual(result["status"], "invalid")
        self.assertTrue(result["issues"])
        for measurements in result["metrics"].values():
            for metric in measurements.values():
                self.assertIsNone(metric["pairedMedianDeltaMs"])
                self.assertIsNone(metric["pairedMedianDelta95CiMs"])

    def update_ledger(self, directory, suite, records, index=0):
        trial = suite["trials"][index]
        path = directory / trial["directory"]
        (path / "admissions.jsonl").write_text("".join(json.dumps(record) + "\n" for record in records))
        trial["evidence"]["admissionsSha256"] = replicated_compare.sha256(path / "admissions.jsonl")
        final = trial["restart"]["processes_after"] if trial["restart"] else trial["initialStartup"]["processes"]
        trial["proxyStatus"] = proxy_status(records, final)
        self.write_trial(directory, suite, index)

    def test_complete_eight_worker_trials_use_trial_pairs_not_action_count(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            fixture(directory, replicas=8, pairs=4, modes=("action",))
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            metric = result["metrics"]["action"]["Click to history DOM: trial median"]
            self.assertEqual(metric["n"], 4)
            self.assertEqual(metric["pairedMedianDeltaMs"], -10)
            self.assertEqual(metric["pairedMedianDelta95CiMs"], [-10, -10])
            self.assertEqual(result["actionLatencyDescriptive"]["action/baseline"]["n"], 32)
            self.assertEqual(result["repeatedRoutesWithinWorkerPhase"], [])
            self.assertEqual(result["bursts"]["action/0/baseline"][0]["scheduleToLastHistoryDomMs"], 117)
            pool = result["admissions"]["action/0/baseline"]["pools"]["runtime"]["measurementWindow"]
            self.assertEqual(pool["requests"], 8)
            self.assertEqual(pool["admissionWaitMs"]["p50"], 1)
            self.assertEqual(pool["leaseMs"]["p50"], 27)
            self.assertEqual(result["processReadiness"]["action/0/baseline"]["initial"]["runtime"]["n"], 8)
            self.assertEqual(result["processReadiness"]["action/0/baseline"]["restart"]["runtime"]["max"], 223)
            self.assertEqual(result["admissions"]["action/0/baseline"]["measuredHistoryFrontendInvocations"]["requests"], 8)

    def test_missing_process_readiness_does_not_silently_shrink_startup_sample(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("action",))
            suite["trials"][0]["restart"]["measurements_ms"].pop("runtime-1")
            self.write_trial(directory, suite)
            self.assert_withheld(replicated_compare.analyze(directory))

    def test_terminal_cancellation_requires_matching_browser_proxy_and_accounting(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("warm",))
            path = directory / suite["trials"][0]["directory"]
            records = [json.loads(line) for line in (path / "admissions.jsonl").read_text().splitlines()]
            selected = "action-0-api_frontend"
            for record in records:
                if record["requestId"] == selected and record["event"] == "released":
                    record["downstreamDisconnected"] = True
            report = json.loads((path / "results.json").read_text())
            invocation = next(record for record in report["network"] if record.get("benchmarkRequestId") == selected)
            invocation.update(cancelled=True, networkError="net::ERR_ABORTED")
            write_json(path / "results.json", report)
            self.update_ledger(directory, suite, records)
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["admissions"]["warm/0/baseline"]["pools"]["api_frontend"]["allRequests"]["verifiedTerminalCancellations"], 1)
            invocation["benchmarkRequestId"] = "unrelated-request"
            write_json(path / "results.json", report)
            self.assert_withheld(replicated_compare.analyze(directory))

    def test_optional_404_requires_exact_bootstrap_capability_request(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("warm",))
            trial = suite["trials"][0]
            path = directory / trial["directory"]
            records = [json.loads(line) for line in (path / "admissions.jsonl").read_text().splitlines()]
            optional = request("optional", trial["initialStartup"]["processes"][0], queued=300, admitted=310, released=320,
                               category="frontend_telemetry_config")
            for record in optional:
                record["method"] = "GET"
            optional[1]["status"] = 404
            records.extend(optional)
            report = json.loads((path / "results.json").read_text())
            report["network"].append({"category": "other_api", "phase": "bootstrap", "worker": 0, "navigation": 0,
                                      "method": "GET", "status": 404, "totalMs": 20, "startedAtUnixMs": 300,
                                      "benchmarkRequestId": "optional"})
            write_json(path / "results.json", report)
            self.update_ledger(directory, suite, records)
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["admissions"]["warm/0/baseline"]["pools"]["api_frontend"]["allRequests"]["optionalResponses"], 1)
            optional[0]["pathCategory"] = optional[1]["pathCategory"] = "other"
            self.update_ledger(directory, suite, records)
            self.assert_withheld(replicated_compare.analyze(directory))

    def test_negative_cdp_preflight_time_requires_proxy_proof_and_never_covers_invoke_time(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("warm",))
            trial = suite["trials"][0]
            path = directory / trial["directory"]
            records = [json.loads(line) for line in (path / "admissions.jsonl").read_text().splitlines()]
            callback = next(process for process in trial["initialStartup"]["processes"] if process["pool"] == "api_callback")
            options = request("options", callback, queued=300, admitted=310, released=320, category="execution_channel")
            for record in options:
                record["method"] = "OPTIONS"
            records.extend(options)
            report = json.loads((path / "results.json").read_text())
            preflight = {"category": "other_api_preflight", "phase": "bootstrap", "worker": 0, "navigation": 0,
                         "method": "OPTIONS", "status": 200, "totalMs": -4.5, "startedAtUnixMs": 300,
                         "responseHeadersMs": .4, "benchmarkRequestId": "options"}
            report["network"].append(preflight)
            write_json(path / "results.json", report)
            self.update_ledger(directory, suite, records)
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            self.assertEqual(len(result["warnings"]), 1)
            self.assertEqual(result["admissions"]["warm/0/baseline"]["browserTimingAnomalies"][0]["totalMs"], -4.5)
            preflight["benchmarkRequestId"] = "unmatched"
            write_json(path / "results.json", report)
            self.assert_withheld(replicated_compare.analyze(directory))
            preflight["benchmarkRequestId"] = "options"
            next(record for record in report["network"] if record["category"] == "invoke")["totalMs"] = -4.5
            write_json(path / "results.json", report)
            self.assert_withheld(replicated_compare.analyze(directory))

    def test_all_modes_validate_with_multiple_warm_bursts(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            fixture(directory)
            result = replicated_compare.analyze(directory)
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["metrics"]["warm"]["Click to history DOM: trial median"]["n"], 2)

    def test_fast_failed_or_incomplete_samples_withhold_every_effect(self):
        for mutation in ("failed", "missing", "missing-worker", "accounting", "cleanup"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=("action",))
                path = directory / suite["trials"][0]["directory"]
                report = json.loads((path / "results.json").read_text())
                if mutation == "failed":
                    report["samples"][0].update(status="error", clickToHistoryDomMs=1)
                elif mutation == "missing":
                    report["samples"].pop()
                elif mutation == "missing-worker":
                    report["samples"][1]["worker"] = 0
                elif mutation == "accounting":
                    suite["trials"][0]["accountingBeforeStop"]["runIds"].pop()
                else:
                    suite["trials"][0]["cleanup"]["ownedServiceProcessesExited"] = False
                write_json(path / "results.json", report)
                self.write_trial(directory, suite)
                self.assert_withheld(replicated_compare.analyze(directory))

    def test_changed_evidence_and_binary_provenance_are_rejected(self):
        for mutation in ("ledger", "inventory", "binary", "frontend", "saved-trial"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=("warm",))
                path = directory / suite["trials"][0]["directory"]
                if mutation == "ledger":
                    with (path / "admissions.jsonl").open("a") as file:
                        file.write("\n")
                elif mutation == "inventory":
                    with (path / "processes.json").open("a") as file:
                        file.write("\n")
                elif mutation == "binary":
                    suite["binaries"]["baseline"]["sha256"] = "d" * 64
                elif mutation == "frontend":
                    suite["frontends"]["baseline"]["buildId"] = "different"
                else:
                    suite["trials"][0]["firstMeasuredClickUnixMs"] += 1
                write_json(directory / "replicated-results.json", suite)
                self.assert_withheld(replicated_compare.analyze(directory))

    def test_proxy_rejections_and_unmatched_counters_cannot_disappear(self):
        for field in ("rejected", "admitted", "active"):
            with self.subTest(field=field), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=("warm",))
                suite["trials"][0]["proxyStatus"]["pools"]["runtime"][field] += 1
                self.write_trial(directory, suite)
                self.assert_withheld(replicated_compare.analyze(directory))

    def test_worker_seven_boundary_proof_is_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, replicas=8, modes=("action",))
            path = directory / suite["trials"][0]["directory"] / "results.json"
            report = json.loads(path.read_text())
            report["coldBoundary"]["workers"][7]["requestsCheckedThroughClick"] = False
            write_json(path, report)
            result = replicated_compare.analyze(directory)
            self.assert_withheld(result)
            self.assertTrue(any("worker 7" in issue for issue in result["issues"]))

    def test_pre_click_requests_are_recomputed_for_each_worker(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory, pairs=1, modes=("action",))
            path = directory / suite["trials"][0]["directory"] / "results.json"
            report = json.loads(path.read_text())
            report["network"].append({"worker": 1, "navigation": 0, "phase": "cold-boundary", "category": "info",
                                      "status": 200, "totalMs": 1, "startedAtUnixMs": 1000.5})
            write_json(path, report)
            result = replicated_compare.analyze(directory)
            self.assert_withheld(result)
            self.assertTrue(any("worker 1: API traffic" in issue for issue in result["issues"]))

    def test_missing_replica_warmup_and_cold_reuse_are_rejected(self):
        for mode in ("warm", "action"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=(mode,))
                trial = suite["trials"][0]
                path = directory / trial["directory"]
                records = [json.loads(line) for line in (path / "admissions.jsonl").read_text().splitlines()]
                if mode == "warm":
                    for record in records:
                        if record["pool"] == "runtime" and record["replica"] == 1 and record["queuedAtUnixMs"] < 1000:
                            record["pathCategory"] = "health"
                else:
                    for record in records:
                        if record["pool"] == "runtime" and record["generation"] == 1:
                            record["replica"] = 0
                (path / "admissions.jsonl").write_text("".join(json.dumps(record) + "\n" for record in records))
                trial["evidence"]["admissionsSha256"] = replicated_compare.sha256(path / "admissions.jsonl")
                self.write_trial(directory, suite)
                self.assert_withheld(replicated_compare.analyze(directory))

    def test_order_and_topology_changes_are_not_code_speedups(self):
        for mutation in ("order", "topology", "database-pool"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                suite = fixture(directory, pairs=1, modes=("warm",))
                if mutation == "order":
                    suite["trials"].reverse()
                    write_json(directory / "replicated-results.json", suite)
                else:
                    path = directory / suite["trials"][1]["directory"] / "run-manifest.json"
                    manifest = json.loads(path.read_text())
                    if mutation == "topology":
                        manifest["execution_topology"] = None
                    else:
                        manifest["database_pool"]["maxConnections"] = 8
                    write_json(path, manifest)
                self.assert_withheld(replicated_compare.analyze(directory))


if __name__ == "__main__":
    unittest.main()
