---
title: Desktop Client Configuration
description: Connect the Flow-Like desktop app to your self-hosted backend.
sidebar:
  order: 56
---

After deploying your self-hosted Flow-Like backend, you need to configure the desktop application to connect to it instead of the default cloud service.

## Environment Variable

Set the `FLOW_LIKE_API_URL` environment variable before launching the desktop app:

```bash
# Linux / macOS
export FLOW_LIKE_API_URL=https://your-api.example.com
./flow-like

# Windows (PowerShell)
$env:FLOW_LIKE_API_URL = "https://your-api.example.com"
.\flow-like.exe

# Windows (Command Prompt)
set FLOW_LIKE_API_URL=https://your-api.example.com
flow-like.exe
```

The desktop uses the value exactly as provided. Do not define
`FLOW_LIKE_API_URL` as an empty string; an explicitly empty value overrides the
build-time and hosted defaults and is a configuration error.

## Configuration Options

| Variable | Description | Example |
|----------|-------------|---------|
| `FLOW_LIKE_API_URL` | Full URL to your self-hosted API | `https://api.flow-like.internal` |

The URL should point to your API service:

- **Docker Compose**: `http://localhost:8080` or your exposed endpoint
- **Kubernetes**: The external URL of your `api` service (e.g., via Ingress)

## Build-time Configuration

For custom builds, you can also set the default backend at compile time:

| Variable | Description |
|----------|-------------|
| `FLOW_LIKE_CONFIG_DOMAIN` | Domain without protocol (e.g., `api.example.com`) |
| `FLOW_LIKE_CONFIG_SECURE` | Use HTTPS (`true` or `false`, defaults to `true`) |

These are set via `option_env!()` during the Rust build process.

## Verification

After configuring, verify the connection:

1. Launch the desktop app
2. Check the settings panel for the connected backend URL
3. Try logging in or creating a local app

## Troubleshooting

### Connection refused

Ensure your API is reachable from the desktop machine:

```bash
# Docker Compose public gateway
curl -v https://your-compose.example.com/health

# Kubernetes API / Ingress
curl -v https://your-kubernetes-api.example.com/health/live
```

### SSL/TLS errors

If using self-signed certificates, you may need to add them to your system's trust store or use HTTP during development.

### Mixed content

If your API uses HTTP but the app expects HTTPS, ensure your `FLOW_LIKE_API_URL` includes the correct protocol.

## Related

- [Docker Compose Deployment](/self-hosting/docker-compose/overview/)
- [Kubernetes Deployment](/self-hosting/kubernetes/overview/)
- [Execution Backends](/self-hosting/execution-backends/)
