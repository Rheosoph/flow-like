import copy
import json
from pathlib import Path
import tempfile
import unittest

import cold_compare
from test_compare import bundle


def fixture(directory):
    suite = {"schemaVersion": 1, "mode": "paired-process-cold", "status": "passed",
             "configuration": {"trials": 2, "modes": ["navigation", "action"], "readinessPollMs": 20, "workerThreads": 1, "executorSlots": 4, "trace": False},
             "binaries": {name: {"sha256": name} for name in ("baseline", "candidate")}, "trials": []}
    pid = 100
    for mode in ("navigation", "action"):
        for pair in range(2):
            order = ("baseline", "candidate") if pair == 0 else ("candidate", "baseline")
            for position, variant in enumerate(order):
                report = bundle()
                report["report"]["configuration"]["samples"] = 1
                report["report"]["samples"] = report["report"]["samples"][:1]
                report["report"]["navigations"][0]["bootstrapValidatedAtNavigationMs"] = 250
                report["manifest"]["binary_sha256"] = variant
                report["manifest"]["readiness_poll_ms"] = 20
                report["manifest"]["trace_requested"] = False
                report["report"]["network"].extend(
                    {"phase": phase, "category": category, "status": 200, "totalMs": 10}
                    for phase in ("bootstrap", "action") for category in ("prerun", "invoke"))
                report["report"]["navigations"][0]["execution"] = [{"runId": "bootstrap-run", "success": True}]
                report["report"]["samples"][0]["execution"] = [{"runId": "action-run", "success": True}]
                relative = f"{mode}/{pair}-{variant}"
                path = directory / relative
                path.mkdir(parents=True)
                processes = [{"name": name, "pid": pid + index, "started": "time"}
                             for index, name in enumerate(("api", "runtime"))]
                pid += 2
                trial = {"mode": mode, "pair": pair, "variant": variant, "position": position,
                         "directory": relative, "status": "passed", "wrapperExitCode": 0, "issues": [],
                         "cleanup": {"status": "ok", "ownedServiceProcessesExited": True, "processInventoryRemoved": True},
                         "initialStartup": {"processes": processes, "measurements_ms": {"api": 10, "runtime": 20}},
                         "restart": None, "accountingBeforeStop": {"status": "confirmed", "runIds": ["bootstrap-run", "action-run"], "polls": 1, "durationMs": 5}}
                if mode == "action":
                    trial["restartRequestedAfterBootstrap"] = True
                    trial["accountingBeforeRestart"] = {"status": "confirmed", "runIds": ["bootstrap-run"], "polls": 1, "durationMs": 5}
                    trial["restart"] = {"processes_before": copy.deepcopy(processes),
                                        "processes_after": [{"name": name, "pid": pid + index, "started": "time"}
                                                            for index, name in enumerate(("api", "runtime"))],
                                        "measurements_ms": {"api": 10, "runtime": 20}}
                    pid += 2
                    report["report"]["coldBoundary"] = {"mode": "cold-action", "status": "confirmed",
                        "bootstrapExecutionSuccessVerified": True, "controllerRestartConfirmed": True,
                        "documentPreserved": True, "unexpectedExecutionRequests": 0, "unexpectedApiRequests": 0, "requestsCheckedThroughClick": True}
                for name, key in (("results.json", "report"), ("run-manifest.json", "manifest")):
                    (path / name).write_text(json.dumps(report[key]))
                suite["trials"].append(trial)
    return suite


class ColdComparison(unittest.TestCase):
    def test_frontend_versions_can_differ_between_variants_but_not_between_trials(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            suite["frontends"] = {name: {"sourceSha256": name, "buildId": name + "-build"}
                                  for name in ("baseline", "candidate")}
            for trial in suite["trials"]:
                path = directory / trial["directory"] / "results.json"
                report = json.loads(path.read_text())
                report["web"]["build"] = {"source": {"sha256": trial["variant"]},
                                          "buildId": trial["variant"] + "-build"}
                path.write_text(json.dumps(report))
            self.assertEqual(self.assess(directory, suite)["issues"], [])
            path = directory / suite["trials"][-1]["directory"] / "results.json"
            report = json.loads(path.read_text())
            report["web"]["build"]["buildId"] = "unexpected-rebuild"
            path.write_text(json.dumps(report))
            issues = self.assess(directory, suite)["issues"]
            self.assertTrue(any("frontend provenance mismatch" in issue for issue in issues))
            self.assertTrue(any("frontend changed within" in issue for issue in issues))

    def assess(self, directory, suite):
        (directory / "cold-results.json").write_text(json.dumps(suite))
        return cold_compare.analyze(directory)

    def test_paired_difference_interval_uses_pairs(self):
        result = cold_compare.paired_summary([(100, 90), (200, 190), (400, 390), (600, 590)])
        self.assertEqual(result["pairedMedianDeltaMs"], -10)
        self.assertEqual(result["pairedMedianDelta95CiMs"], [-10, -10])
        self.assertIsNone(cold_compare.paired_summary([(1, 2)])["pairedMedianDelta95CiMs"])

    def test_complete_suite_requires_fresh_processes_and_semantic_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            result = self.assess(directory, suite)
            self.assertEqual(result["issues"], [])
            self.assertEqual(result["metrics"]["action"]["Click to history DOM"]["n"], 2)
            for mode in result["metrics"]:
                self.assertIn("Executor process readiness", result["metrics"][mode])

    def test_process_reuse_cannot_look_like_a_cold_speedup(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            trial = next(item for item in suite["trials"] if item["mode"] == "action")
            trial["restart"]["processes_after"] = trial["restart"]["processes_before"]
            result = self.assess(directory, suite)
            self.assertTrue(any("reused" in issue for issue in result["issues"]))
            self.assertIn("withheld", cold_compare.render(result))

    def test_failed_and_incomplete_trials_withhold_differences(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            suite["trials"].pop()
            suite["status"] = "failed"
            result = self.assess(directory, suite)
            self.assertIn("suite is incomplete", result["issues"])
            self.assertIn("withheld", cold_compare.render(result))

    def test_network_priming_and_unverified_execution_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            trial = next(item for item in suite["trials"] if item["mode"] == "action")
            path = directory / trial["directory"] / "results.json"
            report = json.loads(path.read_text())
            report["coldBoundary"]["unexpectedApiRequests"] = 1
            report["samples"][0]["executionSuccessVerified"] = False
            path.write_text(json.dumps(report))
            result = self.assess(directory, suite)
            self.assertTrue(any("unverified cold-action" in issue for issue in result["issues"]))
            self.assertTrue(any("terminal success" in issue for issue in result["issues"]))

    def test_missing_http_evidence_on_both_variants_cannot_disappear(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            for trial in suite["trials"]:
                path = directory / trial["directory"] / "results.json"
                report = json.loads(path.read_text())
                report["network"] = []
                path.write_text(json.dumps(report))
            result = self.assess(directory, suite)
            self.assertTrue(any("HTTP timing evidence" in issue for issue in result["issues"]))

    def test_navigation_rejects_a_restart_after_bootstrap(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            path = directory / suite["trials"][0]["directory"] / "results.json"
            report = json.loads(path.read_text())
            report["coldBoundary"] = {"status": "confirmed"}
            path.write_text(json.dumps(report))
            result = self.assess(directory, suite)
            self.assertTrue(any("navigation report unexpectedly" in issue for issue in result["issues"]))

    def test_accounting_of_another_run_cannot_validate_shutdown(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            suite["trials"][0]["accountingBeforeStop"]["runIds"] = ["different-run"]
            result = self.assess(directory, suite)
            self.assertTrue(any("accountingBeforeStop" in issue for issue in result["issues"]))

    def test_environment_mismatch_and_wrong_binary_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            suite = fixture(directory)
            path = directory / suite["trials"][1]["directory"] / "run-manifest.json"
            manifest = json.loads(path.read_text())
            manifest["worker_threads"] = 2
            manifest["binary_sha256"] = "unexpected"
            path.write_text(json.dumps(manifest))
            result = self.assess(directory, suite)
            self.assertTrue(any("worker_threads" in issue for issue in result["issues"]))
            self.assertTrue(any("binary provenance" in issue for issue in result["issues"]))


if __name__ == "__main__":
    unittest.main()
