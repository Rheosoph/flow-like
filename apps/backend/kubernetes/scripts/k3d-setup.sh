#!/bin/bash
# =============================================================================
# k3d Local Development Setup for Flow-Like
# =============================================================================
# This script sets up a complete local Kubernetes environment with:
# - k3d cluster with local registry
# - Builds and pushes images to local registry
# - Installs the Helm chart with storage config from kubernetes/.env
#
# Usage:
#   ./scripts/k3d-setup.sh          # Full setup
#   ./scripts/k3d-setup.sh rebuild  # Rebuild images only
#   ./scripts/k3d-setup.sh delete   # Delete cluster
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$ROOT_DIR/../../.." && pwd)"
ENV_FILE="$ROOT_DIR/.env"

CLUSTER_NAME="flow-like"
REGISTRY_NAME="flow-like-registry"
REGISTRY_PORT="5111"
REGISTRY_INTERNAL_PORT="5000"
NAMESPACE="flow-like"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Load environment variables from .env file
load_env() {
    if [[ ! -f "$ENV_FILE" ]]; then
        log_error "Environment file not found: $ENV_FILE"
        log_error "Please copy .env.example to .env and configure your S3 credentials"
        exit 1
    fi

    log_info "Loading storage configuration from $ENV_FILE..."

    # Source env file
    set -a
    source "$ENV_FILE"
    set +a

    # Determine storage provider
    # Priority: STORAGE_PROVIDER env var > auto-detect from available credentials
    if [[ -z "${STORAGE_PROVIDER:-}" ]]; then
        # Auto-detect based on available credentials
        if [[ -n "${AZURE_STORAGE_ACCOUNT_NAME:-}" ]]; then
            STORAGE_PROVIDER="azure"
        elif [[ -n "${GCP_PROJECT_ID:-}" ]]; then
            STORAGE_PROVIDER="gcp"
        elif [[ -n "${R2_ACCOUNT_ID:-}" ]]; then
            STORAGE_PROVIDER="r2"
        elif [[ -n "${S3_ENDPOINT:-}" ]] || [[ -n "${AWS_ENDPOINT:-}" ]]; then
            STORAGE_PROVIDER="s3"
        elif [[ -n "${AWS_ACCESS_KEY_ID:-}" ]]; then
            STORAGE_PROVIDER="aws"
        else
            log_error "No storage configuration found. Set STORAGE_PROVIDER or provide credentials."
            exit 1
        fi
        log_info "Auto-detected storage provider: $STORAGE_PROVIDER"
    fi

    case "$STORAGE_PROVIDER" in
        aws)
            if [[ -z "${AWS_ACCESS_KEY_ID:-}" ]] || [[ -z "${AWS_SECRET_ACCESS_KEY:-}" ]]; then
                log_error "AWS storage requires AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY"
                exit 1
            fi
            HELM_STORAGE_PROVIDER="aws"
            HELM_STORAGE_VALUES="
storage:
  provider: aws
  aws:
    accessKeyId: \"${AWS_ACCESS_KEY_ID}\"
    secretAccessKey: \"${AWS_SECRET_ACCESS_KEY}\"
    region: \"${AWS_REGION:-us-east-1}\"
    endpoint: \"${AWS_ENDPOINT:-}\"
    metaBucket: \"${META_BUCKET:-flow-like-meta}\"
    contentBucket: \"${CONTENT_BUCKET:-flow-like-content}\"
    logBucket: \"${LOG_BUCKET:-flow-like-logs}\"
    usePathStyle: ${AWS_USE_PATH_STYLE:-false}
    runtimeRoleArn: \"${RUNTIME_ROLE_ARN:-}\""
            if [[ -n "${AWS_ENDPOINT:-}" ]]; then
                log_info "Using AWS S3-compatible: ${AWS_ENDPOINT}"
            else
                log_info "Using AWS S3 (default endpoint)"
            fi
            if [[ -n "${RUNTIME_ROLE_ARN:-}" ]]; then
                log_info "Using runtime role: ${RUNTIME_ROLE_ARN}"
            fi
            ;;
        azure)
            if [[ -z "${AZURE_STORAGE_ACCOUNT_NAME:-}" ]] || [[ -z "${AZURE_STORAGE_ACCOUNT_KEY:-}" ]]; then
                log_error "Azure storage requires AZURE_STORAGE_ACCOUNT_NAME and AZURE_STORAGE_ACCOUNT_KEY"
                exit 1
            fi
            HELM_STORAGE_PROVIDER="azure"
            HELM_STORAGE_VALUES="
storage:
  provider: azure
  azure:
    accountName: \"${AZURE_STORAGE_ACCOUNT_NAME}\"
    accountKey: \"${AZURE_STORAGE_ACCOUNT_KEY}\"
    metaContainer: \"${AZURE_META_CONTAINER:-${META_BUCKET:-flow-like-meta}}\"
    contentContainer: \"${AZURE_CONTENT_CONTAINER:-${CONTENT_BUCKET:-flow-like-content}}\"
    logContainer: \"${AZURE_LOG_CONTAINER:-logs}\""
            log_info "Using Azure Blob Storage: ${AZURE_STORAGE_ACCOUNT_NAME}"
            ;;
        gcp)
            if [[ -z "${GCP_PROJECT_ID:-}" ]]; then
                log_error "GCP storage requires GCP_PROJECT_ID"
                exit 1
            fi
            HELM_STORAGE_PROVIDER="gcp"
            # Escape the service account JSON for YAML
            local sa_key_escaped=""
            if [[ -n "${GOOGLE_APPLICATION_CREDENTIALS_JSON:-}" ]]; then
                sa_key_escaped=$(echo "$GOOGLE_APPLICATION_CREDENTIALS_JSON" | sed 's/"/\\"/g')
            fi
            HELM_STORAGE_VALUES="
storage:
  provider: gcp
  gcp:
    projectId: \"${GCP_PROJECT_ID}\"
    serviceAccountKey: \"${sa_key_escaped}\"
    metaBucket: \"${GCP_META_BUCKET:-${META_BUCKET:-flow-like-meta}}\"
    contentBucket: \"${GCP_CONTENT_BUCKET:-${CONTENT_BUCKET:-flow-like-content}}\"
    logBucket: \"${GCP_LOG_BUCKET:-${LOG_BUCKET:-flow-like-logs}}\""
            log_info "Using Google Cloud Storage: ${GCP_PROJECT_ID}"
            ;;
        s3)
            # Generic S3-compatible (MinIO, R2, etc.)
            local s3_endpoint="${S3_ENDPOINT:-${AWS_ENDPOINT:-}}"
            local s3_access_key="${S3_ACCESS_KEY_ID:-${AWS_ACCESS_KEY_ID:-}}"
            local s3_secret_key="${S3_SECRET_ACCESS_KEY:-${AWS_SECRET_ACCESS_KEY:-}}"

            if [[ -z "$s3_endpoint" ]]; then
                log_error "S3-compatible storage requires S3_ENDPOINT"
                exit 1
            fi
            if [[ -z "$s3_access_key" ]] || [[ -z "$s3_secret_key" ]]; then
                log_error "S3-compatible storage requires S3_ACCESS_KEY_ID and S3_SECRET_ACCESS_KEY"
                exit 1
            fi
            HELM_STORAGE_PROVIDER="s3"
            HELM_STORAGE_VALUES="
storage:
  provider: s3
  s3:
    endpoint: \"${s3_endpoint}\"
    region: \"${S3_REGION:-us-east-1}\"
    accessKeyId: \"${s3_access_key}\"
    secretAccessKey: \"${s3_secret_key}\"
    metaBucket: \"${META_BUCKET:-flow-like-meta}\"
    contentBucket: \"${CONTENT_BUCKET:-flow-like-content}\"
    logBucket: \"${LOG_BUCKET:-flow-like-logs}\"
    usePathStyle: ${S3_USE_PATH_STYLE:-true}"
            log_info "Using S3-compatible storage: ${s3_endpoint}"
            ;;
        r2)
            if [[ -z "${R2_ACCOUNT_ID:-}" ]]; then
                log_error "R2 storage requires R2_ACCOUNT_ID"
                exit 1
            fi
            if [[ -z "${R2_ACCESS_KEY_ID:-}" ]] || [[ -z "${R2_SECRET_ACCESS_KEY:-}" ]]; then
                log_error "R2 storage requires R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY"
                exit 1
            fi
            HELM_STORAGE_PROVIDER="r2"
            HELM_STORAGE_VALUES="
storage:
  provider: r2
  r2:
    accountId: \"${R2_ACCOUNT_ID}\"
    accessKeyId: \"${R2_ACCESS_KEY_ID}\"
    secretAccessKey: \"${R2_SECRET_ACCESS_KEY}\"
    metaBucket: \"${META_BUCKET:-flow-like-meta}\"
    contentBucket: \"${CONTENT_BUCKET:-flow-like-content}\"
    logBucket: \"${LOG_BUCKET:-flow-like-logs}\""
            log_info "Using Cloudflare R2 storage: ${R2_ACCOUNT_ID}"
            ;;
        *)
            log_error "Unknown STORAGE_PROVIDER: $STORAGE_PROVIDER"
            log_error "Supported: aws, azure, gcp, r2, s3"
            exit 1
            ;;
    esac

    log_info "Storage configuration loaded ✓"
}

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."

    for cmd in docker k3d kubectl helm; do
        if ! command -v "$cmd" &> /dev/null; then
            log_error "$cmd is not installed. Please install it first."
            exit 1
        fi
    done

    if ! docker info &> /dev/null; then
        log_error "Docker is not running. Please start Docker first."
        exit 1
    fi

    log_info "All prerequisites met ✓"
}

# Delete existing cluster
delete_cluster() {
    log_info "Deleting existing cluster (if any)..."
    k3d cluster delete "$CLUSTER_NAME" 2>/dev/null || true
    log_info "Cluster deleted ✓"
}

# Create k3d cluster with registry
create_cluster() {
    log_info "Creating k3d cluster with local registry..."

    if k3d cluster list | grep -q "$CLUSTER_NAME"; then
        log_warn "Cluster '$CLUSTER_NAME' already exists. Use 'rebuild' to update images."
        return 0
    fi

    k3d cluster create "$CLUSTER_NAME" \
        --registry-create "${REGISTRY_NAME}:0.0.0.0:${REGISTRY_PORT}" \
        --agents 2 \
        --port "8080:80@loadbalancer" \
        --port "3002:30002@loadbalancer" \
        --port "8443:443@loadbalancer"

    log_info "Cluster created ✓"

    log_info "Waiting for cluster to be ready..."
    kubectl wait --for=condition=Ready nodes --all --timeout=120s

    log_info "Cluster is ready ✓"
}

# Build and push images to local registry
build_and_push_images() {
    log_info "Building and pushing images to local registry..."

    local registry="localhost:${REGISTRY_PORT}"
    local tag="dev"

    # Enable BuildKit for cache mounts
    export DOCKER_BUILDKIT=1

    log_info "Building API image..."
    docker build \
        -f "$ROOT_DIR/api/Dockerfile" \
        -t "${registry}/flow-like/api:${tag}" \
        "$REPO_ROOT"

    log_info "Building Executor image..."
    docker build \
        -f "$ROOT_DIR/executor/Dockerfile" \
        -t "${registry}/flow-like/executor:${tag}" \
        "$REPO_ROOT"

    log_info "Building Migration image..."
    docker build \
        -f "$ROOT_DIR/migration/Dockerfile" \
        -t "${registry}/flow-like/migration:${tag}" \
        "$REPO_ROOT"

    log_info "Building Web image..."
    docker build \
        -f "$ROOT_DIR/web/Dockerfile" \
        -t "${registry}/flow-like/web:${tag}" \
        "$REPO_ROOT"

    log_info "Pushing images to local registry..."
    docker push "${registry}/flow-like/api:${tag}"
    docker push "${registry}/flow-like/executor:${tag}"
    docker push "${registry}/flow-like/migration:${tag}"
    docker push "${registry}/flow-like/web:${tag}"

    log_info "Images built and pushed ✓"
    echo "  - ${registry}/flow-like/api:${tag}"
    echo "  - ${registry}/flow-like/executor:${tag}"
    echo "  - ${registry}/flow-like/migration:${tag}"
    echo "  - ${registry}/flow-like/web:${tag}"
}

# Install Helm chart
install_helm_chart() {
    log_info "Installing Flow-Like Helm chart..."

    # Inside k3d, the registry is accessible as registry-name:internal-port
    local registry="${REGISTRY_NAME}:${REGISTRY_INTERNAL_PORT}"
    local jwt_output
    local backend_key
    local backend_pub
    local backend_kid

    jwt_output="$("$REPO_ROOT/tools/gen-execution-keys.sh" --export)"
    backend_key="$(printf '%s\n' "$jwt_output" | sed -n 's/^BACKEND_KEY=//p')"
    backend_pub="$(printf '%s\n' "$jwt_output" | sed -n 's/^BACKEND_PUB=//p')"
    backend_kid="$(printf '%s\n' "$jwt_output" | sed -n 's/^BACKEND_KID=//p')"

    if [[ -z "$backend_key" || -z "$backend_pub" || -z "$backend_kid" ]]; then
        log_error "Could not generate the BACKEND_KEY, BACKEND_PUB, and BACKEND_KID values."
        exit 1
    fi

    # Create namespace
    kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f -

    # Create values file for local dev
    cat > "$ROOT_DIR/helm/values-local.yaml" <<EOF
# Local development values (auto-generated by k3d-setup.sh)
# DO NOT COMMIT - uses storage config from kubernetes/.env

nameOverride: ""
fullnameOverride: "flow-like"

global:
  imageRegistry: "${registry}/"
  imagePullSecrets: []

api:
  enabled: true
  replicaCount: 1
  image:
    repository: flow-like/api
    tag: dev
    pullPolicy: Always
  resources:
    requests:
      memory: "256Mi"
      cpu: "100m"
    limits:
      memory: "512Mi"
      cpu: "500m"
  autoscaling:
    enabled: false

web:
  enabled: true
  replicaCount: 1
  image:
    repository: flow-like/web
    tag: dev
    pullPolicy: Always
  service:
    type: NodePort
    port: 3001
    nodePort: 30001
  resources:
    requests:
      memory: "32Mi"
      cpu: "10m"
    limits:
      memory: "64Mi"
      cpu: "100m"
  autoscaling:
    enabled: false

executorPool:
  enabled: true
  replicaCount: 1
  image:
    repository: flow-like/executor
    tag: dev
    pullPolicy: Always
  resources:
    requests:
      memory: "256Mi"
      cpu: "100m"
    limits:
      memory: "512Mi"
      cpu: "500m"
  autoscaling:
    enabled: false

executor:
  image:
    repository: flow-like/executor
    tag: dev
    pullPolicy: Always

# Execution JWT keypair (generated locally; values-local.yaml is gitignored)
jwt:
  backendKey: "${backend_key}"
  backendPub: "${backend_pub}"
  backendKid: "${backend_kid}"

# Internal CockroachDB (single node for dev)
database:
  type: internal
  migration:
    enabled: true
    image:
      repository: flow-like/migration
      tag: dev
      pullPolicy: Always
  internal:
    replicas: 1
    persistence:
      size: 1Gi
    resources:
      requests:
        memory: "256Mi"
        cpu: "100m"
      limits:
        memory: "1Gi"
        cpu: "1"
    tls:
      enabled: false

# Internal Redis
redis:
  enabled: true
  auth:
    enabled: false
  master:
    persistence:
      size: 1Gi
    resources:
      requests:
        memory: "64Mi"
        cpu: "50m"

# Storage configuration (injected from .env)
${HELM_STORAGE_VALUES}

# Ingress for API (accessible via localhost:8080)
ingress:
  enabled: true
  className: traefik
  annotations: {}
  hosts:
    - host: ""
      paths:
        - path: /
          pathType: Prefix

# Monitoring stack (Prometheus, Grafana, Tempo)
monitoring:
  enabled: true
  grafana:
    adminPassword: "admin"
    service:
      type: NodePort
      nodePort: 30002
    persistence:
      enabled: true
      size: 1Gi

# RuntimeClass disabled for local dev (no Kata containers)
runtimeClass:
  create: false

# NetworkPolicy - enable for realistic testing
networkPolicy:
  enabled: true
  executor:
    allowedEgress:
      - to:
          - namespaceSelector:
              matchLabels:
                name: flow-like
        ports:
          - port: 443
            protocol: TCP
EOF

    # Add values-local.yaml to gitignore
    if ! grep -q "values-local.yaml" "$ROOT_DIR/helm/.gitignore" 2>/dev/null; then
        echo "values-local.yaml" >> "$ROOT_DIR/helm/.gitignore"
    fi

    # Install helm chart (don't wait - CockroachDB init takes time)
    log_info "Deploying Helm chart (this may take a few minutes for CockroachDB init)..."

    # Build helm command with optional secrets file
    local helm_cmd=(helm upgrade --install flow-like "$ROOT_DIR/helm"
        -n "$NAMESPACE"
        --create-namespace
        -f "$ROOT_DIR/helm/values-local.yaml")

    # Include secrets file if it exists
    if [[ -f "$ROOT_DIR/helm/values-secrets.yaml" ]]; then
        helm_cmd+=(-f "$ROOT_DIR/helm/values-secrets.yaml")
        log_info "Including secrets from values-secrets.yaml"
    fi

    helm_cmd+=(--timeout 10m)

    "${helm_cmd[@]}"

    # Wait for CockroachDB to be initialized
    log_info "Waiting for CockroachDB to initialize..."
    kubectl wait --for=condition=Ready pod -l app.kubernetes.io/component=cockroachdb -n "$NAMESPACE" --timeout=300s || true

    # Wait for API to be ready
    log_info "Waiting for API to be ready..."
    kubectl wait --for=condition=Available deployment/flow-like-api -n "$NAMESPACE" --timeout=300s || true

    log_info "Helm chart installed ✓"
}

# Show status and next steps
show_status() {
    echo ""
    echo "=============================================="
    echo -e "${GREEN}Flow-Like Kubernetes Local Dev Environment${NC}"
    echo "=============================================="
    echo ""

    log_info "Cluster Status:"
    kubectl get nodes
    echo ""

    log_info "Pods in $NAMESPACE namespace:"
    kubectl get pods -n "$NAMESPACE"
    echo ""

    log_info "Services:"
    kubectl get svc -n "$NAMESPACE"
    echo ""

    echo "=============================================="
    echo -e "${GREEN}Access Points:${NC}"
    echo "=============================================="
    echo ""
    echo "  API:          http://localhost:8080"
    echo "  Grafana:      http://localhost:3002  (admin/admin)"
    echo ""
    echo "  CockroachDB:  kubectl port-forward svc/flow-like-cockroachdb-public 26257:26257 -n $NAMESPACE"
    echo ""
    echo "=============================================="
    echo -e "${GREEN}Useful Commands:${NC}"
    echo "=============================================="
    echo ""
    echo "  # View logs"
    echo "  kubectl logs -f deployment/flow-like-api -n $NAMESPACE"
    echo ""
    echo "  # Rebuild and redeploy"
    echo "  ./scripts/k3d-setup.sh rebuild"
    echo ""
    echo "  # Delete cluster"
    echo "  ./scripts/k3d-setup.sh delete"
    echo ""
}

# Main
main() {
    local action="${1:-setup}"

    case "$action" in
        setup)
            check_prerequisites
            load_env
            create_cluster
            build_and_push_images
            install_helm_chart
            show_status
            ;;
        rebuild)
            check_prerequisites
            load_env
            build_and_push_images
            # Restart deployments to pick up new images
            kubectl rollout restart deployment -n "$NAMESPACE" 2>/dev/null || true
            log_info "Deployments restarted. Waiting for rollout..."
            kubectl rollout status deployment -n "$NAMESPACE" --timeout=300s 2>/dev/null || true
            show_status
            ;;
        delete)
            delete_cluster
            log_info "Cluster deleted. Run './scripts/k3d-setup.sh' to recreate."
            ;;
        status)
            show_status
            ;;
        *)
            echo "Usage: $0 [setup|rebuild|delete|status]"
            echo ""
            echo "  setup   - Create cluster and deploy everything (default)"
            echo "  rebuild - Rebuild images and restart deployments"
            echo "  delete  - Delete the cluster"
            echo "  status  - Show current status"
            exit 1
            ;;
    esac
}

main "$@"
