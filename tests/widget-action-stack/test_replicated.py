import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import cold
import replicated
import run
from replica_services import topology_for
from test_topology import fixture as admission_fixture
from test_topology import request as admission_request


class ReplicatedConfiguration(unittest.TestCase):
    def test_defaults_and_custom_warm_counts_keep_whole_bursts(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "config.json"
            source.write_text(json.dumps({"appUrl": "http://localhost:3440", "storageState": "auth.json"}))
            result = replicated.trial_config(source, "warm", 8)
            self.assertEqual((result["samples"], result["warmup"], result["concurrency"]), (104, 3, 8))
            self.assertEqual(result["storageState"], str((source.parent / "auth.json").resolve()))
            source.write_text(json.dumps({"samples": 16, "warmup": 2, "output": "old-output", "concurrency": 1}))
            result = replicated.trial_config(source, "warm", 8)
            self.assertEqual((result["samples"], result["warmup"], result["concurrency"]), (16, 2, 8))
            self.assertNotIn("output", result)
            self.assertEqual(replicated.trial_config(source, "warm", 8, samples=24)["samples"], 24)
            for samples in (0, 7, 9, True, 8.5):
                with self.assertRaisesRegex(ValueError, "multiple"):
                    replicated.trial_config(source, "warm", 8, samples=samples)
            with self.assertRaisesRegex(ValueError, "warmup"):
                replicated.trial_config(source, "warm", 8, warmup=0)

    def test_cold_modes_have_exactly_one_action_per_owned_browser(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "config.json"
            source.write_text(json.dumps({"samples": 100, "warmup": 8, "navigations": 4,
                                          "requireTerminalSuccess": False, "headed": True}))
            for mode in ("navigation", "action"):
                result = replicated.trial_config(source, mode, 8)
                self.assertEqual((result["samples"], result["warmup"], result["navigations"], result["concurrency"]), (8, 0, 1, 8))
                self.assertTrue(result["requireTerminalSuccess"])
                self.assertFalse(result["headed"])
            source.write_text(json.dumps({"cdpUrl": "http://localhost:9222"}))
            with self.assertRaisesRegex(ValueError, "own their browsers"):
                replicated.trial_config(source, "action", 8)
            for count in (0, 65, True):
                with self.assertRaisesRegex(ValueError, "between 1 and 64"):
                    replicated.trial_config(source, "action", count)


class ReplicatedOrchestration(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.state = self.directory / "state"
        self.state.mkdir()
        binary = self.directory / "binary"
        binary.write_bytes(b"local fixture binary")
        self.args = argparse.Namespace(
            state=self.state, output=self.directory, replicas=2, timeout=1,
            baseline_web_state=None, candidate_web_state=None,
            baseline_web_source_root=None, candidate_web_source_root=None,
        )
        self.suite = {"trials": [], "binaries": {"baseline": {"path": str(binary), "sha256": run.file_hash(binary)}}}
        self.events, self.created = [], []
        self.restart_error = None
        self.accounting_error = None

    def factory(self, settings):
        test = self

        class FakeServices:
            def __init__(self):
                self.closed = False
                process = Mock()
                process.poll.side_effect = lambda: 0 if self.closed else None
                self.processes = [process]
                self.inventory = self.generation(0)
                self.topology = topology_for(2)
                self.timings = {"api_frontend-0": 1, "runtime-0": 2}
                self.ready_at_unix_ms = 50
                self.admission_log = settings.output / "admissions.jsonl"
                self.admission_log.write_text("")

            def generation(self, generation):
                return [{"name": f"{pool}-{replica}", "pool": pool, "replica": f"{pool}-{replica}",
                         "generation": generation, "pid": 100 + generation * 10 + index * 2 + replica,
                         "started": "owned", "binarySha256": test.suite["binaries"]["baseline"]["sha256"],
                         "workerThreads": 1, "runtimeModel": "current-thread"}
                        for index, pool in enumerate(("api_frontend", "api_callback", "runtime")) for replica in range(2)]

            def start(self):
                test.events.append("start")
                (test.state / "processes.json").write_text("{}")

            def wait_idle(self):
                test.events.append("idle")
                return {"paused": False, "closing": False, "pools": {pool: {
                    "active": 0, "queued": 0, "quarantined": 0, "failed": False,
                    "failures": 0, "rejected": 0,
                } for pool in ("api_frontend", "api_callback", "runtime")}}

            def restart_backend(self):
                test.events.append("restart")
                if test.restart_error:
                    raise test.restart_error
                before, after = copy.deepcopy(self.inventory), self.generation(1)
                self.inventory.extend(after)
                self.ready_at_unix_ms = 300
                return {"processes_before": before, "processes_after": after, "readyAtUnixMs": 300}

            def close(self):
                test.events.append("close")
                self.closed = True
                (test.state / "processes.json").unlink(missing_ok=True)

        service = FakeServices()
        self.created.append(service)
        return service

    def browser(self, settings, phase_handler=None, driver_output=None):
        self.events.append("browser")

        def entry(worker, name):
            return {"worker": worker, "index": worker, "status": "ok", "browserTimeOriginMs": 100 + worker,
                    "clickAtNavigationMs": 400, "executionSuccessVerified": True,
                    "execution": [{"runId": f"{name}-{worker}", "success": True}]}

        report = {"navigations": [entry(worker, "bootstrap") for worker in range(2)], "samples": []}
        path = settings.output / "results.json"
        path.write_text(json.dumps(report))
        if phase_handler:
            phase_handler()
            report["coldBoundary"] = {
                "status": "confirmed", "workerCount": 2, "bootstrapExecutionSuccessVerified": True,
                "controllerRestartConfirmed": True, "documentPreserved": True,
                "requestsCheckedThroughClick": True, "unexpectedApiRequests": 0, "unexpectedExecutionRequests": 0,
                "pendingRequestsBeforeBoundary": {"status": "confirmed", "workerCount": 2, "pendingApiRequests": 0},
                "workers": [{"worker": worker, "bootstrapExecutionSuccessVerified": True,
                             "documentPreserved": True, "requestsCheckedThroughClick": True,
                             "unexpectedApiRequests": 0} for worker in range(2)],
            }
        report["samples"] = [entry(worker, "action") for worker in range(2)]
        path.write_text(json.dumps(report))
        (settings.output / "run-manifest.json").write_text(json.dumps({
            "binary_sha256": self.suite["binaries"]["baseline"]["sha256"],
        }))

    def accounting(self, state, directory, timeout):
        identifiers = cold.completed_run_ids(directory)
        self.events.append("accounting-stop" if any(value.startswith("action") for value in identifiers) else "accounting-restart")
        if self.accounting_error:
            raise self.accounting_error
        return {"status": "confirmed", "runIds": identifiers, "polls": 1, "durationMs": 1}

    def trial(self, mode="action"):
        with patch.object(replicated, "create_services", side_effect=self.factory), \
                patch.object(replicated.run, "benchmark", side_effect=self.browser), \
                patch.object(replicated.cold, "wait_for_accounting", side_effect=self.accounting), \
                patch.object(replicated.compare, "assessment", return_value={"issues": []}), \
                patch.object(replicated, "admission_evidence", return_value=[]):
            return replicated.run_trial(self.args, self.suite, mode, 0, 0, "baseline", {})

    def test_restart_and_shutdown_wait_for_accounting_and_preserve_all_generations(self):
        record = self.trial()
        self.assertEqual(record["status"], "passed")
        self.assertEqual(self.events, ["start", "browser", "accounting-restart", "idle", "restart", "accounting-stop", "idle", "close"])
        self.assertEqual(len(record["accountingBeforeRestart"]["runIds"]), 2)
        self.assertEqual(len(record["accountingBeforeStop"]["runIds"]), 4)
        self.assertEqual(record["firstMeasuredClickUnixMs"], 500)
        self.assertEqual(record["firstNavigationUnixMs"], 100)
        self.assertEqual(record["readyAtUnixMs"], 300)
        destination = self.directory / record["directory"]
        self.assertEqual(len(json.loads((destination / "processes.json").read_text())), 12)
        self.assertEqual(len(record["initialStartup"]["processes"]), 6)
        self.assertEqual(record["evidence"]["processesSha256"], run.file_hash(destination / "processes.json"))
        self.assertEqual(json.loads((destination / "replicated-trial.json").read_text()), record)
        self.assertEqual(json.loads((self.directory / "replicated-results.json").read_text())["trials"], [record])
        self.assertTrue(record["cleanup"]["ownedServiceProcessesExited"])
        self.assertFalse((self.state / "processes.json").exists())

    def test_failed_restart_keeps_error_and_partial_results_and_closes_services(self):
        self.restart_error = ValueError("one runtime failed readiness")
        record = self.trial()
        self.assertEqual(record["status"], "failed")
        self.assertEqual(record["wrapperExitCode"], 1)
        self.assertTrue(any("failed readiness" in issue for issue in record["issues"]))
        self.assertEqual(self.events[-1], "close")
        self.assertNotIn("accounting-stop", self.events)
        self.assertTrue((self.directory / record["directory"] / "results.json").exists())
        self.assertEqual(record["cleanup"]["status"], "ok")

    def test_failed_final_accounting_never_marks_a_navigation_trial_passed(self):
        self.accounting_error = ValueError("completion did not persist")
        record = self.trial("navigation")
        self.assertEqual(record["status"], "failed")
        self.assertFalse(record["restartRequestedAfterBootstrap"])
        self.assertNotIn("restart", self.events)
        self.assertEqual(self.events[-1], "close")
        self.assertTrue(any("completion did not persist" in issue for issue in record["issues"]))

    def test_binary_change_fails_before_starting_services(self):
        Path(self.suite["binaries"]["baseline"]["path"]).write_bytes(b"replaced binary")
        record = self.trial()
        self.assertEqual(record["status"], "failed")
        self.assertEqual(self.created, [])
        self.assertTrue(any("binary changed" in issue for issue in record["issues"]))


class ReplicatedAdmissionEvidence(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.rows, self.processes, arrangement = admission_fixture(2)
        self.record = {"mode": "warm", "topology": arrangement, "firstMeasuredClickUnixMs": 30,
                       "proxyStatus": {"paused": False, "closing": False, "pools": {
                           pool: {"active": 0, "queued": 0, "quarantined": 0, "failed": False,
                                  "failures": 0, "rejected": 0}
                           for pool in ("api_frontend", "api_callback", "runtime")}}}
        self.report = {"network": [
            {"category": "invoke", "benchmarkRequestId": row["requestId"], "method": "POST", "status": 200,
             "execution": {"success": True, "terminalObserved": True, "runId": "run-" + row["requestId"]}}
            for row in self.rows if row["event"] == "released" and row["pool"] == "api_frontend"]}
        self.manifest = {}
        self.record["accountingBeforeStop"] = {"status": "confirmed", "runIds": [
            entry["execution"]["runId"] for entry in self.report["network"]]}

    def assess(self):
        (self.directory / "admissions.jsonl").write_text("".join(json.dumps(row) + "\n" for row in self.rows))
        (self.directory / "results.json").write_text(json.dumps(self.report))
        (self.directory / "run-manifest.json").write_text(json.dumps(self.manifest))
        return replicated.admission_evidence(self.directory, self.record, self.processes)

    def test_warmup_requires_completed_sse_work_on_every_runtime_replica(self):
        self.assertEqual(self.assess(), [])
        self.assertEqual(self.record["warmupCoverage"]["status"], "confirmed")
        self.assertEqual(len(self.record["warmupCoverage"]["requestIds"]), 6)
        for row in self.rows:
            if row["pool"] == "runtime" and row["replica"] == 1:
                row["pathCategory"] = "executor_stream"
        issues = self.assess()
        self.assertTrue(any("coverage differs" in issue for issue in issues))
        self.assertEqual(self.record["warmupCoverage"]["status"], "failed")

    def test_rejections_missing_from_the_admission_ledger_still_fail(self):
        self.record["proxyStatus"]["pools"]["runtime"]["rejected"] = 1
        issues = self.assess()
        self.assertTrue(any("pending, or failed" in issue for issue in issues))

    def test_terminal_disconnect_requires_correlated_browser_success_and_accounting(self):
        self.rows[1]["downstreamDisconnected"] = True
        self.assertEqual(self.assess(), [])
        self.report["network"][0]["execution"]["terminalObserved"] = False
        issues = self.assess()
        self.assertTrue(any("terminal success" in issue for issue in issues))
        self.report["network"][0]["execution"]["terminalObserved"] = True
        self.record["accountingBeforeStop"]["runIds"] = []
        self.assertTrue(any("persisted accounting" in issue for issue in self.assess()))

    def test_optional_telemetry_404_requires_exact_ledger_route_and_is_not_warmup(self):
        optional = admission_request("optional-config", self.processes[0], queued=21, admitted=22,
                                     released=24, category="frontend_telemetry_config")
        for row in optional:
            row["method"] = "GET"
        optional[1]["status"] = 404
        self.rows.extend(optional)
        self.report["network"].append({"category": "other_api", "phase": "bootstrap", "method": "GET",
                                       "status": 404, "benchmarkRequestId": "optional-config",
                                       "startedAtUnixMs": 21, "totalMs": 3})
        self.manifest["allowedOptionalResponses"] = replicated.topology.OPTIONAL_RESPONSE_POLICY
        self.assertEqual(self.assess(), [])
        self.assertNotIn("optional-config", self.record["warmupCoverage"]["requestIds"])
        for row in optional:
            row["pathCategory"] = "other"
        self.assertTrue(self.assess())


if __name__ == "__main__":
    unittest.main()
