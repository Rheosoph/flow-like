# Flow-Like Kubernetes Backend

Production-ready Kubernetes deployment using Helm with built-in observability.

## Quick Start

**Local development (k3d):**
```bash
cd apps/backend/kubernetes
./scripts/k3d-setup.sh
```

**Production:**
```bash
# First customize values-production.yaml and create its referenced Secrets.
helm upgrade --install flow-like ./helm -n flow-like \
  --values ./helm/values-production.yaml
```

## Documentation

Full step-by-step documentation: **[docs.flow-like.com/self-hosting/kubernetes](https://docs.flow-like.com/self-hosting/kubernetes/)**

| Guide | Description |
|-------|-------------|
| [Overview](https://docs.flow-like.com/self-hosting/kubernetes/overview/) | Architecture and components |
| [Prerequisites](https://docs.flow-like.com/self-hosting/kubernetes/prerequisites/) | System requirements |
| [Installation](https://docs.flow-like.com/self-hosting/kubernetes/installation/) | Step-by-step setup |
| [Configuration](https://docs.flow-like.com/self-hosting/kubernetes/configuration/) | Helm values reference |
| [Local Development](https://docs.flow-like.com/self-hosting/kubernetes/local-development/) | k3d setup guide |
| [Monitoring](https://docs.flow-like.com/self-hosting/kubernetes/monitoring/) | Prometheus, Grafana, Tempo |
| [Storage](https://docs.flow-like.com/self-hosting/kubernetes/storage/) | AWS, Azure, GCP, R2 |
| [Database](https://docs.flow-like.com/self-hosting/kubernetes/database/) | CockroachDB configuration |
| [Security](https://docs.flow-like.com/self-hosting/kubernetes/security/) | Production hardening |

## Components

| Component | Type | Description |
|-----------|------|-------------|
| API | Deployment | Flow-Like API with HPA |
| Web | Deployment | Web application (Next.js static) |
| Executor Pool | Deployment | Warm execution workers with HPA |
| CockroachDB | StatefulSet | Single-node evaluation database; use an external database in production |
| Redis | Deployment | Job queue and state |
| Prometheus | Deployment | Metrics collection |
| Grafana | Deployment | Pre-configured dashboards |
| Tempo | Deployment | Distributed tracing |

## Build Caching

The Dockerfiles use BuildKit cache mounts for faster rebuilds. Build scripts automatically enable BuildKit.

**First build:** ~15-20 minutes (full compilation)
**Subsequent builds:** ~1-3 minutes (incremental)

To clear the build cache:
```bash
docker builder prune --filter type=exec.cachemount
```

## Common Commands

```bash
# Check pod status
kubectl get pods -n flow-like

# View logs
kubectl logs -f deployment/flow-like-api -n flow-like

# Get Grafana password
kubectl get secret flow-like-grafana -n flow-like \
  -o jsonpath='{.data.admin-password}' | base64 -d && echo

# Upgrade Helm release
helm upgrade flow-like ./helm -n flow-like

# Uninstall
helm uninstall flow-like -n flow-like
```

## Local Development Access (k3d)

After running `./scripts/k3d-setup.sh`, services are available at:

| Service | URL | Notes |
|---------|-----|-------|
| API | http://localhost:8080 | NodePort (automatic) |
| Web App | `kubectl port-forward -n flow-like svc/flow-like-web 3001:3001` | Port forward |
| Grafana | http://localhost:3002 | NodePort, default login: admin/admin |
| Registry | http://localhost:5111 | Local Docker registry |

## Production Access

For production, use port-forwarding or configure ingress:

```bash
# Port-forward API
kubectl port-forward -n flow-like svc/flow-like-api 8080:8080

# Port-forward Web App
kubectl port-forward -n flow-like svc/flow-like-web 3001:3001

# Port-forward Grafana
kubectl port-forward -n flow-like svc/flow-like-grafana 3000:80
```
