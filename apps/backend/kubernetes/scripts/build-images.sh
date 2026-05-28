#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Default values
REGISTRY="${REGISTRY:-ghcr.io/rheosoph}"
TAG="${TAG:-latest}"
PUSH="${PUSH:-false}"

echo "Building Flow-Like Kubernetes images..."
echo "Registry: $REGISTRY"
echo "Tag: $TAG"

# Enable BuildKit for cache mounts
export DOCKER_BUILDKIT=1

# Build API image
echo "Building API image..."
docker build \
    -f "$ROOT_DIR/api/Dockerfile" \
    -t "$REGISTRY/flow-like-k8s-api:$TAG" \
    "$ROOT_DIR/../../../"

# Build Executor image
echo "Building Executor image..."
docker build \
    -f "$ROOT_DIR/executor/Dockerfile" \
    -t "$REGISTRY/flow-like-k8s-executor:$TAG" \
    "$ROOT_DIR/../../../"

if [ "$PUSH" = "true" ]; then
    echo "Pushing images..."
    docker push "$REGISTRY/flow-like-k8s-api:$TAG"
    docker push "$REGISTRY/flow-like-k8s-executor:$TAG"
fi

echo "Build complete!"
echo ""
echo "Images built:"
echo "  - $REGISTRY/flow-like-k8s-api:$TAG"
echo "  - $REGISTRY/flow-like-k8s-executor:$TAG"
