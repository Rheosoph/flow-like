Deploy and verify a new synthetic practice instance on Linux. Prepare Docker Engine with API 1.47 or later, Compose 2.24.4 or later, Python 3, OpenSSL, Git, OIDC and capacity for the services/execution slots before the timed lesson.

The default Compose mode is **per_run**. Its manager assigns a single-use gVisor sandbox and gateway to each execution. Warm slots are prepared for future requests; they are not shared multi-run workers. **trusted_shared** is a separate mode.

RustFS object storage is bundled. External storage is an explicit configuration choice with its own credential and endpoint requirements. Kubernetes also defaults to per-run managers; the orchestrator's name alone establishes no isolation guarantee.

Complete the [Linux/gVisor prerequisites](https://docs.flow-like.com/self-hosting/docker-compose/prerequisites/), including required `runsc` runtime arguments. Ordinary Docker Desktop containers do not satisfy the default host setup.

Completion: record tool versions, selected mode, capacity and OIDC callback origin. This core uses a new disposable instance. Existing deployments follow the documented upgrade path, preserving data, configuration and signing keys.
