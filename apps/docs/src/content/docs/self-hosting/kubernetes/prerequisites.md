---
title: Prerequisites
description: Tools, cluster capabilities, images, and external services required by the Helm chart.
sidebar:
  order: 11
---

## Required Tools

For any cluster:

- Helm 3
- A compatible `kubectl`
- Access to the Flow-Like repository or a packaged copy of
  `apps/backend/kubernetes/helm/`
- OpenSSL when generating the ES256 execution-key pair with the repository
  helper

Local k3d development also requires Docker and k3d.

## Application Images

Your cluster must be able to pull these Flow-Like images:

- API
- Web application
- Executor, used by the working warm HTTP pool; the checked-in one-job
  entrypoint for per-run Jobs is not implemented
- Database migration

The optional compiler and sink services need their own images when enabled.

The chart defaults point to a local k3d registry with
`imagePullPolicy: Never`; they are not production image settings. Publish the
images to an accessible registry and configure `global.imagePullSecrets` when
authentication is required.

## Object Storage

Object storage is external to the chart. Select one provider:

- AWS S3
- Azure Blob Storage
- Google Cloud Storage
- Cloudflare R2
- A generic S3-compatible service such as MinIO

Create the meta, content, and logs buckets or containers before installation.
You need provider credentials that can access them. Some execution modes also
require the provider-specific ability to issue scoped runtime credentials.

## Database

Choose one of:

- The chart's single-node, insecure CockroachDB workload for evaluation or
  development
- An externally operated PostgreSQL or CockroachDB service for production

External mode requires a `DATABASE_URL` Secret and network access from the
release namespace.

## Cluster Capabilities

At minimum, provide:

- A Kubernetes cluster that supports `apps/v1`, `batch/v1`,
  `networking.k8s.io/v1`, and persistent-volume claims
- A default `StorageClass`, or explicit storage classes for CockroachDB,
  Redis, Prometheus, and Grafana persistence
- DNS and egress access to the selected object store, database, identity
  provider, and model APIs
- Sufficient CPU, memory, and storage for the enabled workloads

The default chart also enables Prometheus, Grafana, and Tempo. Disable
`monitoring.enabled` for a smaller evaluation install, or size their
persistence and resources deliberately.

## Optional Capabilities

- An Ingress controller and TLS certificate workflow when
  `ingress.enabled: true`
- Metrics APIs for HPA when autoscaling is enabled
- Prometheus Operator CRDs when `monitoring.serviceMonitor.enabled: true`
- Kata Containers installed on cluster nodes before referencing a Kata
  `RuntimeClass` from a separately completed Job runner

Creating a `RuntimeClass` object does not install its runtime handler.

## Secrets to Prepare

Plan for:

- `BACKEND_KEY`, `BACKEND_PUB`, and `BACKEND_KID`
- Provider-specific storage credentials and logical bucket names
- `DATABASE_URL` for an external database
- `REDIS_PASSWORD` when supplying Redis authentication through an existing
  Secret
- Optional registry, LLM-provider, ingress TLS, and observability credentials

Do not pass long-lived secrets as Helm `--set` values. The
[Installation](/self-hosting/kubernetes/installation/) guide uses local
environment files and pre-created Kubernetes Secrets instead.
