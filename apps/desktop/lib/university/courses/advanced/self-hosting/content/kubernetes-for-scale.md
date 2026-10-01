Optional: install the service on Kubernetes. Prepare Linux execution nodes with gVisor, a matching RuntimeClass, Cilium enforcement, persistent storage, Helm/kubectl, DNS, OIDC and certificates before the timed class.

Read [cluster prerequisites](https://docs.flow-like.com/self-hosting/kubernetes/prerequisites/). The default is `execution.isolationMode: per_run` with bundled RustFS and an internal single-node evaluation database. Production needs a separately operated database and reviewed ingress/network settings. Local k3d evaluation uses the distinct trusted-shared path.

From `apps/backend/kubernetes`, configure the hub JSON and real origins as described in [installation](https://docs.flow-like.com/self-hosting/kubernetes/installation/). Select the file through `FLOW_LIKE_CONFIG_FILE` and prepare this cluster's `values-operator.yaml` before deployment.

```bash
./scripts/setup-config.sh
./scripts/resolve-images.py --tag dev
kubectl config current-context
kubectl create namespace flow-like --dry-run=client -o yaml | kubectl apply -f -
kubectl apply -f .generated/secrets.yaml
./scripts/deploy.sh -f values-operator.yaml
```

The example `dev` selection is for disposable practice. Resolve a reviewed release for a maintained service. Generated Secrets stay private. The deployment helper validates the chart and cluster prerequisites before updating the release.

Repeat the harmless Flow and file round trip from Compose. Inspect API/manager rollout and the run outcome. Check HTTP synchronous dispatch and the Redis background lane for per-run mode; shared-pool mode has different constraints.

Completion: record context/namespace, image digests, mode, health and fixture output. Installation does not replace live isolation, load or restore testing.
