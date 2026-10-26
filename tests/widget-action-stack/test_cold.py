import contextlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

import cold
import run


class ColdOrchestration(unittest.TestCase):
    def test_frontend_identity_comes_from_the_selected_production_stage(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            workspace = state / "source"
            stage = workspace / "apps" / "web"
            stage.mkdir(parents=True)
            (state / "web-stage.json").write_text(json.dumps({"stage": str(stage)}))
            (workspace / "build.json").write_text(json.dumps({
                "source": {"sha256": "frozen-source"}, "buildId": "build-one"}))
            self.assertEqual(cold.frontend_identity(state), {
                "sourceSha256": "frozen-source", "buildId": "build-one"})
            (workspace / "build.json").write_text(json.dumps({
                "source": {"sha256": "frozen-source"}, "buildId": ""}))
            with self.assertRaisesRegex(ValueError, "incomplete build provenance"):
                cold.frontend_identity(state)

    def controlled(self, program, handler, concurrency=1):
        children = []
        popen = subprocess.Popen

        def launch(*args, **kwargs):
            child = popen(*args, **kwargs)
            children.append(child)
            return child

        try:
            with patch.object(run.subprocess, "Popen", side_effect=launch), \
                    patch.object(run, "process_started", return_value="owned-test-child"), \
                    contextlib.redirect_stdout(io.StringIO()):
                run.controlled_browser([sys.executable, "-c", program], dict(os.environ), handler, 5, concurrency)
        finally:
            for child in children:
                # Assert before emergency cleanup so a leaked child fails the test.
                exited = child.poll() is not None
                if not exited:
                    child.kill()
                    child.wait(timeout=5)
                self.assertTrue(exited, "The phase controller left its child running")

    def test_restart_finishes_before_browser_receives_continue(self):
        with tempfile.TemporaryDirectory() as temporary:
            ready = Path(temporary) / "backend-ready"
            calls = []

            def restart():
                calls.append("restarted")
                ready.write_text("healthy")

            program = "\n".join([
                "import json, sys",
                "from pathlib import Path",
                "print(json.dumps({'type':'benchmark_cold_action_ready','worker':0,'navigation':0}), flush=True)",
                "reply = json.loads(sys.stdin.readline())",
                "assert reply == {'type':'benchmark_cold_action_continue'}",
                f"assert Path({str(ready)!r}).read_text() == 'healthy'",
            ])
            self.controlled(program, restart)
            self.assertEqual(calls, ["restarted"])

    def test_multiworker_boundary_waits_for_the_complete_worker_set(self):
        handler = Mock()
        program = "\n".join([
            "import json, sys",
            "print(json.dumps({'type':'benchmark_cold_action_ready','workers':[0,1,2,3,4,5,6,7],'navigation':0}), flush=True)",
            "assert json.loads(sys.stdin.readline()) == {'type':'benchmark_cold_action_continue'}",
        ])
        self.controlled(program, handler, concurrency=8)
        handler.assert_called_once_with()

    def test_malformed_worker_sets_cannot_restart_backends(self):
        for workers in ([0], [0, 0], [False, 1], [0, 1, 2], None, "01"):
            with self.subTest(workers=workers), self.assertRaisesRegex(ValueError, "worker boundary"):
                run.validate_cold_marker({"navigation": 0, "workers": workers}, 2)
        for message in ({"navigation": False, "worker": 0},
                        {"navigation": 0, "worker": 0, "workers": [0]}):
            with self.assertRaisesRegex(ValueError, "worker boundary"):
                run.validate_cold_marker(message, 1)

    def test_restart_failure_sends_abort_and_preserves_original_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            received = Path(temporary) / "reply.json"
            program = "\n".join([
                "import json, sys",
                "from pathlib import Path",
                "print(json.dumps({'type':'benchmark_cold_action_ready','worker':0,'navigation':0}), flush=True)",
                "reply = json.loads(sys.stdin.readline())",
                f"Path({str(received)!r}).write_text(json.dumps(reply))",
            ])

            def fail_restart():
                raise ValueError("owned runtime did not become healthy")

            with self.assertRaisesRegex(ValueError, "owned runtime did not become healthy"):
                self.controlled(program, fail_restart)
            self.assertEqual(json.loads(received.read_text())["type"], "benchmark_cold_action_abort")

    def test_successful_driver_exit_without_boundary_is_rejected(self):
        handler = Mock()
        with self.assertRaisesRegex(ValueError, "without its restart boundary"):
            self.controlled("print('normal driver output')", handler)
        handler.assert_not_called()

    def test_repeated_boundary_does_not_restart_backend_twice(self):
        handler = Mock()
        program = "\n".join([
            "import json, sys",
            "marker = {'type':'benchmark_cold_action_ready','worker':0,'navigation':0}",
            "print(json.dumps(marker), flush=True)",
            "assert json.loads(sys.stdin.readline())['type'] == 'benchmark_cold_action_continue'",
            "print(json.dumps(marker), flush=True)",
            "assert json.loads(sys.stdin.readline())['type'] == 'benchmark_cold_action_abort'",
        ])
        with self.assertRaisesRegex(ValueError, "repeated cold-action phase marker"):
            self.controlled(program, handler)
        handler.assert_called_once()

    def test_owned_exited_child_is_reaped_without_waiting_for_ps_to_disappear(self):
        child = Mock(pid=12345)
        child.poll.return_value = 0
        record = {"pid": child.pid, "started": "same lifetime"}
        # ps can still see an exited, unreaped child; poll must reap it before
        # the grace period expires and the process group receives SIGKILL.
        with patch.object(run, "process_started", return_value="same lifetime"), \
                patch.object(run.os, "getpgid", return_value=child.pid), \
                patch.object(run.os, "killpg") as kill, \
                patch.object(run.time, "monotonic", side_effect=[0, 1, 6]), \
                patch.object(run.time, "sleep") as sleep:
            run.stop_record(record, child)
        child.poll.assert_called_once()
        kill.assert_called_once_with(child.pid, signal.SIGINT)
        sleep.assert_not_called()

    def test_cold_configuration_has_one_first_action_and_balanced_order(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.json"
            path.write_text(json.dumps({"samples": 100, "warmup": 3, "navigations": 2,
                                       "concurrency": 8, "requireTerminalSuccess": False}))
            config = cold.trial_config(path)
            self.assertEqual({key: config[key] for key in ("samples", "warmup", "navigations", "concurrency")},
                             {"samples": 1, "warmup": 0, "navigations": 1, "concurrency": 1})
            self.assertIs(config["requireTerminalSuccess"], True)
            self.assertEqual(cold.trial_order(0), ("baseline", "candidate"))
            self.assertEqual(cold.trial_order(1), ("candidate", "baseline"))
            path.write_text(json.dumps({"cdpUrl": "http://localhost:9222"}))
            with self.assertRaisesRegex(ValueError, "own their browser"):
                cold.trial_config(path)

    def test_accounting_drain_waits_for_every_verified_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)

            def entry(identifier):
                return {"executionSuccessVerified": True,
                        "execution": [{"success": True, "runId": identifier}]}

            (directory / "results.json").write_text(json.dumps({
                "navigations": [entry("onload-run")], "samples": [entry("action-run")]}))
            with patch.object(cold.stack, "compose", side_effect=[Mock(stdout="1\n"), Mock(stdout="2\n")]) as query, \
                    patch.object(cold.time, "sleep"):
                result = cold.wait_for_accounting(directory, directory, 5)
            self.assertEqual(result["status"], "confirmed")
            self.assertEqual(result["runIds"], ["action-run", "onload-run"])
            self.assertEqual(query.call_count, 2)

    def test_accounting_drain_rejects_unexpected_ids_before_querying(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "results.json").write_text(json.dumps({"navigations": [{
                "executionSuccessVerified": True,
                "execution": [{"success": True, "runId": "unexpected'identifier"}]}]}))
            with patch.object(cold.stack, "compose") as query:
                with self.assertRaisesRegex(ValueError, "Unexpected benchmark run identifier"):
                    cold.wait_for_accounting(directory, directory, 5)
            query.assert_not_called()


if __name__ == "__main__":
    unittest.main()
