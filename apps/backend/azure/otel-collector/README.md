# Azure native OTLP collector

This image receives OTLP/gRPC from the Flow-Like API on the Container Apps
environment's internal TCP endpoint. It exports logs, traces, and metrics to
Azure Monitor native OTLP ingestion through a private Data Collection Endpoint.

Authentication is Microsoft Entra only: the collector's user-assigned managed
identity obtains a token for `https://monitor.azure.com/.default`, and Terraform
grants that identity `Monitoring Metrics Publisher` only on the deployment's
Data Collection Rule. There are no instrumentation keys, workspace keys, client
secrets, or public ingestion endpoints.

## Build and promote

Build this directory as the context, scan and sign the result, push it to the
deployment ACR from the private CI runner, then place its immutable ACR digest
in `otel_collector_image`:

```sh
docker buildx build \
  --platform linux/amd64 \
  --tag "$ACR_LOGIN_SERVER/flowlike/otel-collector:$VERSION" \
  --push \
  apps/backend/azure/otel-collector
```

The base is pinned to the official multi-architecture 0.148.0 manifest. Update
both the digest and version comment together after vulnerability review.

## Runtime security boundary

Port 4317 is exposed only as internal TCP ingress inside the private control
Container Apps environment. The health extension on 13133 has no ingress and
is used only by Container Apps probes. Collector logs go to standard output so
the environment diagnostic setting retains exporter/authentication failures in
the private Log Analytics workspace.

The queue is deliberately memory-only: it cannot place telemetry or Azure
tokens in an unmanaged persistent volume. Two replicas are used by the maximum
profile. Validate DCR ingestion, exporter retry behavior, alert delivery, and
both replica failure scenarios before acknowledging the observability runtime
readiness gate.
