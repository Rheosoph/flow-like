import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import compare
import run


def bundle():
    return {
        "report": {
            "schemaVersion": 1, "mode": "actual-app", "status": "passed",
            "app": {"origin": "http://127.0.0.1:3440", "path": "/use/personen"},
            "configuration": {"samples": 2, "navigations": 1, "warmup": 0, "concurrency": 1,
                              "requireTerminalSuccess": True, "requireActionTrace": True},
            "machine": {"cpu": "fixture", "memoryBytes": 1024}, "browser": "Chrome/fixture",
            "web": {"mode": "production-static-export", "build": {"source": {"sha256": "web-a"}}},
            "navigations": [{"status": "ok", "cardsDomMs": 200, "executionSuccessVerified": True}],
            "network": [{"category": "bootstrap", "phase": "bootstrap", "status": 200, "totalMs": 50}],
            "samples": [{"navigation": 0, "index": index, "status": "ok", "warmup": False,
                         "executionSuccessVerified": True, "clickToHistoryDomMs": latency,
                         "clickToHistoryFrameOpportunityMs": latency + 20, "clickToSettledMs": latency + 100}
                        for index, latency in enumerate((100, 200))],
        },
        "manifest": {
            "requested_profile": "release-fast", "binary_profile_hint": "release-fast", "binary_sha256": "binary-a",
            "web_mode": "production", "worker_threads": 1, "executor_slots": 4,
            "thread_stack_mb": 32,
            "compute_accounting_memory_mb": 2048, "enforced_memory_limit_mb": None,
            "backend_trace_sample_rate": 0.05, "transport": "http", "action_config_sha256": "actions",
            "telemetry_enabled": False, "telemetry_sink": "db", "rust_log": "info",
            "fixture": {"app_id": "app", "graph_source_sha256": "source", "graph_fixture_sha256": "fixture",
                        "fixture_sha256": "full-fragmented", "database": {
                            "profile": "production", "layout": "fragmented", "records_sha256": "records", "rows": 47,
                            "data_storage_version": "2.2", "pylance": "8.0.0", "schema_sha256": "schema"}},
        },
    }


class ReportComparison(unittest.TestCase):
    def test_layout_and_implementation_changes_keep_identical_workload(self):
        first = bundle()
        second = copy.deepcopy(first)
        second["manifest"]["fixture"]["database"]["layout"] = "compact"
        second["manifest"]["fixture"]["fixture_sha256"] = "full-compact"
        second["manifest"]["binary_sha256"] = "binary-b"
        second["report"]["web"]["build"]["source"]["sha256"] = "web-b"
        self.assertEqual(compare.mismatches(first, second), [])
        output, invalid = compare.render(first, second, [])
        self.assertFalse(invalid)
        self.assertIn("layout=compact", output)
        self.assertIn("Frontend source: changed", output)
        self.assertEqual(compare.assessment(first)["metrics"]["History DOM (click)"]["p95"], 195)

    def test_environment_or_logical_workload_change_is_rejected(self):
        first = bundle()
        paths = ["report.configuration.concurrency", "report.machine.cpu", "report.browser",
                 "manifest.worker_threads", "manifest.executor_slots", "manifest.thread_stack_mb", "manifest.binary_profile_hint",
                 "manifest.fixture.graph_source_sha256", "manifest.fixture.graph_fixture_sha256",
                 "manifest.fixture.database.records_sha256", "manifest.action_config_sha256"]
        paths.extend(["manifest.telemetry_enabled", "manifest.telemetry_sink", "manifest.rust_log",
                      "manifest.backend_trace_sample_rate", "manifest.fixture.database.data_storage_version",
                      "manifest.fixture.database.pylance", "manifest.fixture.database.schema_sha256"])
        for path in paths:
            with self.subTest(path=path):
                second = copy.deepcopy(first)
                target = second
                keys = path.split(".")
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = "changed"
                self.assertTrue(compare.mismatches(first, second))

    def test_failed_fast_candidate_cannot_claim_speedup(self):
        first, second = bundle(), bundle()
        second["report"]["status"] = "failed"
        second["report"]["samples"][0]["clickToHistoryDomMs"] = 1
        second["report"]["samples"][1]["status"] = "error"
        output, invalid = compare.render(first, second, [])
        self.assertTrue(invalid)
        self.assertIn("1/2 successful, 1 failed", output)
        self.assertIn("Percent changes withheld", output)
        self.assertNotIn("-99", output)

    def test_forced_cleanup_keeps_timings_but_cannot_claim_full_run_success(self):
        for cleanup in ({"status": "forced-after-measurement", "wrapperExitCode": 1,
                         "ownedChromeProcessesExited": True},
                        {"status": "success", "wrapperExitCode": 1},
                        {"status": "forced-after-measurement", "wrapperExitCode": 0}):
            with self.subTest(cleanup=cleanup):
                first, second = bundle(), bundle()
                second["report"]["cleanup"] = cleanup
                output, invalid = compare.render(first, second, [])
                self.assertTrue(invalid)
                self.assertIn("driver cleanup did not complete successfully", output)
                self.assertIn("Percent changes withheld", output)
                self.assertEqual(compare.assessment(second)["metrics"]["History DOM (click)"]["n"], 2)

    def test_missing_sample_or_terminal_evidence_is_not_success(self):
        first = bundle()
        first["report"]["samples"].pop()
        first["report"]["samples"][0].pop("executionSuccessVerified")
        result = compare.assessment(first)
        self.assertEqual(result["successful"], 0)
        self.assertEqual(result["missing"], 1)
        self.assertTrue(result["issues"])

    def test_bootstrap_retries_do_not_hide_failed_requests(self):
        first = bundle()
        first["report"]["network"].append({
            "category": "bootstrap", "phase": "bootstrap", "networkError": "connection reset", "totalMs": 5000,
        })
        result = compare.assessment(first)
        self.assertEqual(result["bootstrap_failed"], 1)
        self.assertEqual(result["metrics"]["Bootstrap HTTP"]["n"], 1)
        self.assertTrue(result["issues"])

    def test_unverified_bootstrap_or_reset_prevents_false_speedup(self):
        first = bundle()
        first["report"]["navigations"][0].pop("executionSuccessVerified")
        result = compare.assessment(first)
        self.assertEqual(result["navigations"], 0)
        self.assertTrue(result["issues"])
        first = bundle()
        first["report"]["configuration"]["resetBetweenActions"] = True
        first["report"]["resets"] = [{"status": "ok", "executionSuccessVerified": True}]
        self.assertFalse(compare.assessment(first)["issues"])
        first["report"]["resets"][0]["executionSuccessVerified"] = False
        self.assertEqual(compare.assessment(first)["reset_failed"], 1)
        first["report"]["resets"] = []
        self.assertIn("recorded 0 of 1 planned resets", compare.assessment(first)["issues"])

    def test_missing_provenance_in_both_reports_is_not_a_match(self):
        first, second = bundle(), bundle()
        for item in (first, second):
            item["manifest"].pop("binary_sha256")
        self.assertTrue(compare.mismatches(first, second))

    def test_mismatch_requires_explicit_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            directories = [Path(temporary) / name for name in ("first", "second")]
            for index, directory in enumerate(directories):
                directory.mkdir()
                value = bundle()
                value["report"]["machine"]["cpu"] = str(index)
                (directory / "results.json").write_text(json.dumps(value["report"]))
                (directory / "run-manifest.json").write_text(json.dumps(value["manifest"]))
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(compare.main(list(map(str, directories))), 2)
                self.assertEqual(compare.main([*map(str, directories), "--allow-mismatch"]), 0)

    def test_failed_browser_run_keeps_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            config = state / "config.json"
            config.write_text('{"samples":2,"storageState":"private-session"}')
            (state / "run-manifest.json").write_text(json.dumps(bundle()["manifest"]))
            (state / "startup-timings.json").write_text('{"measurements_ms":{"api":123}}')
            args = SimpleNamespace(state=state, config=config, output=state / "failed-results")
            with patch.object(run.stack, "environment", return_value={}), patch.object(
                    run.stack, "run", side_effect=subprocess.CalledProcessError(1, "browser")):
                with self.assertRaises(subprocess.CalledProcessError):
                    run.benchmark(args)
            preserved = json.loads((args.output / "run-manifest.json").read_text())
            self.assertEqual(preserved["binary_sha256"], "binary-a")
            self.assertEqual(len(preserved["action_config_sha256"]), 64)
            self.assertTrue((args.output / "startup-timings.json").is_file())

    def test_existing_services_cannot_silently_enable_trace_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            config = state / "config.json"
            config.write_text('{"samples":2}')
            (state / "run-manifest.json").write_text(json.dumps(bundle()["manifest"]))
            args = SimpleNamespace(state=state, config=config, output=state / "results", trace=True)
            with patch.object(run.stack, "run") as execute:
                with self.assertRaisesRegex(ValueError, "serve --trace"):
                    run.benchmark(args)
                execute.assert_not_called()

    def test_trace_configuration_is_private_and_leaves_default_platform_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            original = '{"features":{"telemetry":false,"flow_hosting":true}}\n'
            (state / "platform.json").write_text(original)
            env = {"FLOW_LIKE_CONFIG_FILE": str(state / "platform.json"), "RUST_LOG": "info"}
            default_env = dict(env)
            directory, normal = run.service_diagnostics(state, default_env)
            self.assertEqual(default_env, env)
            self.assertFalse(normal["telemetry_enabled"])
            self.assertEqual(normal["backend_trace_sample_rate"], 0.05)
            self.assertEqual(directory, state / "logs")
            traced_env = dict(env)
            directory, traced = run.service_diagnostics(state, traced_env, trace=True)
            self.assertEqual((state / "platform.json").read_text(), original)
            self.assertTrue(traced["telemetry_enabled"])
            self.assertEqual(traced["telemetry_sink"], "log")
            self.assertEqual(traced["backend_trace_sample_rate"], 1)
            self.assertEqual(traced["rust_log"], "info")
            self.assertNotEqual(traced_env["FLOW_LIKE_CONFIG_FILE"], env["FLOW_LIKE_CONFIG_FILE"])
            private = Path(traced_env["FLOW_LIKE_CONFIG_FILE"])
            self.assertEqual(private.stat().st_mode & 0o777, 0o600)
            self.assertTrue(directory.is_relative_to(state / "traces"))
            first, second = bundle(), bundle()
            second["manifest"].update(traced)
            self.assertTrue(compare.mismatches(first, second))


if __name__ == "__main__":
    unittest.main()
