---
title: Prerequisites
description: Tools, cluster capabilities, images, and external services required by the Helm chart.
sidebar:
  order: 11
---

Prepare the cluster's execution boundary and storage before deploying the chart.
The default `per_run` mode requires Linux execution nodes with gVisor and Cilium.

## Build and operator tools

Install Docker with BuildKit, Helm 3, a compatible `kubectl`, Python 3 and
OpenSSL. Python runs configuration and deployment helpers; execution supervision,
gateway enforcement and the slot adapter run in Rust.

The build machine needs the Flow-Like repository and permission to push images
to a registry reachable by the cluster. Configure `global.imagePullSecrets`
when the registry requires authentication. Isolated execution requires immutable
manager and executor image digests; the image helper records them after a push.

## Execution nodes and Cilium

Provide:

- A working `runsc` runtime handler on eligible Linux nodes.
- A matching Kubernetes RuntimeClass, normally named `runsc`.
- Cilium with policy enforcement enabled, Kubernetes NetworkPolicy support and
  `allow-localhost=policy`.
- A completed Cilium DaemonSet rollout and the CiliumNetworkPolicy CRD.
- CPU and memory for active executions plus the warm reserve and gateway Pods.

Creating a RuntimeClass object only names an installed handler. The Helm chart
does not install gVisor or configure the container runtime.

Standard Kubernetes NetworkPolicy permits traffic to the local node. The chart
therefore requires Cilium deny rules for node, Kubernetes API and metadata
destinations. The deploy helper checks Cilium configuration. Each warm slot also
checks a reachable gateway and prohibited endpoints before accepting tenant
input. These checks supplement live isolation tests; they do not cover every
possible network path.

See [Security](/self-hosting/kubernetes/security/) for namespace and policy
ownership requirements. A local k3d cluster does not provide this boundary;
its helper requires explicit `trusted_shared` mode.

## Persistent services

A default StorageClass, or explicit storage classes, must satisfy the enabled
RustFS, Redis, database and monitoring PVCs. Size them for retained objects,
execution state, queue claims and metrics.

RustFS and its private metadata, content and log buckets are provisioned by the
chart. An external object store is optional. Any replacement used by isolated
execution must support the required prefix-scoped temporary credentials and a
bucket-only public data endpoint.

The internal CockroachDB workload is a single-node, insecure evaluation database.
For production, prepare an externally operated PostgreSQL or CockroachDB service
and a complete `DATABASE_URL`. External Redis is supported through a Secret
containing an authenticated `REDIS_URL`.

## Public endpoints and identity

Choose the browser origins for the web application, API and S3 object gateway.
The S3 origin must resolve and be reachable from both browsers and Pods; presigned
URLs are bound to that exact host and path.

Prepare an ingress controller and certificates for public HTTPS endpoints.
Configure the API's public hub and OIDC settings in the JSON file selected by
`FLOW_LIKE_CONFIG` before building the API. Those settings are embedded at build
time, while credentials belong in Kubernetes Secrets.

Optional features may require Metrics Server for HPAs, Prometheus Operator CRDs
for ServiceMonitor resources, and additional network rules for external database,
Redis, DNS or private integration endpoints.

Continue with [Installation](/self-hosting/kubernetes/installation/). The setup
helper generates matching execution keys and separate service credentials.
