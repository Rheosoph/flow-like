From a repository checkout, prepare a new deployment after satisfying the host prerequisites:

```bash
cd apps/backend/docker-compose
python3 scripts/setup-env.py
cp flow-like.config.example.json flow-like.config.json
```

Edit the copied JSON for your OIDC provider and deployment origins. Set `FLOW_LIKE_RUNTIME_CONFIG_FILE=./flow-like.config.json` in the generated `.env`. Keep `EXECUTION_ISOLATION_MODE=per_run` and `COMPOSE_PROFILES=per-run`. Setup refuses to replace existing secrets.

```bash
python3 scripts/pull-images.py
python3 scripts/preflight.py
python3 scripts/up.py
docker compose ps --all
curl --fail http://localhost:8080/health
curl --fail http://localhost:3001/health
docker compose exec execution-manager /app/execution-manager healthcheck
```

For a reviewed release, pass an available tag to `pull-images.py --tag`. The helper pins workloads and sandbox references together. A moving sandbox tag fails preflight; do not assume an illustrative version exists in the registry.

Initialization jobs should exit successfully; long-running services should become healthy. Resolve initializer failures first. Avoid sharing interpolated Compose configuration because it contains secrets.

Open `http://localhost:3001`, sign in and create a synthetic app. Build a harmless Flow that returns or logs `restore-probe-v1`, run it remotely and inspect the result. Upload `restore-marker.txt` with that text and read it back.

Completion: successful health checks, manager readiness, one remote run and a matching file read. For a storage reachability failure, check the configured `s3.localhost` origin and resolution. The [installation guide](https://docs.flow-like.com/self-hosting/docker-compose/installation/) covers public TLS hosting.
