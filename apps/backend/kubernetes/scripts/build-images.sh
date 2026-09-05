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
for component in ${COMPONENTS:-api executor execution-manager runtime compiler signaling migration object-store-init web}; do
  context="$REPO_DIR"
  case "$component" in
    api|executor|execution-manager|migration|web) dockerfile="$BACKEND_DIR/$component/Dockerfile" ;;
    runtime|compiler|signaling) dockerfile="$BACKEND_DIR/../docker-compose/$component/Dockerfile" ;;
    object-store-init) context="$BACKEND_DIR/../docker-compose/object-store"; dockerfile="$context/Dockerfile" ;;
    *) echo "Unknown component: $component" >&2; exit 1 ;;
  esac
  repository="$REGISTRY/flow-like-k8s-$component"
  reference="$repository:$TAG"
  extra=()
  if [[ "$component" == api ]]; then
    extra+=(--build-arg "FLOW_LIKE_CONFIG=${FLOW_LIKE_CONFIG:-apps/backend/kubernetes/flow-like.config.example.json}")
  fi
  if [[ "$component" == web ]]; then
    extra+=(--build-arg "NEXT_PUBLIC_API_URL=${PUBLIC_API_URL:-http://localhost:8080}")
    extra+=(--build-arg "NEXT_PUBLIC_REDIRECT_URL=${PUBLIC_WEB_URL:-http://localhost:3001}/callback")
    extra+=(--build-arg "NEXT_PUBLIC_REDIRECT_LOGOUT_URL=${PUBLIC_WEB_URL:-http://localhost:3001}/")
  fi
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
mapping={'api':('api','image'),'web':('web','image'),'signaling':('signaling','image'),'compiler':('compiler','image'),'migration':('database','migration','image'),'runtime':('executionManager','queueBridge','image'),'execution-manager':('executionManager','image'),'object-store-init':('rustfs','bootstrap','image')}
values=json.loads(Path(sys.argv[2]).read_text()) if Path(sys.argv[2]).exists() else {}
values.setdefault('global',{})['imageRegistry']=''
for line in open(sys.argv[1]):
    component,repository,tag,digest=line.rstrip('\n').split('\t')
    if component=='executor':
        values.setdefault('executor',{})['image']={'repository':repository,'tag':tag,'pullPolicy':'IfNotPresent'}
        values.setdefault('executorPool',{})['image']={'repository':repository,'tag':tag,'pullPolicy':'IfNotPresent'}
        values.setdefault('executionManager',{}).setdefault('sandbox',{})['image']=repository+'@'+digest if digest else ''
    else:
        target=values
        for key in mapping[component][:-1]:
            target=target.setdefault(key,{})
        image={'repository':repository,'tag':tag,'pullPolicy':'IfNotPresent'}
        if component=='execution-manager' and digest:
            image['digest']=digest
        target['image']=image
with open(sys.argv[2],'w') as out:
    json.dump(values,out,indent=2)
    out.write('\n')
PY
printf 'Wrote image values to %s\n' "$OUTPUT_FILE"
if [[ "$PUSH" != true ]]; then
  echo 'Per-run Kubernetes deployment also requires pushed manager and executor digests. Set PUSH=true or supply verified digest values.'
fi
