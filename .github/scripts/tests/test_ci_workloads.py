"""Exercise CI routing and its final gate without starting build or publish jobs."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "ci_workloads.py"
SPEC = importlib.util.spec_from_file_location("ci_workloads", SCRIPT)
ci = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ci)


def flags(*enabled):
    return {name: name in enabled for name in ci.CHANGE_FLAGS}


def selected(**overrides):
    result = dict.fromkeys(ci.WORKLOADS, False)
    result.update(overrides)
    return result


def results(planned):
    return {
        "changes": {
            "result": "success",
            "outputs": {key: str(value).lower() for key, value in planned.items()},
        },
        **{key: {"result": "success" if value else "skipped"} for key, value in planned.items()},
    }


class RoutingTests(unittest.TestCase):
    def test_dev_rust_pull_request_runs_core_checks(self):
        actual = ci.plan("pull_request", "refs/pull/12/merge", "dev", {}, flags("rust"))
        self.assertEqual(actual, selected(tests=True, lint=True, boundaries=True))

    def test_checks_keep_independent_input_filters(self):
        cases = {
            "bun": selected(tests=True),
            "ui": selected(tests=True),
            "azure_migrations": selected(tests=True),
            "dsql": selected(lint=True),
            "browser": selected(browser=True),
            "compiler": selected(compiler=True),
        }
        for changed, expected in cases.items():
            with self.subTest(changed=changed):
                self.assertEqual(ci.plan("push", "refs/heads/dev", "", {}, flags(changed)), expected)

    def test_non_dev_pull_requests_keep_container_checks_only(self):
        actual = ci.plan("pull_request", "refs/pull/12/merge", "main", {}, flags(*ci.CHANGE_FLAGS))
        self.assertEqual(actual, selected(containers=True, compiler=True))

    def test_pull_request_container_checks_need_changed_inputs_on_any_target(self):
        for base in ("dev", "main", "alpha", "release/next"):
            with self.subTest(base=base):
                self.assertEqual(ci.plan("pull_request", "refs/pull/12/merge", base, {}, flags()), selected())
                self.assertEqual(
                    ci.plan("pull_request", "refs/pull/12/merge", base, {}, flags("containers")),
                    selected(containers=True, compiler=True),
                )

    def test_dev_push_does_not_publish_for_unrelated_changes(self):
        self.assertEqual(ci.plan("push", "refs/heads/dev", "", {}, flags()), selected())
        self.assertEqual(
            ci.plan("push", "refs/heads/dev", "", {}, flags("containers")),
            selected(containers=True, compiler=True),
        )

    def test_release_pushes_always_publish_containers(self):
        for ref in ("refs/heads/main", "refs/tags/v1.2.3", "refs/tags/beta-v1.2.3"):
            with self.subTest(ref=ref):
                self.assertEqual(
                    ci.plan("push", ref, "", {}, flags()), selected(containers=True, compiler=True),
                )

    def test_alpha_push_shares_one_compiler_prerequisite(self):
        self.assertEqual(
            ci.plan("push", "refs/heads/alpha", "", {}, flags()),
            selected(containers=True, compiler=True, alpha=True),
        )

    def test_feature_pushes_and_other_tags_do_not_publish(self):
        for ref in ("refs/heads/feature/test", "refs/heads/version", "refs/tags/release-1", "refs/tags/alpha-v1"):
            with self.subTest(ref=ref):
                self.assertEqual(ci.plan("push", ref, "", {}, flags(*ci.CHANGE_FLAGS)), selected())

    def test_manual_checks_never_publish_even_on_release_refs(self):
        for ref in ("refs/heads/dev", "refs/heads/main", "refs/heads/alpha", "refs/tags/v1", "refs/heads/feature/test"):
            for inputs in ({}, {"scope": "checks", "cloud": "all"}):
                with self.subTest(ref=ref, inputs=inputs):
                    self.assertEqual(
                        ci.plan("workflow_dispatch", ref, "", inputs, flags(*ci.CHANGE_FLAGS)),
                        selected(tests=True, lint=True, boundaries=True, browser=True, compiler=True),
                    )

    def test_manual_containers_only_publish_on_release_refs(self):
        for ref in ("refs/heads/dev", "refs/heads/main", "refs/heads/alpha", "refs/tags/v1", "refs/tags/beta-v1"):
            for cloud in ci.CLOUDS:
                with self.subTest(ref=ref, cloud=cloud):
                    self.assertEqual(
                        ci.plan("workflow_dispatch", ref, "", {"scope": "containers", "cloud": cloud}, flags()),
                        selected(containers=True, compiler=True),
                    )
        for ref in ("refs/heads/feature/test", "refs/heads/main-fix", "refs/tags/other"):
            with self.subTest(ref=ref), self.assertRaisesRegex(ValueError, "publication requires"):
                ci.plan("workflow_dispatch", ref, "", {"scope": "containers"}, flags())

    def test_unavailable_diff_forces_all_checks_without_bypassing_event_rules(self):
        all_changed = flags(*ci.CHANGE_FLAGS)
        self.assertEqual(
            ci.plan("push", "refs/heads/dev", "", {}, all_changed),
            selected(tests=True, lint=True, boundaries=True, browser=True, containers=True, compiler=True),
        )
        self.assertFalse(ci.plan("workflow_dispatch", "refs/heads/alpha", "", {}, all_changed)["alpha"])

    def test_action_output_strings_are_parsed_as_booleans(self):
        changed = {key: str(value).lower() for key, value in flags("ui").items()}
        self.assertEqual(ci.plan("push", "refs/heads/dev", "", {}, changed), selected(tests=True))

    def test_invalid_or_incomplete_plans_fail_closed(self):
        for value in (None, 0, 1, "", "TRUE", "no", [], {}):
            with self.subTest(value=value), self.assertRaises(ValueError):
                ci.plan("push", "refs/heads/dev", "", {}, {**flags(), "rust": value})
        with self.assertRaisesRegex(ValueError, "Missing change flags"):
            ci.plan("push", "refs/heads/dev", "", {}, {})
        with self.assertRaisesRegex(ValueError, "Unknown CI scope"):
            ci.plan("workflow_dispatch", "refs/heads/dev", "", {"scope": "all"}, flags())
        with self.assertRaisesRegex(ValueError, "Unknown container cloud"):
            ci.plan("workflow_dispatch", "refs/heads/dev", "", {"scope": "containers", "cloud": "unknown"}, flags())
        with self.assertRaisesRegex(ValueError, "Unsupported CI event"):
            ci.plan("workflow_run", "refs/heads/dev", "", {}, flags())


class GateTests(unittest.TestCase):
    def test_success_requires_selected_jobs_and_accepts_intended_skips(self):
        for planned in (selected(), selected(tests=True, browser=True), dict.fromkeys(ci.WORKLOADS, True)):
            with self.subTest(planned=planned):
                self.assertEqual(ci.check_results(results(planned)), [])

    def test_selected_jobs_fail_closed_for_every_non_success_result(self):
        for job in ci.WORKLOADS:
            for actual in ("failure", "cancelled", "skipped", "pending", "neutral", "", None):
                with self.subTest(job=job, actual=actual):
                    needs = results(dict.fromkeys(ci.WORKLOADS, True))
                    needs[job]["result"] = actual
                    self.assertTrue(ci.check_results(needs))

    def test_unselected_jobs_must_be_skipped(self):
        for job in ci.WORKLOADS:
            for actual in ("success", "failure", "cancelled", "pending", None):
                with self.subTest(job=job, actual=actual):
                    needs = results(selected())
                    needs[job]["result"] = actual
                    self.assertTrue(ci.check_results(needs))

    def test_failed_or_missing_detection_cannot_pass_with_all_jobs_skipped(self):
        for result in ("failure", "cancelled", "skipped", None):
            with self.subTest(result=result):
                needs = results(selected())
                needs["changes"]["result"] = result
                self.assertTrue(ci.check_results(needs))
        needs = results(selected())
        del needs["changes"]
        self.assertTrue(ci.check_results(needs))

    def test_missing_jobs_outputs_and_invalid_flags_fail_closed(self):
        for job in ci.WORKLOADS:
            with self.subTest(job=job):
                needs = results(selected())
                del needs[job]
                self.assertTrue(ci.check_results(needs))
                needs = results(selected())
                del needs["changes"]["outputs"][job]
                self.assertTrue(ci.check_results(needs))
                needs = results(selected())
                needs["changes"]["outputs"][job] = ""
                self.assertTrue(ci.check_results(needs))
        needs = results(selected())
        needs["changes"].pop("outputs")
        self.assertTrue(ci.check_results(needs))

    def test_malformed_or_unplanned_jobs_do_not_pass(self):
        for needs in (None, [], "success", {**results(selected()), "extra": {"result": "failure"}}):
            with self.subTest(needs=needs):
                self.assertTrue(ci.check_results(needs))
        for job in ("changes", *ci.WORKLOADS):
            for malformed in (None, [], "success"):
                with self.subTest(job=job, malformed=malformed):
                    needs = results(selected())
                    needs[job] = malformed
                    self.assertTrue(ci.check_results(needs))


class CommandTests(unittest.TestCase):
    def test_default_command_appends_github_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            output.write_text("previous=value\n")
            env = {
                "GITHUB_EVENT_NAME": "push", "GITHUB_REF": "refs/heads/dev",
                "CHANGES_JSON": json.dumps(flags("ui")), "GITHUB_OUTPUT": str(output),
            }
            with patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ci.main([]), 0)
            expected = "previous=value\n" + "".join(
                f"{key}={str(value).lower()}\n" for key, value in selected(tests=True).items()
            )
            self.assertEqual(output.read_text(), expected)

    def test_gate_command_reports_failure_and_success(self):
        needs = results(selected(tests=True))
        for actual, expected_code in (("success", 0), ("skipped", 1)):
            needs["tests"]["result"] = actual
            captured = io.StringIO()
            with patch.dict(os.environ, {"NEEDS_JSON": json.dumps(needs)}, clear=True), contextlib.redirect_stdout(captured):
                self.assertEqual(ci.main(["gate"]), expected_code)
            self.assertEqual("::error::" in captured.getvalue(), bool(expected_code))

    def test_invalid_environment_or_json_returns_failure(self):
        for command, env in (([], {}), ([], {"CHANGES_JSON": "{"}), (["gate"], {"NEEDS_JSON": "{"})):
            with self.subTest(command=command, env=env), patch.dict(os.environ, env, clear=True), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(ci.main(command), 1)


if __name__ == "__main__":
    unittest.main()
