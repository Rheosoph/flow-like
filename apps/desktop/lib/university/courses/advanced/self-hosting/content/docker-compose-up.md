Friday afternoon on `staging-01`, the VM the infrastructure team lent you. You clone the repo, `cd apps/backend/docker-compose`, copy `.env.example` to `.env`, and run `docker compose up -d --build`. Twenty minutes of Rust compilation later — the first build is the slow one — containers come up. All except `api`, which restart-loops with a startup error about an unsupported runtime provider.

> **Predict first:** the machine is fine and you typed nothing wrong. Which line of the copied template killed the API?

## 1 · Know your ten services

The Compose file runs the complete stack, not just an API:

| Service | Published port | Job |
| --- | --- | --- |
| `web` | `3001` | Flow-Like web application |
| `api-gateway` | `8080` | Nginx entrypoint, load-balances the API replicas |
| `api` | internal | API replicas: auth, app state, dispatch |
| `runtime` | internal `9000` | Shared execution workers + Redis queue consumers |
| `compiler` | `8081` | WASM compilation for custom nodes |
| `signaling` | `4444` | Realtime collaboration |
| `sink-services` | — | Cron and configured bot/event adapters |
| `postgres` / `redis` | `5432` / `6379` | Metadata / run state and queues |
| `db-init` | — | One-time database initialization job |

@DockerComposeArchitecture

The map above shows how they connect: browser and desktop clients enter through the web app on 3001 and the Nginx API gateway on 8080; the API replicas sit behind the gateway; below them PostgreSQL, Redis, and a dashed object-storage card marked "external provider"; along the bottom, the runtime worker pool (1..N), the WASM compiler, sink services, and signaling, with an optional monitoring strip — Prometheus, Tempo, exporters, Grafana — enabled by the `monitoring` profile.

That dashed card matters: object storage is the one thing Compose does not create. You bring three buckets or containers — metadata, content, and logs — before the stack starts.

## 2 · The four decisions the template forces

**Storage provider — the hook's answer.** The copied template selects `aws`, but the checked-in Compose API image ships with Azure, GCP, and R2 runtime-credential support and omits its AWS feature. At startup the API rejects `aws` as an unsupported runtime provider and dies. Pick Azure, GCP, or the R2 configuration — or rebuild the API with its `flow-like-api/aws` feature.

**No empty strings.** Compose injects `RUNTIME_CREDENTIALS_PROVIDER` and `CDN_BUCKET_NAME` into the API even when they're blank, and the configuration treats an explicit empty value as configured. Nothing falls back, and an empty credential field never activates an ambient cloud credential chain. Set real values for the selected provider's meta, content, and log names too.

**Trust keys.** Flow-Like signs backend JWTs with one ES256 keypair:

```bash
../../../tools/gen-execution-keys.sh --export
```

Copy the emitted `BACKEND_KEY`, `BACKEND_PUB`, and `BACKEND_KID` into `.env`. The private key stays with the API; runtime and compiler receive only the public key.

**Identity.** The hub configuration uses OpenID Connect placeholders. Wire in your provider's authority, client ID, and callback URLs — and never expose a deployment that still points at `https://your-auth-provider.com`.

Server-side schedules and bots run in `sink-services` behind `SINK_SECRET` and a scoped JWT; the Events course covers what those events do. The stack starts without the token — sink-services will just complain it can't authenticate.

## 3 · Verify like an operator

```bash
docker compose config --quiet
docker compose up -d --build
docker compose ps --all
```

`db-init` should exit successfully — it's a one-time job, not a crash. Then hit the health endpoints on 8080, 3001, 4444, and 8081 with `curl --fail`, and check the internal runtime from inside its container:

```bash
docker compose exec runtime curl --fail http://localhost:9000/health
```

Open `http://localhost:3001`, log in through your OIDC provider, and run a flow. That's the entire Flow-Like backend on one VM — Priya's customer records now sleep in your buckets.

**Watch out:** `docker compose config` without `--quiet` prints interpolated secrets. Never paste its output into a ticket or public log.

**Recap**

- Ten services, one host; the published API port belongs to the Nginx gateway, and object storage is always external.
- The template's storage defaults don't survive contact with the stock image — choose Azure, GCP, or R2, and never leave injected values explicitly empty.
- Generate the `BACKEND_*` key trio, replace the OIDC placeholders, and verify with `ps --all` plus health checks.
