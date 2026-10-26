import io
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import replica_services
import run
import stack


def pool_status(**changes):
    return {"pools": {
        pool: {"active": 0, "queued": 0, "failed": False, "failures": 0,
               "rejected": 0, "quarantined": 0, **changes}
        for pool in replica_services.POOL_PORTS
    }}


class ReplicaServicesBoundaries(unittest.TestCase):
    def services(self, replicas=1):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        state = Path(temporary.name)
        (state / "platform.json").write_text('{"features":{"telemetry":false}}')
        binary = state / "widget_action_stack"
        binary.write_bytes(b"frozen test binary")
        args = SimpleNamespace(
            state=state, output=state / "trial", replicas=replicas, workers=7,
            executor_slots=8, thread_stack_mb=32, trace=False, binary=binary,
            profile="release-fast", timeout=1, web_mode="production",
        )
        with patch.object(stack, "environment", return_value={"MAX_CONCURRENT_EXECUTIONS": "8"}):
            services = replica_services.ReplicaServices(args)
        services.binary_sha256 = run.file_hash(binary)
        services.persist_inventory = Mock()
        return services

    def owned_processes(self, services, generation=0, first_pid=100):
        created = []
        for pool in replica_services.POOL_PORTS:
            for index in range(services.args.replicas):
                pid = first_pid + len(created)
                record = {"name": f"{pool}-{index}", "pool": pool,
                          "replica": f"{pool}-{index}", "generation": generation,
                          "pid": pid, "started": f"lifetime-{pid}"}
                process = Mock(pid=pid)
                process.poll.return_value = None
                services.records.append(record)
                services.processes.append(process)
                services.inventory.append(dict(record))
                services.timings[record["name"]] = 2.5
                created.append(record)
        return created

    def restart_mocks(self, services, after_launch=None, final_status=None):
        timeline = []
        services.wait_idle = Mock(side_effect=lambda: timeline.append("idle"))

        def control(path, payload=None):
            timeline.append((path, payload))
            return final_status or pool_status()

        def stopped(record, process):
            timeline.append(("stop", record.get("pool", record["name"])))
            process.poll.return_value = 0

        def launched():
            timeline.append("launch")
            self.owned_processes(services, generation=services.generation, first_pid=200)
            if after_launch:
                after_launch()

        services.control = Mock(side_effect=control)
        services.launch_backend = Mock(side_effect=launched)
        stop_patch = patch.object(run, "stop_record", side_effect=stopped)
        identity_patch = patch.object(run, "process_started", side_effect=lambda pid: f"lifetime-{pid}")
        self.addCleanup(stop_patch.stop)
        self.addCleanup(identity_patch.stop)
        stop_patch.start()
        identity_patch.start()
        return timeline

    def test_fixed_independent_runtime_and_callback_configuration(self):
        services = self.services(replicas=2)
        self.assertEqual(services.env["BENCH_RUNTIME_MODEL"], "current-thread")
        self.assertEqual(services.env["BENCH_WORKER_THREADS"], "1")
        self.assertEqual(services.env["MAX_CONCURRENT_EXECUTIONS"], "1")
        for key in ("API_URL", "API_BASE_URL", "CALLBACK_BASE_URL"):
            self.assertEqual(services.env[key], "http://127.0.0.1:8442")
        self.assertEqual(services.env["EXECUTOR_URL"], "http://127.0.0.1:8441")
        self.assertEqual(services.env["DATABASE_POOL_MAX_CONNECTIONS"], "4")
        self.assertEqual(services.env["DATABASE_POOL_MIN_CONNECTIONS"], "0")
        for invalid in (True, 0, 65, "8"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                replica_services.topology_for(invalid)

    def test_idle_wait_requires_live_processes_and_empty_healthy_pools(self):
        services = self.services()
        self.owned_processes(services)
        services.control = Mock(side_effect=[pool_status(active=1), pool_status()])
        with patch.object(replica_services.time, "sleep") as sleep:
            services.wait_idle()
        self.assertEqual(services.control.call_count, 2)
        sleep.assert_called_once_with(.05)
        services.control.reset_mock(side_effect=True)
        services.processes[0].poll.return_value = 1
        with self.assertRaisesRegex(ValueError, "exited"):
            services.wait_idle()
        services.control.assert_not_called()

    def test_idle_wait_rejects_each_admission_failure_signal(self):
        for field, value in (("failed", True), ("failures", 1), ("rejected", 1), ("quarantined", 1)):
            with self.subTest(field=field):
                services = self.services()
                services.control = Mock(return_value=pool_status(**{field: value}))
                with self.assertRaisesRegex(ValueError, "admission failed"):
                    services.wait_idle()

    def test_restart_pauses_before_stopping_and_resumes_only_after_fresh_processes(self):
        services = self.services()
        before = self.owned_processes(services)
        unrelated = {"name": "replica-proxy", "pid": 99, "started": "lifetime-99"}
        services.records.insert(0, unrelated)
        proxy_process = Mock(pid=99)
        proxy_process.poll.return_value = None
        services.processes.insert(0, proxy_process)
        timeline = self.restart_mocks(services)
        result = services.restart_backend()
        self.assertEqual(timeline[:2], ["idle", ("/pause", {})])
        self.assertEqual(timeline[2:5], [("stop", "runtime"), ("stop", "api_callback"), ("stop", "api_frontend")])
        self.assertEqual(timeline[-3:], ["launch", ("/status", None), ("/generation", {"generation": 1})])
        self.assertEqual(result["processes_before"], before)
        self.assertEqual(len(result["processes_after"]), 3)
        self.assertTrue(all(record["generation"] == 1 for record in result["processes_after"]))
        self.assertEqual(services.records[0], unrelated)
        self.assertEqual(proxy_process.poll(), None)
        self.assertEqual(len(services.inventory), 6)

    def test_changed_process_identity_fails_after_pause_without_stopping_or_resuming(self):
        services = self.services()
        self.owned_processes(services)
        self.restart_mocks(services)
        with patch.object(run, "process_started", return_value="another-process"), \
                patch.object(run, "stop_record") as stop:
            with self.assertRaisesRegex(ValueError, "identity changed"):
                services.restart_backend()
        stop.assert_not_called()
        services.launch_backend.assert_not_called()
        self.assertEqual(services.control.call_args_list[0].args, ("/pause", {}))
        self.assertEqual(services.control.call_count, 1)

    def test_changed_binary_fails_closed_after_old_processes_are_stopped(self):
        services = self.services()
        self.owned_processes(services)
        timeline = self.restart_mocks(services)
        with patch.object(run, "file_hash", return_value="changed"):
            with self.assertRaisesRegex(ValueError, "binary changed"):
                services.restart_backend()
        self.assertEqual(len([entry for entry in timeline if isinstance(entry, tuple) and entry[0] == "stop"]), 3)
        self.assertFalse(services.records)
        services.launch_backend.assert_not_called()
        self.assertEqual(services.control.call_count, 1)

    def test_partial_launch_failure_keeps_proxy_paused_and_retains_cleanup_inventory(self):
        services = self.services()
        self.owned_processes(services)
        handle = Mock()
        services.handles.append(handle)

        def partial_launch():
            services.records.append({"name": "api_frontend-0", "pid": 200, "started": "lifetime-200"})
            process = Mock(pid=200)
            process.poll.return_value = None
            services.processes.append(process)
            raise ValueError("new runtime failed readiness")

        self.restart_mocks(services)
        services.launch_backend.side_effect = partial_launch
        with self.assertRaisesRegex(ValueError, "failed readiness"):
            services.restart_backend()
        self.assertEqual(services.control.call_count, 1)
        self.assertEqual(len(services.records), 1)
        services.close()
        self.assertTrue(all(process.poll() == 0 for process in services.processes))
        handle.close.assert_called_once()

    def test_unexpected_traffic_during_restart_does_not_resume_generation(self):
        services = self.services()
        self.owned_processes(services)
        self.restart_mocks(services, final_status=pool_status(rejected=1))
        with self.assertRaisesRegex(ValueError, "Unexpected traffic"):
            services.restart_backend()
        self.assertEqual([call.args[0] for call in services.control.call_args_list], ["/pause", "/status"])

    def test_each_backend_uses_direct_health_and_checks_actual_runtime_before_inventory(self):
        services = self.services(replicas=2)
        launched = []

        def launch(name, command, url):
            launched.append((name, command, url, dict(services.env)))
            services.records.append({"name": name, "pid": 100 + len(launched), "started": "owned"})

        services.launch = Mock(side_effect=launch)
        with patch.object(replica_services.urllib.request, "urlopen",
                          side_effect=lambda *_args, **_kwargs: io.StringIO('{"runtimeModel":"current-thread"}')) as probe:
            services.launch_backend()
        self.assertEqual(len(launched), 6)
        self.assertEqual(len(services.inventory), 6)
        self.assertEqual(probe.call_count, 6)
        ports = set()
        for name, command, health, env in launched:
            pool = name.rsplit("-", 1)[0]
            self.assertEqual(command[-1], "runtime" if pool == "runtime" else "api")
            expected_health = "/health" if pool == "runtime" else "/api/v1/health"
            self.assertTrue(health.endswith(expected_health))
            self.assertNotIn(":844", health)
            ports.add(env["API_PORT"])
        self.assertEqual(len(ports), 6)
        self.assertTrue(all(record["runtimeModel"] == "current-thread" for record in services.inventory))

    def test_wrong_runtime_is_not_recorded_as_a_valid_replica(self):
        services = self.services()

        def launch(name, _command, _url):
            services.records.append({"name": name, "pid": 1, "started": "owned"})

        services.launch = Mock(side_effect=launch)
        with patch.object(replica_services.urllib.request, "urlopen",
                          side_effect=lambda *_args, **_kwargs: io.StringIO('{"runtimeModel":"multi-thread"}')):
            with self.assertRaisesRegex(ValueError, "required current-thread"):
                services.launch_backend()
        self.assertFalse(services.inventory)
        self.assertEqual(len(services.records), 1)
        services.persist_inventory.assert_not_called()

    def test_trace_cleanup_drains_replica_api_once_before_stop_without_delaying_normal_runs(self):
        for tracing in (False, True):
            for role in ("api_frontend", "api_callback", "runtime"):
                with self.subTest(tracing=tracing, role=role):
                    services = self.services(replicas=2)
                    services.args.trace = tracing
                    self.owned_processes(services)
                    services.records = [record for record in services.records if record["pool"] == role]
                    owned_pids = {record["pid"] for record in services.records}
                    services.processes = [process for process in services.processes if process.pid in owned_pids]
                    timeline = []

                    def stop(record, process):
                        timeline.append(("stop", record["replica"]))
                        process.poll.return_value = 0

                    with patch.object(run.time, "sleep", side_effect=lambda seconds: timeline.append(("drain", seconds))) as sleep, \
                            patch.object(run, "stop_record", side_effect=stop):
                        services.close()
                    expected_drain = tracing and role != "runtime"
                    if expected_drain:
                        sleep.assert_called_once_with(5.1)
                        self.assertEqual(timeline[0], ("drain", 5.1))
                    else:
                        sleep.assert_not_called()
                    self.assertEqual(len([event for event in timeline if event[0] == "stop"]), 2)
                    self.assertTrue(all(process.poll() == 0 for process in services.processes))


if __name__ == "__main__":
    unittest.main()
