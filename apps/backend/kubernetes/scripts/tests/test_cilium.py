"""Fail closed on disabled policy or incomplete Cilium rollouts."""
import copy
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[1] / "check-cilium.py"
spec = importlib.util.spec_from_file_location("check_cilium", path)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

class CiliumTest(unittest.TestCase):
    def setUp(self):
        self.config = {"data": {"allow-localhost": "policy", "enable-policy": "default", "enable-k8s-networkpolicy": "true", "enable-cilium-network-policy": "true"}}
        self.daemonset = {"metadata": {"generation": 2}, "status": {"observedGeneration": 2, "desiredNumberScheduled": 3, "numberReady": 3, "updatedNumberScheduled": 3}}

    def test_ready_enforcing_configuration(self):
        check.validate(self.config, self.daemonset)

    def test_disabled_policy_and_host_bypass_rejected(self):
        for key, value in [("allow-localhost", "always"), ("enable-policy", "never"), ("enable-k8s-networkpolicy", "false"), ("enable-cilium-network-policy", "false")]:
            config = copy.deepcopy(self.config)
            config["data"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                check.validate(config, self.daemonset)

    def test_unready_or_stale_daemonset_rejected(self):
        for key, value in [("observedGeneration", 1), ("numberReady", 2), ("updatedNumberScheduled", 2), ("desiredNumberScheduled", 0)]:
            daemonset = copy.deepcopy(self.daemonset)
            daemonset["status"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                check.validate(self.config, daemonset)

if __name__ == "__main__":
    unittest.main()
