"""Development compatibility commands preserve the explicit trust boundary."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[2]

class DevelopmentTest(unittest.TestCase):
    def test_setup_and_bootstrap_reject_implicit_trusted_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "configuration"
            env = {"PATH": os.environ["PATH"]}
            for script, args in [("dev.sh", ["setup"]), ("dev-bootstrap.sh", ["--output-dir", str(output)])]:
                result = subprocess.run(["bash", str(BASE / "scripts" / script), *args], env=env, capture_output=True, text=True)
                with self.subTest(script=script):
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("K3D_EXECUTION_MODE=trusted_shared", result.stderr)
                    self.assertFalse(output.exists())

    def test_bootstrap_uses_private_generator_and_preserves_existing_files(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "configuration"
            env = {"PATH": os.environ["PATH"], "K3D_EXECUTION_MODE": "trusted_shared"}
            command = ["bash", str(BASE / "scripts/dev-bootstrap.sh"), "--namespace", "dev-test", "--release", "dev-test", "--output-dir", str(output)]
            subprocess.run(command, env=env, check=True, capture_output=True)
            original = (output / "secrets.yaml").read_bytes()
            values = json.loads((output / "values-generated.yaml").read_text())
            self.assertEqual(values["fullnameOverride"], "dev-test")
            self.assertNotIn("isolationMode", values["execution"])
            retry = subprocess.run(command, env=env, capture_output=True)
            self.assertNotEqual(retry.returncode, 0)
            self.assertEqual((output / "secrets.yaml").read_bytes(), original)

    def test_deploy_rejects_a_different_prerequisite_target(self):
        with tempfile.TemporaryDirectory() as directory:
            values = Path(directory) / "values.json"
            values.write_text("{}")
            env = {"PATH": os.environ["PATH"], "VALUES": str(values)}
            for arguments in [["--kube-context", "different"], ["--kubeconfig=/other/config"], ["--namespace", "different"]]:
                result = subprocess.run(["bash", str(BASE / "scripts/deploy.sh"), *arguments], env=env, capture_output=True, text=True)
                with self.subTest(arguments=arguments):
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("KUBECONFIG", result.stderr)

    def test_status_forwards_namespace_without_trusted_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            kubectl = directory / "kubectl"
            kubectl.write_text('#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:]))\n')
            kubectl.chmod(0o700)
            env = {"PATH": str(directory) + os.pathsep + os.environ["PATH"], "K8S_NAMESPACE": "dev-test"}
            result = subprocess.run(["bash", str(BASE / "scripts/dev.sh"), "status"], env=env, check=True, capture_output=True, text=True)
            self.assertEqual(json.loads(result.stdout), ["get", "pods,jobs", "-n", "dev-test"])

if __name__ == "__main__":
    unittest.main()
