---
title: Monitoring
description: Run the bundled Prometheus, Grafana, Tempo, and exporter services
sidebar:
  order: 25
---

The optional `monitoring` profile adds metrics, dashboards, distributed
tracing, and database exporters to the same Compose network.

## Start the profile

```bash
docker compose --profile monitoring up -d
docker compose --profile monitoring ps
```

The profile starts:

| Service | Template host port | Purpose |
| --- | --- | --- |
| Grafana | `3002` | Provisioned dashboards and trace exploration |
| Prometheus | `9091` | Metrics storage, rules, and queries |
| Tempo | `3200` | Trace storage and query API |
| Tempo OTLP gRPC | `4317` | Trace ingestion |
| Tempo OTLP HTTP | `4318` | Trace ingestion |
| Redis exporter | `9121` | Redis metrics |
| PostgreSQL exporter | `9187` | PostgreSQL metrics |

The compiler metrics listener is separately published on `9092` by default.
API and runtime metrics stay on the Compose network.

Change `GRAFANA_ADMIN_PASSWORD` before making Grafana reachable outside a
trusted development machine.

## What Prometheus scrapes

The checked-in configuration at
`monitoring/prometheus/prometheus.yml` defines these targets:

| Job | Internal target | Metrics path |
| --- | --- | --- |
| Prometheus | `localhost:9090` | `/metrics` |
| Flow-Like API | `api:9090` | `/metrics` |
| Runtime | `runtime:9000` | `/metrics` |
| WASM compiler | `compiler:9091` | `/metrics` |
| Redis | `redis-exporter:9121` | `/metrics` |
| PostgreSQL | `postgres-exporter:9187` | `/metrics` |

There is no cAdvisor service in the current profile, so the bundled stack does
not collect per-container CPU, memory, or network metrics by default.

Check target health:

```bash
curl --fail http://localhost:9091/api/v1/targets
```

Check the service listeners directly:

```bash
docker compose exec api curl --fail http://localhost:9090/metrics
docker compose exec runtime curl --fail http://localhost:9000/metrics
curl --fail http://localhost:9092/metrics
```

## Provisioned dashboards

Grafana loads the JSON dashboards in
`monitoring/grafana/dashboards/` into a **Flow-Like** folder:

- system overview;
- API;
- execution runtime;
- WASM compiler;
- PostgreSQL;
- Redis;
- distributed tracing.

The provisioned provider is read-only in the UI. To maintain another bundled
dashboard, add its JSON file to that directory and restart Grafana:

```bash
docker compose restart grafana
```

For dashboards managed independently of the repository, configure another
Grafana provider or use an external Grafana instance.

## Metrics and trace configuration

Useful environment variables include:

```dotenv
PROMETHEUS_PORT=9091
GRAFANA_PORT=3002
GRAFANA_ADMIN_USER=admin
GRAFANA_ADMIN_PASSWORD=<strong-password>

TEMPO_HTTP_PORT=3200
TEMPO_OTLP_GRPC_PORT=4317
TEMPO_OTLP_HTTP_PORT=4318

OTEL_EXPORTER_OTLP_ENDPOINT=http://tempo:4317
OTEL_EXPORTER_OTLP_PROTOCOL=grpc
OTEL_TRACES_SAMPLER=parentbased_traceidratio
OTEL_TRACES_SAMPLER_ARG=0.1
```

The API, runtime, and compiler use their own `OTEL_SERVICE_NAME` values in the
Compose file. Grafana provisions both Prometheus and Tempo data sources.

Prometheus retention is currently fixed to `15d` in
`docker-compose.yml`. Change it with a small override:

```yaml
services:
  prometheus:
    command:
      - --config.file=/etc/prometheus/prometheus.yml
      - --storage.tsdb.path=/prometheus
      - --storage.tsdb.retention.time=30d
      - --web.enable-lifecycle
```

Prometheus, Grafana, and Tempo already use named volumes. Back up those volumes
if their history or dashboard state is important.

## Recording and alert rules

`monitoring/prometheus/rules/alerts.yml` contains API, runtime, Redis, and
PostgreSQL alerts plus recording rules for request and execution metrics.
Prometheus loads and evaluates the file automatically.

The bundled Prometheus configuration has an empty Alertmanager list. Alerts
therefore appear in Prometheus but are not delivered to email, chat, or an
incident-management system. To route alerts:

1. run an Alertmanager service or use a managed endpoint;
2. add it under `alerting.alertmanagers` in a maintained Prometheus
   configuration;
3. mount that configuration with a Compose override;
4. validate the target and a test route before relying on it.

Do not assume that the presence of an alert rule means notifications are
configured.

## Troubleshooting

### A target is down

```bash
docker compose --profile monitoring ps
docker compose logs prometheus
docker compose logs api runtime compiler
```

Prometheus targets use internal ports and service names. Do not substitute
host-published ports inside `prometheus.yml`.

### Grafana has no data

Open **Connections → Data sources** and test the provisioned Prometheus URL:
`http://prometheus:9090`. Then inspect the Prometheus targets page to determine
whether collection or visualization is failing.

### Traces are missing

Check Tempo health and the application exporters:

```bash
curl --fail http://localhost:3200/ready
docker compose logs tempo api runtime compiler
```

Confirm that `OTEL_EXPORTER_OTLP_ENDPOINT` resolves to `http://tempo:4317`
inside the Compose network and that the sampling ratio is greater than zero.
