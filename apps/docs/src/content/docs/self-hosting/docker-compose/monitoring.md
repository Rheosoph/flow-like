---
title: Monitoring
description: Observe admission, warm capacity, storage and service health in Docker Compose
sidebar:
  order: 25
---

Enable the optional monitoring profile to collect service metrics and inspect
execution capacity. Alert delivery and host/container resource collection need
additional operator configuration.

## Enable collection

Add monitoring without removing the execution profile:

```dotenv
COMPOSE_PROFILES=per-run,monitoring
```

Then run:

```bash
python3 scripts/up.py
docker compose ps
```

Grafana binds to `http://localhost:3002` and uses the generated
`GRAFANA_ADMIN_PASSWORD`. Prometheus, Tempo, their ingestion ports and the Redis
and PostgreSQL exporters remain private.

Prometheus discovers API replicas at port 9090, compiler replicas at 9091, and
execution managers/queue bridges at 9000 through Docker DNS. Shared runtimes are
discovered when the trusted profile is active. DNS refreshes every ten seconds;
scrapes and rule evaluations run every fifteen seconds.

Inspect targets through an internal service:

```bash
docker compose exec api curl --fail http://prometheus:9090/api/v1/targets
docker compose exec api curl --fail http://execution-manager:9000/metrics
```

## Execution signals

| Metric | Interpretation |
| --- | --- |
| `executor_active_jobs`, `executor_capacity` | Occupied manager admission permits and configured limit |
| `executor_ready_sandboxes` | Unused slots available for immediate assignment |
| `executor_creating_sandboxes` | Slots being prepared |
| `executor_retiring_sandboxes` | Expired unused slots still being cleaned up |
| `executor_sandbox_creation_errors_total` | Preparation failures |
| `flow_executions_total` | Completed, failed and rejected requests |
| `executor_assignment_seconds_sum/count` | Local reservation and durable binding time |

Assignment timing excludes client transit, credential issuance, artifact loading
and the first workflow node. Measure those stages separately before setting a
startup-latency objective.

A manager can be healthy while its warm reserve is empty. Watch reserve depletion
alongside active capacity and rejection counts. Persistent preparation errors
call for host/runtime investigation; increasing queue concurrency cannot create
missing sandbox capacity.

Also track Redis memory, queue age and retained uncertain deliveries, PostgreSQL
connections, object-store latency and host resource use. The supplied profile
does not include cAdvisor or a complete host/storage metrics collector.

## Dashboards, traces and retention

Grafana provisions dashboards from `monitoring/grafana/dashboards/` and data
sources for Prometheus and Tempo. Maintain bundled dashboards in the repository;
the provisioned provider is read-only in the UI.

Tracing is disabled by an empty exporter endpoint. To enable application traces:

```dotenv
OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4317
OTEL_EXPORTER_OTLP_PROTOCOL=grpc
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1
```

Prometheus retains fifteen days by default. Review storage growth before changing
`--storage.tsdb.retention.time` in a Compose override. Named monitoring volumes
persist across container recreation; back them up if their history is required.

Logs and traces may contain workflow material. Restrict their readers and
retention. The object gateway disables access logging and request-context error
logging to avoid exposing presigned credentials.

## Alert delivery

The rules in `monitoring/prometheus/rules/alerts.yml` are evaluated, but the
configured Alertmanager list is empty. They do not send notifications by default.

Provide an Alertmanager endpoint and receivers, mount the maintained Prometheus
configuration and test a notification route. Add deployment-specific thresholds
for warm reserve depletion, preparation errors, uncertain queue entries and
datastore saturation.

If Grafana has no data, test its `http://prometheus:9090` data source, inspect
the target list and compare it with `docker compose ps`. For missing traces,
inspect `docker compose logs tempo api compiler queue-bridge` and verify the
exporter endpoint and sampling setting.
