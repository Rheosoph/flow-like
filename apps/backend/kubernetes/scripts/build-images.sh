#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$BACKEND_DIR/../../.." && pwd)"
REGISTRY="${REGISTRY:-ghcr.io/rheosoph}"
TAG="${TAG:-local}"
PUSH="${PUSH:-false}"
OUTPUT_FILE="${IMAGE_VALUES_FILE:-$BACKEND_DIR/.generated/values-images.yaml}"
export DOCKER_BUILDKIT=1
records=$(mktemp)
trap 'rm -f "$records"' EXIT
for component in ${COMPONENTS:-api executor execution-manager sink-trigger runtime compiler signaling migration object-store-init audit-worker web}; do
  context="$REPO_DIR"
  case "$component" in
    api|executor|execution-manager|migration|web|sink-trigger)
      dockerfile="$BACKEND_DIR/$component/Dockerfile"
      repository="$REGISTRY/flow-like-kubernetes-$component" ;;
    runtime|compiler|signaling)
      dockerfile="$BACKEND_DIR/../docker-compose/$component/Dockerfile"
      repository="$REGISTRY/flow-like-docker-compose-$component" ;;
    audit-worker)
      dockerfile="$BACKEND_DIR/../audit-worker/Dockerfile"
      repository="$REGISTRY/flow-like-audit-worker" ;;
    object-store-init)
      dockerfile="$BACKEND_DIR/../docker-compose/object-store/Dockerfile"
      repository="$REGISTRY/flow-like-docker-compose-$component" ;;
    *) echo "Unknown component: $component" >&2; exit 1 ;;
  esac
  reference="$repository:$TAG"
  extra=()
  if [[ ( "$component" == api || "$component" == audit-worker ) && -n "${FLOW_LIKE_BUILD_CONFIG:-}" ]]; then
    # The API's fallback behind the runtime config; the worker's only audit policy.
    extra+=(--build-arg "FLOW_LIKE_CONFIG=$FLOW_LIKE_BUILD_CONFIG")
  fi
  # Web deployment URLs come from Helm runtime settings, not build arguments.
  docker build -f "$dockerfile" -t "$reference" ${extra[@]+"${extra[@]}"} "$context"
  digest=""
  if [[ "$PUSH" == true ]]; then
    docker push "$reference"
    digest=$(docker image inspect "$reference" --format '{{json .RepoDigests}}' | python3 -c 'import json,sys; repo=sys.argv[1]; values=[x.split("@",1)[1] for x in json.load(sys.stdin) if x.startswith(repo+"@")]; assert values,"pushed image digest missing"; print(values[0])' "$repository")
  fi
  printf '%s\t%s\t%s\t%s\n' "$component" "$repository" "$TAG" "$digest" >> "$records"
done
mkdir -p "$(dirname "$OUTPUT_FILE")"
python3 - "$records" "$OUTPUT_FILE" <<'PY'
import json,sys
from pathlib import Path
mapping={'audit-worker':[('audit','image')],'api':[('api','image')],'web':[('web','image')],'signaling':[('signaling','image')],'compiler':[('compiler','image')],'migration':[('database','migration','image')],'runtime':[('executionManager','queueBridge','image')],'execution-manager':[('executionManager','image')],'object-store-init':[('rustfs','bootstrap','image')],'sink-trigger':[('sinkServices','image')],'executor':[('executor','image'),('executorPool','image')]}
values=json.loads(Path(sys.argv[2]).read_text()) if Path(sys.argv[2]).exists() else {}
values.setdefault('global',{})['imageRegistry']=''
for line in open(sys.argv[1]):
    component,repository,tag,digest=line.rstrip('\n').split('\t')
    image={'repository':repository,'tag':tag,'pullPolicy':'IfNotPresent'}
    if digest:
        image['digest']=digest
    for path in mapping[component]:
        target=values
        for key in path[:-1]:
            target=target.setdefault(key,{})
        target[path[-1]]=dict(image)
    if component=='executor':
        values.setdefault('executionManager',{}).setdefault('sandbox',{})['image']=repository+'@'+digest if digest else ''
with open(sys.argv[2],'w') as out:
    json.dump(values,out,indent=2)
    out.write('\n')
PY
printf 'Wrote image values to %s\n' "$OUTPUT_FILE"
if [[ "$PUSH" != true ]]; then
  echo 'Per-run Kubernetes deployment also requires pushed manager and executor digests. Set PUSH=true or supply verified digest values.'
fi
