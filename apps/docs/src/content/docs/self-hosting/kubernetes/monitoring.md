---
title: Monitoring
description: Operate the Prometheus, Grafana, and Tempo resources in the Helm chart
sidebar:
  order: 35
---

The Helm chart can deploy an in-cluster Prometheus, Grafana, and Tempo. The
default `values.yaml` sets both monitoring and tracing to enabled.

```yaml
monitoring:
  enabled: true
  tracing:
    enabled: true
    retention: 72h
```

Disable all monitoring resources with:

```bash
helm upgrade --install flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --set monitoring.enabled=false
```

Setting `monitoring.tracing.enabled=false` keeps Prometheus and Grafana but
omits Tempo and the tracing dashboard.

## What the chart deploys

| Component | Purpose | Service port |
| --- | --- | --- |
| Prometheus | Metrics collection, recording rules, and built-in alert rules | `9090` |
| Grafana | Provisioned Prometheus/Tempo data sources and dashboards | `80` |
| Tempo | OTLP trace ingestion and trace queries | `4317`, `4318`, `3200` |

Resource names use the Helm release's computed fullname. The examples below
assume a release named `flow-like`.

## Prometheus

The chart's own Prometheus discovers:

- Flow-Like API metrics on the Service's `metrics` port;
- executor-pool metrics on its `metrics` port;
- compiler metrics when the compiler is enabled;
- CockroachDB metrics when the internal database is enabled;
- Kubernetes API, node, and cAdvisor metrics;
- annotated pods in the release namespace.

It does **not** deploy a Redis exporter or kube-state-metrics. Dashboards and
rules that query those exporters require you to install and scrape compatible
exporters separately.

Access the UI:

```bash
kubectl port-forward --namespace flow-like \
  service/flow-like-prometheus 9090:9090
```

Then open `http://localhost:9090/targets` and confirm every required target is
up.

### Supported values

```yaml
monitoring:
  prometheus:
    image:
      repository: prom/prometheus
      tag: v2.48.0
      pullPolicy: IfNotPresent
    service:
      type: ClusterIP
    scrapeInterval: 15s
    evaluationInterval: 15s
    retention: 15d
    persistence:
      enabled: true
      storageClass: ""
      size: 50Gi
    resources:
      requests:
        memory: 512Mi
        cpu: 250m
      limits:
        memory: 2Gi
        cpu: "1"
```

The Prometheus ConfigMap contains fixed recording and alert rules from
`templates/monitoring/prometheus.yaml`.

:::note
`monitoring.prometheusRule.rules` exists in `values.yaml`, but the current chart
does not render a `PrometheusRule` resource from it. Likewise,
`monitoring.alertmanager.enabled` adds an Alertmanager target to Prometheus but
does not deploy an Alertmanager. Add those resources separately before enabling
or documenting them as operational.
:::

## Grafana

Access Grafana:

```bash
kubectl port-forward --namespace flow-like \
  service/flow-like-grafana 3000:80
```

The admin user defaults to `admin`. Read the generated password:

```bash
kubectl get secret flow-like-grafana \
  --namespace flow-like \
  --output jsonpath='{.data.admin-password}' |
  base64 --decode
```

The provisioned configuration is in
`flow-like-grafana-config`, including:

- Prometheus as the default data source;
- Tempo when tracing is enabled;
- dashboard provisioning under the **Flow-Like** folder.

The Deployment mounts overview, API, and executor dashboards. It additionally
mounts database, Redis, and tracing dashboards when their corresponding
features are enabled. A dashboard being present does not prove every queried
metric is available; inspect blank panels against Prometheus targets and series.

Supported Grafana values include image, service type, admin credentials,
domain, anonymous access, persistence, resources, ingress, node selectors, and
tolerations:

```yaml
monitoring:
  grafana:
    adminUser: admin
    adminPassword: ""
    anonymous:
      enabled: false
    persistence:
      enabled: true
      storageClass: ""
      size: 10Gi
    ingress:
      enabled: false
      className: ""
      annotations: {}
      hosts:
        - host: grafana.flow-like.local
          paths:
            - path: /
              pathType: Prefix
      tls: []
```

Set `adminPassword` through a protected values file or secret-management
workflow. Do not commit it to the normal values file.

## Tempo and tracing

When tracing is enabled, the chart:

- deploys one Tempo instance;
- configures OTLP gRPC on `4317` and OTLP HTTP on `4318`;
- exposes Tempo queries on `3200`;
- injects `OTEL_EXPORTER_OTLP_ENDPOINT` into the API, executor pool, and
  compiler;
- uses local Tempo storage with the configured retention.

```yaml
monitoring:
  tracing:
    enabled: true
    retention: 72h
  tempo:
    persistence:
      enabled: false
      storageClass: ""
      size: 10Gi
```

With persistence disabled, traces are lost when the Tempo pod is replaced.

## Prometheus Operator integration

The optional value:

```yaml
monitoring:
  serviceMonitor:
    enabled: true
    interval: 30s
    scrapeTimeout: 10s
```

renders a `monitoring.coreos.com/v1` `ServiceMonitor`, so the Prometheus
Operator CRDs must already exist.

The checked-in ServiceMonitor selects the API Service and asks for the `http`
port at `/metrics`; the chart's built-in Prometheus instead scrapes the
dedicated `metrics` port. Render and validate the ServiceMonitor against your
API before relying on it:

```bash
helm template flow-like apps/backend/kubernetes/helm \
  --namespace flow-like \
  --set monitoring.serviceMonitor.enabled=true
```

## Endpoint exposure

API, executor, and compiler metrics are exposed through internal ClusterIP
Service ports by default. Do not route those ports through the public Ingress.
If cluster tenants are not mutually trusted, add network policy for metrics and
Tempo traffic; enabling monitoring does not create that isolation by itself.

## Troubleshooting

List the monitoring workloads:

```bash
kubectl get pods --namespace flow-like \
  -l app.kubernetes.io/component
```

Inspect component logs:

```bash
kubectl logs --namespace flow-like deployment/flow-like-prometheus
kubectl logs --namespace flow-like deployment/flow-like-grafana
kubectl logs --namespace flow-like deployment/flow-like-tempo
```

Check a metrics endpoint through its Service:

```bash
kubectl port-forward --namespace flow-like \
  service/flow-like-api 9091:9090
curl http://localhost:9091/metrics
```

Inspect the actual Grafana configuration:

```bash
kubectl get configmap flow-like-grafana-config \
  --namespace flow-like \
  --output yaml
```

If traces are missing, verify Tempo readiness and the endpoint injected into the
workload:

```bash
kubectl get pods --namespace flow-like \
  -l app.kubernetes.io/component=tempo

kubectl get deployment flow-like-api \
  --namespace flow-like \
  --output jsonpath='{.spec.template.spec.containers[0].env}'
```

## Related

- [Kubernetes Configuration](/self-hosting/kubernetes/configuration/)
- [Executor](/self-hosting/kubernetes/executor/)
- [Security](/self-hosting/kubernetes/security/)
