---
title: Execution Backends
description: Understanding job isolation and choosing the right execution backend.
sidebar:
  order: 55
---

Flow-Like supports multiple execution backends, each with different isolation guarantees, performance characteristics, and use cases.

## Backend Overview

| Backend | Isolation Level | Latency | Best For |
|---------|-----------------|---------|----------|
| HTTP → Warm Pool | Process | Low | Trusted workloads, low latency |
| HTTP → Lambda | MicroVM (Firecracker) | Medium | Multi-tenant SaaS |
| Lambda SDK Invoke | MicroVM (Firecracker) | Medium | Fire-and-forget batch |
| Lambda SDK Stream | MicroVM (Firecracker) | Medium | Streaming from private Lambdas |
| Kubernetes Job | Pod | High | Untrusted code, compliance |
| Docker Compose | Container | Low | Development, small deployments |

## Isolation & Security Model

### AWS Lambda (Strongest Isolation)

AWS Lambda provides **hardware-level isolation** via [Firecracker microVMs](https://firecracker-microvm.github.io/):

- Each execution runs in its own microVM with hardware-level isolation
- Memory is wiped between invocations from different tenants
- No shared filesystem between executions
- Cold starts create fresh environments
- Warm starts reuse the same microVM for the **same function** only (not shared across tenants)

**Invocation methods:**

| Method | Description | Use Case |
|--------|-------------|----------|
| HTTP (Function URL) | HTTP POST to Lambda Function URL | Streaming responses, simple setup |
| Lambda SDK Invoke | Async invocation via AWS SDK | Fire-and-forget batch jobs |
| Lambda SDK Stream | Streaming invocation via AWS SDK | Streaming from private Lambdas |

**Best for:** Multi-tenant SaaS, untrusted workloads, pay-per-use pricing.

### Kubernetes Warm Pool (HTTP → Deployment)

A pool of long-running executor pods handles requests:

```
┌─────────────────────────────────────────────────────┐
│                  Kubernetes Cluster                  │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐             │
│  │Executor │  │Executor │  │Executor │  ← Warm Pool │
│  │  Pod 1  │  │  Pod 2  │  │  Pod 3  │             │
│  └────┬────┘  └────┬────┘  └────┬────┘             │
│       │            │            │                   │
│       └────────────┼────────────┘                   │
│                    │                                 │
│              ┌─────┴─────┐                          │
│              │  Service  │ ← Load balanced          │
│              └───────────┘                          │
└─────────────────────────────────────────────────────┘
```

**Characteristics:**

- **Process-level isolation**: Each request runs in the same pod but can use separate processes
- **Shared resources**: Pods handle multiple requests over their lifetime
- **Faster response**: No cold start - pods are already running
- **Cost efficient**: Fewer pod creations, better resource utilization

**Security considerations:**

Requests from different users may run on the same pod. This is suitable when:

- Tenants are trusted (same organization)
- Execution code is sandboxed (e.g., WASM, containers within pods)
- Performance is prioritized over strict isolation

**Best for:** Internal/trusted workloads, low-latency requirements, cost optimization.

### Kubernetes Isolated Job (Strongest K8s Isolation)

Each execution creates a dedicated Kubernetes Job:

```
┌─────────────────────────────────────────────────────┐
│                  Kubernetes Cluster                  │
│                                                      │
│  Request 1 → ┌─────────┐                            │
│              │  Job 1  │ ← Fresh pod                │
│              │  Pod    │                            │
│              └─────────┘                            │
│                                                      │
│  Request 2 → ┌─────────┐                            │
│              │  Job 2  │ ← Fresh pod                │
│              │  Pod    │                            │
│              └─────────┘                            │
│                                                      │
│  Request 3 → ┌─────────┐                            │
│              │  Job 3  │ ← Fresh pod                │
│              │  Pod    │                            │
│              └─────────┘                            │
└─────────────────────────────────────────────────────┘
```

**Characteristics:**

- **Pod-level isolation**: Fresh pod for every execution
- **Resource guarantees**: Dedicated CPU/memory per job
- **Clean environment**: No state leakage between executions
- **Network policies**: Can apply per-job network restrictions
- **Kata Containers**: Optional hardware-level isolation via `RuntimeClass`
- **Slower startup**: Pod scheduling + image pull overhead (mitigated with pre-pulled images)

**Best for:** Untrusted code execution, strict compliance requirements, resource-intensive workloads.

### Docker Compose (Development)

For local development and small deployments:

- **Container-level isolation**: Each executor is a separate container
- **Shared host resources**: Containers share the Docker host
- **Simpler setup**: No orchestration complexity

**Best for:** Development, testing, small-scale deployments.

## Choosing a Backend

### Decision Matrix

| Requirement | Recommended Backend |
|-------------|---------------------|
| Multi-tenant SaaS | Lambda (strongest isolation) |
| Low latency | HTTP → Warm Pool (K8s/Lambda) |
| Untrusted code | Kubernetes Job or Lambda |
| Batch processing | Lambda SDK Invoke (fire-and-forget) |
| Streaming response | HTTP or Lambda SDK Stream |
| Cost optimization | HTTP → Warm Pool |
| Compliance/audit | Kubernetes Job (per-job logging) |
| Development | Docker Compose |

### Latency Comparison

| Backend | Cold Start | Warm Request |
|---------|------------|--------------|
| Warm Pool (K8s) | N/A (always warm) | ~10-50ms |
| Lambda | ~100-500ms | ~10-50ms |
| Kubernetes Job | ~2-10s | N/A (always cold) |

### Cost Comparison

| Backend | Idle Cost | Per-Request Cost |
|---------|-----------|------------------|
| Warm Pool | High (running pods) | Low |
| Lambda | None | Medium (per-ms billing) |
| Kubernetes Job | Low (no idle pods) | High (pod overhead) |

## Configuration

### Environment Variables

```bash
# Backend selection for streaming/sync requests (/invoke endpoints)
EXECUTION_BACKEND=http              # http, lambda_invoke, lambda_stream, kubernetes_job

# Backend selection for async requests (/invoke/async endpoints)
ASYNC_EXECUTION_BACKEND=redis       # http, redis, sqs, kafka

# HTTP backend (Warm Pool, Lambda Function URL, Azure, GCP)
EXECUTOR_URL=https://executor.example.com

# Lambda backends
LAMBDA_EXECUTOR_FUNCTION=arn:aws:lambda:us-east-1:123456789:function:executor
AWS_REGION=us-east-1

# Kubernetes Job backend
K8S_NAMESPACE=flow-like
K8S_EXECUTOR_IMAGE=ghcr.io/rheosoph/flow-like-executor:latest
```

### Runtime Selection

You can override the backend per-request via the API:

```json
POST /apps/{app_id}/events/{event_id}/invoke
{
  "payload": { ... },
  "mode": "kubernetes_job",
  "backend_config": {
    "executor_url": "https://custom-executor.example.com"
  }
}
```

Available modes:
- `local` - Track only, no execution
- `http` - HTTP POST to executor
- `lambda_invoke` - AWS Lambda async invoke
- `lambda_stream` - AWS Lambda streaming invoke
- `kubernetes_job` - Isolated K8s Job

## Security Recommendations

### Multi-Tenant SaaS

Use **Lambda** or **Kubernetes Isolated Jobs** with Kata Containers:

```yaml
# Kubernetes Job with Kata runtime
spec:
  template:
    spec:
      runtimeClassName: kata-qemu  # Hardware isolation
```

### Internal/Trusted Workloads

Use **Warm Pool** for best performance:

```yaml
# Kubernetes Deployment for warm pool
apiVersion: apps/v1
kind: Deployment
metadata:
  name: executor-pool
spec:
  replicas: 3
  template:
    spec:
      containers:
      - name: executor
        resources:
          requests:
            memory: "512Mi"
            cpu: "250m"
```

### Network Isolation

Apply network policies for Kubernetes backends:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: executor-isolation
spec:
  podSelector:
    matchLabels:
      app: executor
  policyTypes:
  - Egress
  egress:
  - to:
    - namespaceSelector:
        matchLabels:
          name: flow-like
```
