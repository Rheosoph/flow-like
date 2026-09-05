---
title: Monitoring
description: Operate the Prometheus, Grafana, and Tempo resources in the Helm chart
sidebar:
  order: 35
---

Monitor clean execution capacity, queue retention and data-service health alongside
API request metrics. A ready API does not establish that an initialized execution
slot is available.

The chart enables Prometheus, Grafana and Tempo by default. Set
`monitoring.enabled=false` in your normal values file to disable them, or
`monitoring.tracing.enabled=false` to omit Tempo while keeping metrics.

## Access the monitoring services

Examples assume release and namespace `flow-like`.

| Service | Local access |
| --- | --- |
| Prometheus | `kubectl port-forward -n flow-like svc/flow-like-prometheus 9090:9090` |
| Grafana | `kubectl port-forward -n flow-like svc/flow-like-grafana 3000:80` |
| API metrics | `kubectl port-forward -n flow-like svc/flow-like-api 9091:9090` |
| Manager metrics | `kubectl port-forward -n flow-like svc/flow-like-execution-manager 9000:9000` |

Open Prometheus at `http://localhost:9090/targets` and verify expected targets.
API and manager metrics are available at `/metrics` on their forwarded ports.
Keep these endpoints private.

Grafana's admin user defaults to `admin`. Its password is stored in the
`flow-like-grafana` Secret under `admin-password`. Retrieve it through your
operator credential workflow; do not include it in shared logs or screenshots.

## Execution-manager metrics

| Metric | Meaning |
| --- | --- |
| `executor_active_jobs` | Active requests held by the supervisor |
| `executor_capacity` | Configured active request capacity |
| `executor_warm_slots` | Clean initialized slots available |
| `executor_warm_target` | Configured warm reserve |
| `executor_warm_initializing` | Slots being prepared |
| `executor_warm_failures_total` | Failed preparations |
| `executor_warm_retiring` | Expired slots awaiting cleanup |
| `executor_assignment_seconds_sum`, `_count` | Time through run binding, gateway configuration and runner acknowledgement |

Assignment counters do not measure browser-to-first-node latency or provide
percentiles. Instrument representative end-to-end runs to establish start-time
p95/p99, queue wait and sustainable refill rate.

Investigate a falling reserve before increasing consumer concurrency. Check
runner scheduling, image availability, gVisor startup, gateway probes, CPU,
memory and Pod creation pressure. A healthy manager with an empty reserve refuses
new immediate work.

## Current scrape coverage

The built-in Prometheus discovers API metrics on the Service's `metrics` port,
the trusted executor pool, optional compiler, internal CockroachDB, Kubernetes
API/node/cAdvisor metrics and annotated Pods in the release namespace.

The native execution-manager Deployment currently has no scrape annotation and
no dedicated built-in scrape job. Add a manager scrape to your operated Prometheus
configuration. For an external Prometheus Operator installation, this
ServiceMonitor selects the manager:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: flow-like-execution-manager
  namespace: flow-like
spec:
  namespaceSelector:
    matchNames: [flow-like]
  selector:
    matchLabels:
      app.kubernetes.io/instance: flow-like
      app.kubernetes.io/component: execution-manager
  endpoints:
    - port: http
      path: /metrics
      interval: 15s
      scrapeTimeout: 10s
```

The Operator CRD and a Prometheus instance selecting this ServiceMonitor must
already exist. Permit that scraper through the deployment's network policies.

The chart's optional `monitoring.serviceMonitor.enabled` resource currently
selects API port `http` at `/metrics`; the API metrics listener is on the
separate Service port named `metrics`. Use a corrected operator-managed
ServiceMonitor for the API until that template is changed.

## Dashboard and alert boundaries

Grafana provisions Prometheus and, when enabled, Tempo data sources and
dashboards. Inspect their queries against the series present in your installation.
The existing executor-pool exhaustion alert references
`flow_executor_pool_available`, which is not the native manager's warm-slot
metric. Use `executor_warm_slots` for clean capacity alerts.

The chart does not deploy Redis exporter or kube-state-metrics. Rules requiring
their metrics need separately installed and scraped exporters.
`monitoring.prometheusRule.rules` does not currently create a PrometheusRule
resource. `monitoring.alertmanager.enabled` configures a target without
deploying Alertmanager. Provide those resources explicitly before relying on
notification delivery.

Track Redis memory, queue age, quarantined jobs and retained replay claims.
Bundled Redis uses `noeviction`; a full instance rejects writes rather than
silently discarding execution records.

## Persistence and tracing

```yaml
monitoring:
  prometheus:
    scrapeInterval: 15s
    evaluationInterval: 15s
    retention: 15d
    persistence:
      enabled: true
      size: 50Gi
  grafana:
    persistence:
      enabled: true
      size: 10Gi
  tracing:
    enabled: true
    retention: 72h
  tempo:
    persistence:
      enabled: false
      size: 10Gi
```

Tempo receives OTLP on ports 4317 and 4318. With persistence disabled, traces
are lost when its Pod is replaced. Size monitoring storage and retention
independently of application object storage.

## Troubleshoot a missing signal

```bash
kubectl logs deployment/flow-like-prometheus -n flow-like --tail=100
kubectl logs deployment/flow-like-grafana -n flow-like --tail=100
kubectl logs deployment/flow-like-execution-manager -n flow-like --tail=100
kubectl get endpointslice -n flow-like
kubectl get networkpolicy -n flow-like
```

First confirm that the endpoint emits the metric, then check Service port
selection, discovery labels and network access from the scraper. For missing
traces, verify Tempo readiness and the workload's configured OTLP endpoint.
