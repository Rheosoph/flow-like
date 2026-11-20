---
title: Query service
description: Run, size and secure the service that executes every data read for the API
sidebar:
  order: 57
---

The query service runs the data reads of a hub: SQL in Data Studio, table
browsing, graph reads, forecasts and the SQL of Home data widgets. The API stays
in charge of access. For each request it checks the caller's permissions, mints
short-lived read-only storage credentials for exactly the databases involved,
and signs the request with its backend key. The service verifies that signature
with `BACKEND_PUB`, reads the data, and streams the result back. It stores
nothing and holds no storage permissions of its own.

Running reads in a separate service keeps large joins, sorts and forecasts from
using the API's memory, and lets you size the service for them.

## Connect the API

Set one of these on the API:

| Variable | Use |
| --- | --- |
| `QUERY_SERVICE_URL` | Base URL of the query container, for example `http://query:8090`. Docker Compose, Kubernetes, Azure and GCP use this. |
| `QUERY_LAMBDA_FUNCTION` | Name of the query Lambda function on AWS. The API needs `lambda:InvokeFunction` on it. |
| `QUERY_AUTH` | `gcp_id_token` when the service runs on Cloud Run with IAM invokers. The API then sends a Google ID token for `QUERY_SERVICE_URL`. |

If neither `QUERY_SERVICE_URL` nor `QUERY_LAMBDA_FUNCTION` is set, data routes
answer `503` with the code `QUERY_SERVICE_UNAVAILABLE`. The native local backend
runs the service inside the API process instead.

Keep the service on an internal network. Only the API should reach it.

## Configure the service

| Variable | Default | Meaning |
| --- | --- | --- |
| `BACKEND_PUB` | required | Public key matching the API's `BACKEND_KEY`. Requests signed with another key are refused. |
| `PORT` | `8090` | Listen port of the container. |
| `QUERY_MAX_ROWS` | `10000` | Most rows one request may return. Requests asking for more are lowered, with a warning. |
| `QUERY_MAX_TIMEOUT_MS` | `50000` | Longest time one request may run. |
| `QUERY_MEMORY_BYTES` | 60 % of the Lambda memory, else 2 GiB | Memory for sorts, joins and aggregates before they spill to disk. |
| `QUERY_SPILL_DIR` | `<temp dir>/flow-like-query-spill` | Directory for spill files. |
| `QUERY_SPILL_MAX_BYTES` | 400 MiB on Lambda, else 4 GiB | Most spill space; `0` turns spilling off. |
| `QUERY_MODEL_CACHE_DIR` | `<temp dir>/flow-like-query-models` | Where forecasting checkpoints are cached after their first download. |
| `QUERY_MODEL_CACHE_MAX_BYTES` | 1 GiB on Lambda, else 4 GiB | Largest size of the checkpoint cache. |
| `QUERY_ALLOW_PRIVATE_NETWORKS` | `false` | `true` lets external databases on private, shared or reserved addresses be reached. |

Set `QUERY_MEMORY_BYTES` to about 60 % of the container's memory limit, leaving
room for Arrow buffers and the result being sent. Put `QUERY_SPILL_DIR` and
`QUERY_MODEL_CACHE_DIR` on disk-backed volumes. A `tmpfs` counts against the
container's memory limit, so spilling to it adds memory pressure instead of
relieving it.

## Deployment targets

- **Docker Compose** runs the `query` service. Spill files go to an anonymous
  volume and checkpoints to the `query_models` volume. Size it with
  `QUERY_CPUS`, `QUERY_MEMORY` and `QUERY_MEMORY_BYTES` in `.env`.
- **Kubernetes** runs it from the Helm values under `query`: resources,
  `memoryBytes`, `spillMaxBytes` and `spillSizeLimit`, and `modelCacheMaxBytes`
  and `modelCacheSizeLimit`. Spill and checkpoints use `emptyDir` volumes. With
  `networkPolicy.enabled`, add rules for external databases to
  `query.extraEgress`.
- **AWS** runs it as a Lambda function with response streaming, invoked only by
  the API. `/tmp` holds spill files and checkpoints, so give the function enough
  ephemeral storage for both.
- **Azure Container Apps** and **GCP Cloud Run** run the container with internal
  ingress. On Cloud Run, make the API's service account the only invoker and
  set `QUERY_AUTH=gcp_id_token` on the API. Cloud Run's file system is
  memory-backed, so spill and checkpoint space count against the instance's
  memory.

## External databases

[External databases](/apps/data-studio/#external-databases) are read by this
service. The API decrypts the stored credentials and sends them with the
request that needs them; the service connects, runs the read and keeps nothing.

The service refuses addresses on private networks unless
`QUERY_ALLOW_PRIVATE_NETWORKS=true`. ClickHouse is reached over plain HTTP, so
the service only connects to it with that setting; keep such databases on a
private network. Self-hosted databases usually live on such
networks, so the Docker Compose and Helm defaults enable it. The service's
network must also allow the database traffic itself. For example, a Kubernetes
NetworkPolicy needs a matching `query.extraEgress` rule, and an Azure
environment without a default internet route needs firewall network rules for
the database ports.
