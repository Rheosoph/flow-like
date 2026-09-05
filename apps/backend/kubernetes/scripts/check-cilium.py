#!/usr/bin/env python3
"""Read the installed Cilium configuration before deploying isolated execution."""
import json
import os
import subprocess


def get(*args):
    result = subprocess.run(["kubectl", "get", *args, "-o", "json"], capture_output=True, text=True)
    if result.returncode:
        raise ValueError("Unable to read Cilium prerequisites: " + " ".join(args))
    return json.loads(result.stdout)


def validate(config, daemonset):
    data = config.get("data", {})
    if data.get("enable-policy", "default") not in ("default", "always"):
        raise ValueError("Cilium policy enforcement must be default or always")
    for key in ("enable-k8s-networkpolicy", "enable-cilium-network-policy"):
        if data.get(key, "true") != "true":
            raise ValueError(f"Cilium {key} must be enabled")
    if data.get("allow-localhost") != "policy":
        raise ValueError("Configure Cilium allow-localhost=policy and complete its rollout before deploying tenant execution")
    status = daemonset.get("status", {})
    desired = status.get("desiredNumberScheduled", 0)
    if not desired or status.get("numberReady") != desired or status.get("updatedNumberScheduled") != desired or status.get("observedGeneration", 0) < daemonset["metadata"]["generation"]:
        raise ValueError("Cilium DaemonSet must finish rolling out and be ready on every scheduled node")


def main():
    namespace = os.environ.get("CILIUM_NAMESPACE", "kube-system")
    get("crd", "ciliumnetworkpolicies.cilium.io")
    config = get("configmap", os.environ.get("CILIUM_CONFIGMAP", "cilium-config"), "-n", namespace)
    daemonset = get("daemonset", os.environ.get("CILIUM_DAEMONSET", "cilium"), "-n", namespace)
    validate(config, daemonset)
    print("Cilium CRD, policy configuration and DaemonSet readiness verified. Each execution slot also checks denied connections before admission.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, json.JSONDecodeError) as error:
        raise SystemExit(str(error)) from None
