"""Exercise setup and image-value generation without Docker or a cluster."""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[2]

class SetupTest(unittest.TestCase):
    def test_private_files_and_existing_secrets_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "config"
            command = ["python3", str(BASE / "scripts/setup-config.py"), "--output-dir", str(output)]
            env = {"PATH": os.environ["PATH"]}
            subprocess.run(command, env=env, check=True, capture_output=True)
            contents = (output / "secrets.yaml").read_bytes()
            self.assertEqual(stat.S_IMODE((output / "secrets.yaml").stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
            result = subprocess.run(command, env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((output / "secrets.yaml").read_bytes(), contents)

    def test_built_digests_and_partial_build_merge(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            docker = directory / "docker"
            docker.write_text("""#!/usr/bin/env python3
import json,os,sys
with open(os.environ['BUILD_LOG'],'a') as out: out.write(json.dumps(sys.argv[1:])+'\\n')
if sys.argv[1:3]==['image','inspect']:
    repository=sys.argv[3].rsplit(':',1)[0]
    print(json.dumps([repository+'@sha256:'+'a'*64]))
""")
            docker.chmod(0o700)
            output = directory / "values.json"
            output.write_text(json.dumps({"web": {"image": {"repository": "preserved/web", "tag": "existing"}}}))
            log = directory / "build.log"
            env = {"PATH": str(directory) + os.pathsep + os.environ["PATH"], "REGISTRY": "registry.example.com/team", "TAG": "review", "PUSH": "true", "COMPONENTS": "api executor execution-manager", "IMAGE_VALUES_FILE": str(output), "BUILD_LOG": str(log), "FLOW_LIKE_CONFIG": "flow-like.kubernetes.config.json"}
            script = ["bash", str(BASE / "scripts/build-images.sh")]
            subprocess.run(script, env=env, check=True, capture_output=True)
            values = json.loads(output.read_text())
            self.assertEqual(values["web"]["image"]["tag"], "existing")
            self.assertEqual(values["executionManager"]["image"]["digest"], "sha256:" + "a" * 64)
            self.assertEqual(values["executionManager"]["sandbox"]["image"], "registry.example.com/team/flow-like-k8s-executor@sha256:" + "a" * 64)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            self.assertTrue(any("FLOW_LIKE_CONFIG=flow-like.kubernetes.config.json" in call for call in calls))
            env.update({"PUSH": "false", "COMPONENTS": "executor"})
            subprocess.run(script, env=env, check=True, capture_output=True)
            values = json.loads(output.read_text())
            self.assertEqual(values["executionManager"]["sandbox"]["image"], "")
            self.assertIn("digest", values["executionManager"]["image"])

if __name__ == "__main__":
    unittest.main()
