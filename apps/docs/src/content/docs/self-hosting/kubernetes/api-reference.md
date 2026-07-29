---
title: API Reference
description: Inspect and test the Flow-Like API running on Kubernetes.
sidebar:
  order: 61
---

The Kubernetes image uses the shared Flow-Like API router. The generated OpenAPI document in the running deployment is the authoritative endpoint reference; this page describes how to reach it and highlights the Kubernetes-specific routes.

## Open the live reference

Start a foreground port forward:

```bash
kubectl port-forward service/flow-like-api 8083:8080 -n flow-like
```

Then open:

| URL | Purpose |
|---|---|
| `http://localhost:8083/swagger-ui` | Interactive Swagger UI |
| `http://localhost:8083/api-doc/openapi.json` | Machine-readable OpenAPI document |
| `http://localhost:8083/api/v1/` | Hub configuration |
| `http://localhost:8083/api/v1/version` | API version route |

`8083` is an arbitrary local port. `8080` is the API Service port in the chart:

```bash
kubectl port-forward service/flow-like-api 3000:8080 -n flow-like
```

With that command, use `http://localhost:3000`.

## Health endpoints

### Kubernetes probes

These routes are added by the Kubernetes API binary and are outside `/api/v1`.

| Endpoint | Expected success body | Kubernetes behavior |
|---|---|---|
| `GET /health/live` | `{"status":"healthy","version":"…"}` | Restarts the container after repeated failures |
| `GET /health/ready` | `{"status":"ready","version":"…"}` | Removes the Pod from Service endpoints while failing |
| `GET /health/startup` | `{"status":"started","version":"…"}` | Delays liveness and readiness checks during startup |

```bash
curl -fsS http://localhost:8083/health/live
curl -fsS http://localhost:8083/health/ready
curl -fsS http://localhost:8083/health/startup
```

The current probe handlers report process state. They do not query the database.

### Shared API health

| Endpoint | Purpose |
|---|---|
| `GET /api/v1/health` | Basic shared-router health check |
| `GET /api/v1/health/db` | Database ping with round-trip time in milliseconds |

```bash
curl -fsS http://localhost:8083/api/v1/health
curl -fsS http://localhost:8083/api/v1/health/db
```

Example shapes:

```json
{"status":"ok"}
```

```json
{"rtt":5}
```

## API route groups

The shared router currently mounts these high-level groups under `/api/v1`:

| Prefix | Area |
|---|---|
| `/apps` | Apps and nested boards, events, pages, routes, data, roles, teams, and packages |
| `/user`, `/profile` | User and profile operations |
| `/execution` | Executor progress/events and run status |
| `/sink` | Sink management and trigger delivery |
| `/registry` | WASM package registry |
| `/store`, `/solution`, `/courses` | Public catalogs and solution content |
| `/auth`, `/oauth` | OpenID proxy/configuration and OAuth flows |
| `/ai`, `/chat`, `/embeddings` | AI and chat services |
| `/usage`, `/audit`, `/admin` | Usage, audit, and administration |
| `/info`, `/health` | Hub information and shared health |

Routes change as the shared API evolves. Use the live OpenAPI document for methods, request bodies, response schemas, and authentication requirements instead of copying a static endpoint list.

## Inbound app routes

Registered inbound interfaces are mounted separately:

| Prefix | Interface |
|---|---|
| `/r/*` | App-defined inbound REST routes |
| `/m/*` | App-defined inbound MCP routes |

These routers bypass the API's general user-JWT middleware because each registration enforces its own authentication. Treat them as public-facing integration surfaces and configure every registration explicitly.

## Metrics

Metrics use a separate listener and Service port:

```bash
kubectl port-forward service/flow-like-api 9090:9090 -n flow-like
curl -fsS http://localhost:9090/metrics
```

Do not expose the metrics port publicly unless an authenticated monitoring path protects it.

## Test from inside the cluster

Start an ephemeral curl Pod:

```bash
kubectl run flow-like-debug \
  --image=curlimages/curl \
  --restart=Never \
  --rm -it \
  -n flow-like \
  -- sh
```

Inside the Pod:

```bash
curl -fsS http://flow-like-api:8080/health/ready
curl -fsS http://flow-like-api:8080/api/v1/health/db
```

The Service DNS name assumes the Helm release is named `flow-like`.

## Common response codes

| Code | Meaning |
|---:|---|
| 200 | Request succeeded |
| 201 | Resource created |
| 204 | Request succeeded with no response body |
| 400 | Invalid input |
| 401 | Missing or invalid authentication |
| 403 | Authenticated principal lacks permission |
| 404 | Route or resource was not found |
| 409 | Request conflicts with current state |
| 429 | Usage or rate limit reached |
| 500 | Internal server error |
| 503 | Required service or feature is unavailable |

## Related

- [API Service](/self-hosting/kubernetes/api/)
- [kubectl Basics](/self-hosting/kubernetes/kubectl-basics/)
- [Security](/self-hosting/kubernetes/security/)
