---
title: Prerequisites
description: Prepare a Linux execution host, identity provider, networking and capacity
sidebar:
  order: 21
---

The default Compose deployment requires a Linux Docker daemon with gVisor.
Prepare that execution host before generating configuration or building images.
A client workstation may connect to a suitable remote daemon, but ordinary
Docker Desktop containers do not satisfy the required execution runtime setup.

## Host tools

Install Git, Python 3, OpenSSL, Docker Engine and Docker Compose v2. The daemon
must support Engine API 1.47, introduced in Engine 27.2; see the
[API version matrix](https://docs.docker.com/reference/api/engine/#api-version-matrix).
Compose must be at least 2.24.4 because the supplied overlays use its
[reset and override tags](https://docs.docker.com/reference/compose-file/merge/#replace-value).

Verify the tools:

```bash
docker version
docker compose version
python3 --version
openssl version
```

The setup scripts use Python on the operator's machine. The execution manager
and per-run gateway themselves are Rust binaries.

## Configure gVisor

Install `runsc` using the
[gVisor Docker instructions](https://gvisor.dev/docs/user_guide/quick_start/docker/).
Merge this runtime entry into the execution daemon's
`/etc/docker/daemon.json`, preserving its existing settings:

```json
{
  "runtimes": {
    "runsc": {
      "path": "/usr/local/bin/runsc",
      "runtimeArgs": ["--network=none", "--host-uds=open"]
    }
  }
}
```

Restart Docker after updating the configuration. The manager checks both
arguments and refuses a shared-kernel fallback. Each runner receives only its
own proxy socket volume, with a read-only mount. It receives no Docker socket
or host directory.

The trusted manager mounts the Docker socket to administer execution containers.
Restrict access to this host and its deployment credentials accordingly.

## Identity and public access

Prepare an OpenID Connect application with the correct discovery/JWKS endpoints,
client ID, callback URL and logout URL. The checked-in hub configuration contains
placeholder identity-provider settings.

Public hosting also needs DNS and an operator-managed TLS reverse proxy.
Configure HTTPS for web/API/storage and WSS for signaling. Keep the browser
origins, OIDC redirects and hub configuration consistent.

The default published listeners bind to loopback:

| Address | Service |
| --- | --- |
| `localhost:3001` | Web |
| `localhost:8080` | API |
| `localhost:4444` | Signaling |
| `s3.localhost:9000` | Object data gateway |
| `localhost:3002` | Grafana, when monitoring is enabled |

PostgreSQL, Redis, the compiler, execution managers, RustFS administration and
metrics listeners are private. The `s3.localhost` name must resolve to the
gateway from both clients and containers. Add a hosts entry on clients whose
resolver does not recognize `.localhost`.

## Capacity and outbound access

Budget active executions plus unused warm slots. The defaults allow ten active
executions and two additional warm slots per manager. Each slot has a runner
limited to 1 GiB and one CPU, plus a gateway limited to 128 MiB and half a CPU.
Leave room for the API, compiler, datastores, storage, monitoring and image builds.

Select smaller counts before installation when the host cannot support those
limits. See [Scaling](/self-hosting/docker-compose/scaling/) for the capacity
calculation.

The host needs registry/package access for builds and access to your identity
provider. Workflows can reach only the destinations permitted by the execution
gateway. Plan explicit HTTPS integration grants; raw TCP/UDP clients cannot use
the default sandbox network.

RustFS is included, so a new installation does not require external object
storage. Existing installations should retain their current store until a
separately verified data migration is complete.
