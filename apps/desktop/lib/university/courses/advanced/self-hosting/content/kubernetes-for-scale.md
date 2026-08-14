Staging convinced everyone. Now the platform team wants the real thing: multi-host scheduling, autoscaling, network policies — a production cluster inside the VPC. You point Helm at the repository chart with its default values, install, and watch every single pod land in `ImagePullBackOff`.

> **Predict first:** the chart is checked into the repo you already cloned. Why can't your cluster pull a single image?

## 1 · What the chart gives you

The Helm chart at `apps/backend/kubernetes/helm/` deploys, by default: the web app, the API, and a warm executor pool (each a Deployment plus Service), an internal CockroachDB, chart-managed Redis with authentication, a database-migration Job, and a Prometheus–Grafana–Tempo monitoring stack. Ingress, the WASM compiler, and sink services exist in the values but start disabled. Autoscaling is configurable and off by default.

@KubernetesArchitecture

Read the diagram's badges — they are the course in miniature: clients pass through an optional Ingress to the web pods and the API Service (HPA configurable); the state row shows CockroachDB marked "single-node default", Redis marked "no queue consumer deployed", a dashed external object-storage card, and an optional WASM compiler; the execution row shows the warm executor pool as the default path, a Job dispatcher marked "incomplete — creates Job; runner pending", optional sink services, and a network card that says "check selectors". The observability strip is enabled by chart defaults.

## 2 · Defaults are for k3d

Here's the hook's answer: the default image values target the local registry `k3d-flow-like.localhost:5000` with `pullPolicy: Never`. They exist for the repository's bundled local-development script (`./scripts/k3d-setup.sh` builds images straight into a k3d cluster) and are intentionally not an installable production configuration. Storage credentials and backend keys default to empty for the same reason.

A production install therefore always overrides three things: images your cluster can pull (plus `imagePullSecrets` for a private registry), the JWT Secret, and the storage provider. Keep the credentials out of values files with the `existingSecret` pattern:

```bash
kubectl -n flow-like create secret generic flow-like-backend-jwt \
  --from-env-file=flow-like-backend-jwt.env \
  --dry-run=client -o yaml | kubectl apply -f -
```

The JWT Secret carries the same `BACKEND_KEY`, `BACKEND_PUB`, and `BACKEND_KID` trio you generated on staging. The storage Secret follows the same provider rules as Compose: the stock API speaks Azure, GCP, and R2; AWS and generic S3 need a rebuilt image.

## 3 · The database decision

`database.type: internal` runs one CockroachDB pod with `start-single-node --insecure`. That's an evaluation database, full stop. And don't try to fix it by raising `database.internal.replicas` — independent single-node pods never form a cluster; they just corrupt your assumptions.

Production means `database.type: external` plus a Secret containing `DATABASE_URL` (PostgreSQL-compatible, `sslmode=require`). One more policy call: the migration Job runs on every install and upgrade and currently executes `prisma db push --accept-data-loss`. Read that flag again, then decide whether it runs automatically or whether you disable `database.migration` and run an approved migration process instead.

## 4 · Render, install, verify

Render before you install — it's how you catch a wrong Secret name while it's still cheap:

```bash
helm lint apps/backend/kubernetes/helm --values flow-like-values.yaml
helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like --values flow-like-values.yaml
helm upgrade --install flow-like apps/backend/kubernetes/helm \
  --namespace flow-like --values flow-like-values.yaml
```

Then verify the rollout and open the app:

```bash
kubectl rollout status deployment/flow-like-api -n flow-like
kubectl rollout status deployment/flow-like-executor-pool -n flow-like
kubectl port-forward -n flow-like service/flow-like-web 3001:3001
```

**Watch out:** the chart's default `execution.asyncBackend` is `redis`, but the chart's executor-pool binary doesn't consume that Redis list. For a chart-only deployment set both execution lanes to `http` — the full dispatch story is next lesson.

**Recap**

- Chart defaults are a k3d development profile; production overrides images, the JWT Secret, and storage via `existingSecret`.
- Internal CockroachDB is single-node and insecure — use an external `DATABASE_URL` for production and review the migration Job's `--accept-data-loss`.
- `helm lint` and `helm template` before `helm upgrade --install`; verify with rollout status.
