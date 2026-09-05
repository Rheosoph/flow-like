#!/usr/bin/env bash
# Local development uses the explicitly selected trusted workflow mode.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CLUSTER_NAME="${K3D_CLUSTER_NAME:-flow-like}"
NAMESPACE="${K8S_NAMESPACE:-flow-like}"
case "${1:-setup}" in
  delete) exec k3d cluster delete "$CLUSTER_NAME" ;;
  status) exec kubectl get pods,jobs -n "$NAMESPACE" ;;
  setup|rebuild) ;;
  *) echo 'Usage: k3d-setup.sh [setup|rebuild|status|delete]' >&2; exit 1 ;;
esac
if [[ "${K3D_EXECUTION_MODE:-}" != trusted_shared ]]; then
  echo 'k3d does not install gVisor. For trusted local workflows, set K3D_EXECUTION_MODE=trusted_shared explicitly.' >&2
  echo 'For tenant isolation, install runsc and an enforcing CNI on Linux execution nodes, then use setup-config.sh and deploy.sh.' >&2
  exit 1
fi
: "${S3_PUBLIC_ENDPOINT:?Set an S3 gateway origin reachable from both your browser and cluster; see README.md}"
for command in docker k3d kubectl helm python3 openssl; do
  command -v "$command" >/dev/null || { echo "Required command missing: $command" >&2; exit 1; }
done
if ! k3d cluster list -o json | python3 -c 'import json,sys; name=sys.argv[1]; sys.exit(0 if any(x["name"]==name for x in json.load(sys.stdin)) else 1)' "$CLUSTER_NAME"; then
  k3d cluster create "$CLUSTER_NAME" --agents 2 --port '8080:80@loadbalancer'
fi
kubectl wait --for=condition=Ready nodes --all --timeout=120s
if [[ ! -f "$BACKEND_DIR/.generated/values-generated.yaml" ]]; then
  "$SCRIPT_DIR/setup-config.sh" --namespace "$NAMESPACE"
fi
# Images are imported directly. The isolated deployment requires a real registry.
TAG="${TAG:-dev}" PUSH=false "$SCRIPT_DIR/build-images.sh"
python3 - "$BACKEND_DIR/.generated/values-images.yaml" <<'PY' | while IFS= read -r image; do
import json,sys
values=json.load(open(sys.argv[1]))
def walk(value):
    if isinstance(value,dict):
        if 'repository' in value and 'tag' in value:
            print(value['repository']+':'+value['tag'])
        for nested in value.values(): walk(nested)
walk(values)
PY
  k3d image import "$image" -c "$CLUSTER_NAME"
done
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f "$BACKEND_DIR/.generated/secrets.yaml"
"$SCRIPT_DIR/deploy.sh" --set execution.isolationMode=trusted_shared --set execution.asyncBackend=http \
  --set ingress.enabled=true --set ingress.className=traefik \
  --set ingress.hosts[0].host='' \
  --set networkPolicy.ingressNamespaceSelector.matchLabels.kubernetes\\.io/metadata\\.name=kube-system
