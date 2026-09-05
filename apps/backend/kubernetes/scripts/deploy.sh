#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
NAMESPACE="${K8S_NAMESPACE:-${NAMESPACE:-flow-like}}"
RELEASE="${RELEASE:-flow-like}"
VALUES="${VALUES:-$BACKEND_DIR/.generated/values-generated.yaml}"
IMAGE_VALUES="${IMAGE_VALUES_FILE:-$BACKEND_DIR/.generated/values-images.yaml}"
[[ -f "$VALUES" ]] || { echo "Missing $VALUES; run scripts/setup-config.py first" >&2; exit 1; }
# Helm and kubectl prerequisite checks must use the same target and identity.
# KUBECONFIG and its selected context are inherited by both commands.
for argument in "$@"; do
  case "$argument" in
    --kube*|--namespace|--namespace=*|-n|-n?*)
      echo 'Select the cluster and identity through KUBECONFIG, and the namespace through K8S_NAMESPACE; per-command overrides would bypass the prerequisite target checks.' >&2
      exit 1 ;;
  esac
done
extra=()
[[ ! -f "$IMAGE_VALUES" ]] || extra+=(-f "$IMAGE_VALUES")
# Render first so missing secrets, digests and incompatible modes fail before writes.
helm lint "$BACKEND_DIR/helm" -f "$VALUES" ${extra[@]+"${extra[@]}"} "$@"
rendered=$(mktemp)
trap 'rm -f "$rendered"' EXIT
helm template "$RELEASE" "$BACKEND_DIR/helm" --namespace "$NAMESPACE" -f "$VALUES" ${extra[@]+"${extra[@]}"} "$@" > "$rendered"
if grep -q '^kind: CiliumNetworkPolicy$' "$rendered"; then
  python3 "$SCRIPT_DIR/check-cilium.py"
fi
kubectl get namespace "$NAMESPACE" >/dev/null
helm upgrade --install "$RELEASE" "$BACKEND_DIR/helm" \
  --namespace "$NAMESPACE" --values "$VALUES" ${extra[@]+"${extra[@]}"} \
  --wait --wait-for-jobs --timeout "${HELM_TIMEOUT:-20m}" "$@"
printf 'Deployment ready. Inspect with: kubectl get pods -n %s\n' "$NAMESPACE"
