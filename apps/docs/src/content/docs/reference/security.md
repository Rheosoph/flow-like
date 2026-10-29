---
title: Security Architecture
description: Current trust boundaries, sandbox controls, authentication, authorization, and self-hosting responsibilities
sidebar:
  order: 0
  badge:
    text: Important
    variant: caution
---

Flow-Like combines a Rust application core, a Wasmtime boundary for external
WASM nodes, authenticated APIs, app-scoped authorization, and deployment-level
controls. These layers reduce risk, but they do not make an untrusted workflow
or an incorrectly exposed deployment safe by themselves.

![A third-party node inside the WASM boundary, with only approved paths to network, scoped storage, and configured models](../../../assets/WasmSandbox.webp)

## Security boundaries

Standalone devices have a separate management and workload boundary. See
[Device security and networking](/devices/security/) for outbound remote
access, authenticated encryption, scoped sharing, local secrets, and the
limits of device isolation.

| Boundary | Enforced by | Operator or author responsibility |
| --- | --- | --- |
| External WASM node | Wasmtime memory isolation, fuel, epoch interruption, resource limits, capability-aware host functions | Review requested permissions and package provenance |
| Self-hosted workflow in `per_run` mode | Single-use gVisor sandbox, external Rust egress gateway, signed dispatch and confirmed termination | Install and qualify the runtime and network boundary; size active and warm capacity |
| Workflow and app data | Scoped storage paths, runtime credentials, app roles | Configure storage and identities with least privilege |
| Public API | OIDC, API keys, PATs, internal JWTs, route-level permission checks | Protect credentials and expose the API only through an appropriate network boundary |
| Service-to-service calls | Audience-specific ES256 backend JWTs | Keep the private signing key on authorized signers; distribute the public verification key to workers and coordinate rotation |
| Network transport | Reverse proxy, load balancer, ingress, and network policy | Configure TLS, trusted forwarding, egress, and internal segmentation |
| Third-party services | Explicit network/model/OAuth use by workflows | Understand what data leaves the deployment and which provider receives it |

## Self-hosted workflow isolation

Compose and Kubernetes default to `per_run`. A Rust manager reserves a clean,
prepared environment, supplies one signed dispatch and destroys the environment
after execution. Tenant code and credentials enter only after reservation.
Failed or cancelled executions never return their environment to the warm pool.
The runner verifies the complete dispatch binding before loading executable code.

Compose runners have no external network interface and access their gateway
through a private Unix socket. Kubernetes runners can reach only their paired
gateway Pod; mandatory Cilium policies deny node, Kubernetes API and metadata
access. The gateway runs outside the tenant sandbox and enforces one immutable
callback/storage/integration policy. Ignoring the proxy settings does not grant
direct external connectivity in the qualified deployment.

The manager is trusted control-plane code. Compose's Docker socket grants
control of its execution daemon; the Kubernetes manager has namespace-scoped
Pod, policy and cancellation permissions. Neither authority reaches workflow
Pods. Restrict who can change execution images, labels, policies and manager
configuration, because those changes can alter the isolation boundary.

Cancellation records a shared marker and confirms termination before reporting
success. Unconfirmed cleanup closes admission and retains state for
reconciliation. External side effects and exported STS credentials can outlive
the sandbox. Shared workers selected through `trusted_shared` do not provide
this per-execution boundary.

## WASM execution

Flow-Like uses [Wasmtime](https://wasmtime.dev/) for external WASM nodes. Core
modules and Component Model binaries use different ABI adapters, then converge
on the same Flow-Like capability and resource boundary.

### Isolation and interruption

- Each instance has WebAssembly linear memory separate from host memory.
- WASI is disabled by the normal security configurations unless explicitly
  enabled.
- Host process environment variables are never inherited by WASM guests,
  including permissive and runtime execution profiles. Any guest environment
  value must be supplied individually through explicit guest configuration.
- Fuel metering limits instruction execution.
- Epoch interruption enforces a wall-clock deadline.
- Memory, tables, memories, table elements, instances, and stack depth are
  bounded by the selected `WasmLimits`.
- Host functions check the granted capabilities before performing protected
  work.

These controls limit a module's direct authority. They do not prove that a
permitted HTTP request is trustworthy, that returned data is safe to use, or
that a high resource limit cannot affect overall capacity.

### Resource profiles

There are two related configuration paths:

1. A package manifest selects memory and timeout tiers. The desktop loader for
   installed packages applies them to every node it loads from the package.
2. Node-level permissions can be converted directly into a security
   configuration that uses the runtime defaults. The server executor uses this
   path and does not apply the manifest tiers.

Current package-manifest defaults are:

| Setting | Default tier |
| --- | --- |
| Memory | Standard, 64 MiB |
| Timeout | Standard, 30 seconds |

The general `WasmLimits::default()` used by direct node-permission conversion
currently allows 256 MiB, 120 seconds, and 100 billion fuel units. The
restrictive preset uses 16 MiB, 10 seconds, and 1 billion fuel units. Do not
assume one table describes every package: inspect the manifest and the
execution path used by the deployment.

Fuel is an instruction budget, not a portable duration. Its wall-clock cost
depends on the module and host.

### Node permissions

External nodes declare permissions with these serialized names:

| Permission | Capability granted |
| --- | --- |
| `network:http` | Outbound HTTP operations |
| `network:websocket` | WebSocket connections |
| `network:tcp` | TCP sockets |
| `network:udp` | UDP sockets |
| `network:dns` | DNS lookups |
| `storage:read` | Read from the host-provided storage scope |
| `storage:write` | Write and delete within the host-provided storage scope |
| `database:read` | Read from wired database and SQL session pins |
| `database:write` | Modify rows through wired database pins |
| `variables` | Read and write execution variables |
| `cache` | Read and write the execution cache |
| `streaming` | Stream incremental output |
| `models` | Use configured model providers |
| `a2ui` | Use dynamic A2UI operations |
| `oauth` | Access configured OAuth flows |
| `functions` | Call registered functions or subflows |

The node-permission conversion begins with no capabilities and adds the
declared set. A package manifest cannot add or remove capabilities. It authors
only resource tiers, a network host allowlist, and OAuth scopes. The capability
listing shown in the store is derived by the registry from the compiled node
definitions, not authored.

:::caution
An empty `allowed_hosts` list means unrestricted hosts. Use an explicit
allowlist when a package only needs known services, but do not treat it as a
complete egress control: the desktop loader for installed packages applies it
to WebSocket connects and WASI sockets, the Flow-Like HTTP host function does
not consult it, and the server executor does not apply it. In a self-hosted
`per_run` sandbox, the deployment's external gateway and network restrictions
still apply to permitted WASM host calls and native nodes.
:::

See [Sandboxing and permissions](/dev/wasm-nodes/sandboxing/) for the author
and user workflow.

## Authentication

The API supports several identities for different callers.

### Interactive users

The deployment delegates interactive authentication to a configured OpenID
Connect provider. Under the API v1 prefix, Flow-Like exposes:

| Endpoint | Behavior |
| --- | --- |
| `/api/v1/auth/openid` | Return the configured OpenID client information |
| `/api/v1/auth/discovery` | Redirect to the provider discovery URL |
| `/api/v1/auth/jwks` | Redirect to the provider JWKS URL |
| `/api/v1/auth/authorize` | Proxy authorization requests |
| `/api/v1/auth/token` | Proxy token exchange |
| `/api/v1/auth/userinfo` | Proxy user-info requests |
| `/api/v1/auth/revoke` | Proxy revocation requests |

Bearer tokens are validated before protected routes resolve the caller and
their app membership.

### API keys and PATs

- Technical users authenticate with `X-API-Key: <key>`.
- Personal Access Tokens use `Authorization: pat_<id>.<secret>`. The API also
  accepts `Authorization: Bearer pat_<id>.<secret>`.

For example, with a token stored in `FLOW_LIKE_PAT`:

```sh
curl --fail https://api.flow-like.com/api/v1/apps \
  -H "Authorization: $FLOW_LIKE_PAT"
```

Authentication identifies the caller; route and app permission checks still
decide what that caller may do. Treat both values as secrets, use narrow roles,
and revoke credentials that are no longer needed.

Personal access tokens use three permission levels: Read Only (`1`), Read &
Write (`2`), and Admin (`4`). Each level is limited by the owner's app and
account permissions. A token does not grant its owner any new role. Read Only
tokens cannot trigger workflows or obtain write-capable upload URLs. Read &
Write tokens can edit and execute ordinary app content; credential and access
management requires Admin.

Earlier UI labels and backend capability names disagreed. Existing values
`1`, `2`, and `4` now follow the UI's three levels. Integrations that used `1`
as unrestricted backend access must replace that token with an appropriately
scoped token. Other legacy capability combinations are rejected and must be
replaced. Creating a PAT through another PAT cannot extend its privileges or
expiry.

The API reads current PAT records and app and registry permissions for each
request. Revocation therefore applies across replicas, at the cost of more
database reads and a dependency on database availability.

### Backend JWTs

API signers use an ES256 P-256 keypair configured through `BACKEND_KEY`,
`BACKEND_PUB`, and optional `BACKEND_KID`. Isolated runners receive the public
verification key and a run-bound JWT, never the private signing key. Tokens include a
type-specific audience. Current audiences cover executors, compilers, users,
realtime collaboration, interaction responders, and app connections.

Verification checks the ES256 algorithm, issuer, expected audience, and token
time claims. All horizontally scaled API instances must use the same active
keypair while tokens signed by it remain valid.

Cached executor identities use the verifier's signed expiry and clock-skew
allowance; caching cannot extend that accepted lifetime. Remote queue references
carry a signed URL and payload hash, which workers verify before downloading
the job. Deploy the API signer before the workers. Workers reject unsigned
remote references, including jobs queued by older producers.

### Untrusted downloads and uploads

Open Graph previews retain support for public and intranet HTTP/HTTPS URLs,
environment proxies and up to five redirects. They read at most 512 KiB,
matching the existing parsing limit without first buffering the entire page.
OAuth discovery and JWKS fetches retain configured HTTP, private-network and
proxy endpoints. Malformed bearer tokens are rejected before fetching keys;
accepted tokens still require signature and claim validation.

Temporary files, registry artifacts, and metadata media
upload directly to object storage using signed URLs. The API handles signing
and metadata rather than file bytes. Temporary upload requests accept 100 files
per batch; clients can send further batches without a daily file-count quota.
Signed PUT URLs remain reusable until expiry, including for retries. The
reported temporary size limit is a client hint, not a byte limit
enforced by those signed PUTs. Object-store policies and deployment quotas
must govern storage consumption.

Cleanup deletes objects under `tmp/user/` after 32 days and objects
under `tmp/solution-staging/` after eight days, allowing issued download URLs
to remain usable for their full seven-day lifetime. This includes older temporary
objects but excludes app files and submitted solution attachments. Persistent
API processes run cleanup hourly; serverless deployments use the
`cache_cleanup` maintenance job. Keep that maintenance schedule enabled.

New 24-hour solution submissions and upload reservations are retired. Their
endpoints return `410 Gone` without creating storage objects, email or payment
sessions. Historical records remain available to administrators; tracking
requires sign-in and the request's tracking token.

Workflow HTTP nodes preserve response bodies, including the final body after
streaming callbacks. Operators may set positive
`FLOW_LIKE_HTTP_MAX_RESPONSE_BYTES` and `FLOW_LIKE_HTTP_TIMEOUT_SECONDS` values
to opt into byte and duration limits. Neither setting imposes a default limit.
Attachment extraction and DOCX/PPTX processing retain support for large inputs
and collections. Their buffered outputs and document expansion still require
suitable execution memory and isolation.

Registry publication stages input in temporary files, hashes it incrementally,
and uploads extracted widget files individually. Media transformation stages
input on disk before decoding and retains the image library's existing decoder
limits. If temporary files are unavailable, both paths fall back to memory.
Registry publication also retains single-PUT support for stores that cannot
complete multipart uploads. Large artifacts remain supported, with resource
use governed by available memory, disk and existing format and compiler limits.

Server runs never receive the key of a private (`custom:*`) Bit. Executors
reach the private Bits of a run's profile through the API's model relay, which
loads the Bit for its owner and calls the provider itself. Subscription Bits
(Claude Code, Codex, GitHub Copilot and Microsoft 365 Copilot) run only on
their owner's desktop. The API's calls to private model endpoints use
guarded egress: public HTTPS only, unless the operator sets
`CUSTOM_MODEL_EGRESS=private`, which also allows private address ranges for
self-hosted endpoints such as vLLM or Ollama. Host-plane addresses are refused
in every mode: `0.0.0.0/8`, loopback, link-local (`169.254.0.0/16`,
`fe80::/10`), broadcast, multicast, `168.63.129.16`, `100.100.100.200`,
`fd00:ec2::254`, `fd00:ec2::23` and `fd20:ce::254`. On a server, a custom
Vertex Bit needs a service-account key or an access token; application-default
credentials, which would resolve the server's own identity, are refused.
Gemini's own media download accepts only public HTTPS URLs.

Media for a private model travels as links wherever a provider can fetch them.
The executor sends links to public hosts and into the installation's own
storage as the run holds them, and media that exists only as bytes inline. No
provider reaches a private or internal host, so the executor fetches media
there itself, with the reach of its HTTP nodes (host-plane addresses refused),
and sends it inline, up to 4 MB per call within the relay's 6 MB request limit.
The relay then hands a link to providers that fetch public HTTPS links
themselves; links into the installation's own storage are checked with the
run's own link and re-signed for five minutes first. Media a provider needs
inline is fetched by the relay: from its own storage even when that is
internal, otherwise only over public HTTPS, also when
`CUSTOM_MODEL_EGRESS=private` lets the model endpoint itself be private, with a
timeout and a size cap per provider that downloads reserve before their bytes
arrive. The relay never fetches any other host. Runs that call a provider
directly follow the same rule: a provider that fetches links itself gets public
ones, and the run sends any other media inline. Links into the installation's
own storage are re-signed for five minutes with the run's own credentials, or
sent inline when the run cannot read them that way; only object keys inside the
storage's own bucket count as its storage.
A file type a model cannot take at all reaches it as the note
`[attachment omitted: unsupported file type <type>]`; any other media that
cannot be delivered fails the call, naming the host, never the URL. A host that
does not resolve is refused with the same message as one on a refused address.

Global chat fetches attachment links the same way: from the installation's own
storage, or over public HTTPS with a timeout and a size cap enforced while
reading. Other attachment downloads retain their existing network behavior,
including server-local services and proxies. Identity, attachment and Open
Graph fetches can therefore reach destinations allowed by the deployment
network. Open Graph
destinations come directly from callers; attachment URLs can also originate
from tenant-controlled data. Network isolation remains necessary where those
callers must not reach server-local services.

Compiler jobs retain their existing size, concurrency and timeout settings.
A timeout ends the async job but cannot stop native compilation that has already
started on a blocking thread. Such work can continue consuming resources after
the request ends. Signed job validation and deployment resource limits remain
in place; hard cancellation of native work is not implemented.

## Authorization

App roles are bitflags evaluated at protected routes. `Owner` and `Admin`
imply the other permissions.

| Area | Permissions |
| --- | --- |
| Team and roles | `ReadTeam`, `ReadRoles` |
| Files | `ReadFiles`, `WriteFiles` |
| Databases | `ReadDatabase`, `WriteDatabase` |
| Boards | `ReadBoards`, `ExecuteBoards`, `WriteBoards` |
| Events | `ListEvents`, `ReadEvents`, `ExecuteEvents`, `WriteEvents` |
| Observability | `ReadLogs`, `ReadAnalytics` |
| App configuration | `ReadConfig`, `WriteConfig` |
| Reusable content | `ReadTemplates`, `WriteTemplates`, `ReadCourses`, `WriteCourses`, `ReadWidgets`, `WriteWidgets` |
| Routes and metadata | `WriteRoutes`, `WriteMeta` |
| API invocation | `InvokeApi` |

`ReadFiles` and `WriteFiles` also imply the corresponding database access for
backward compatibility. Prefer the database-only permissions when a role does
not need file access.

### Workflow runs

Server workflow runs never carry a user's access token or personal access
token. Each run receives an executor token bound to its App and run, and the
API decides every request the run makes from that token and the current
records:

- A run that a person triggers acts as that person: their role in the App,
  their profile and their private models. That covers runs in the App, API and
  MCP calls with their login, hosted pages they open while signed in, location
  crossings on their device, and calls another App passes them through. Through
  an App connection their rights are their own role in that App limited by the
  connection's role, column attributes included; a person who is not a member
  may only trigger Events. Owner and Admin rights are never handed out this way.
  Such a run uses the Event's saved OAuth sign-ins only for providers the person
  did not sign in to themselves, only while the run-as person may still run the
  Event, and only to run the Event's current Flow draft or pinned version: a
  hosted page action loaded for an earlier draft is refused once the Flow
  changes.
  Every run re-reads the Event just before it starts, so switching an Event off
  also stops runs that were about to start. Anonymous callers never learn which
  saved sign-in failed.
- A run that nobody triggers (schedules, webhooks, bots, inbound mail,
  anonymous hosted visitors, public API calls, scheduled replays) runs as the
  Event's run-as person: whoever last saved, restored, edited or switched it on,
  or edited the Flow it runs at its latest version, with that person's role. A
  trigger that fires during such an edit runs exactly the draft it read, or
  answers `503 SINK_RUN_AS_UNAVAILABLE` so the sender retries. Only someone with
  `ExecuteEvents` can switch an Event on. When the run-as person leaves the
  App, loses `ExecuteEvents` or no longer has an active account, the Event
  pauses for such runs and the App's owners and admins are notified; runs a
  person triggers continue. Events of an inactive App do not fire. Such runs
  never pass a person on to other Apps: their connection tokens name the run
  (`run:{id}`), not the run-as person. An edit rebinds the Event even while it
  is switched off, so switching it back on never runs someone else's edit with
  the previous person's sign-ins; if the rebind cannot be written, the edit is
  refused with `503 SINK_REBIND_UNAVAILABLE` and nothing is saved.
- A run on a device mints its connection tokens with its user's own sign-in, so
  it always passes that user to connected Apps, with the user's own role there,
  also when nobody started it, such as a schedule. Only a run on the hub can act
  for the App without a person. A run id it names must be a run the same user
  or API key started.
- A run started with an API key acts as the key, with the key's role. It has
  no user cache and no personal locked files, and it must name a member to
  notify. Its requests, model calls included, are refused once the key is
  deleted, loses its role or expires.
- An anonymous hosted visitor's run never has more access than the Event's
  run-as person, limited to `ExecuteEvents`. See
  [Events](/apps/events/#publish-a-hosted-chat-form-or-page).
- Shadow runs and Quality replays get `403 SHADOW_SIDE_EFFECT_REFUSED` for
  side effects through the API, such as App cache writes, notifications,
  connected-App tokens, locked-file writes and OAuth refreshes. See
  [Release and roll back an Event](/apps/event-releases/#validate-with-quality).

A server run reads its person only through `GET /execution/apps/{app_id}/user`,
which returns the profile fields the hub shares in user lookups, never account
internals such as billing or subscription ids. Runs without a person (API keys,
App connections, anonymous visitors) have no profile to read and no personal
cache.

Only desktop-local runs carry a personal access token. It belongs to the
account signed in to the desktop app, which the desktop verifies before it
registers a trigger.

## Storage and credentials

Desktop-local, Docker Compose, Kubernetes, and managed deployments do not have
the same storage boundary.

- Local desktop data is stored on the user's machine unless a configured
  workflow, model provider, connection, or hosted feature sends it elsewhere.
- Hosted APIs use the configured object-store provider and app/user-scoped
  paths.
- Runtime credentials may be scoped for the executing user and app, depending
  on the configured provider.
- A WASM storage capability grants access only through host functions; it does
  not mount the host filesystem into the module.

The bundled RustFS setup separates root, API storage and STS issuer identities.
Root keys reach only the store and initializer. The API requests temporary
credentials whose session policy narrows access to authorized prefixes; the
store must enforce that restriction. The public S3 gateway rejects STS and
administration, while issuance uses a private endpoint. In-run credential renewal
is not implemented: checkout checks actual remaining lifetime against queued
wait, execution and supervisor allowances, including cached grants.

Do not put credentials in board definitions, documentation examples, route
query parameters, or screenshot fixtures.

For Kubernetes, prefer workload identity or an external secret manager where
the provider supports it. Kubernetes Secrets are not encrypted merely because
their manifest values are base64 encoded. Restrict Secret access with RBAC and
enable encryption at rest in the cluster.

For Docker Compose, `.env` and interpolated environment values are deployment
configuration, not a secret-management system. Limit file permissions and use
a dedicated secret mechanism when the environment requires one.

## Transport and network controls

The self-hosted API and supporting services expose HTTP endpoints inside their
deployment network. Production TLS is normally terminated by an ingress,
reverse proxy, or cloud load balancer.

Operators must:

- serve public traffic over HTTPS;
- configure trusted proxy and forwarding behavior correctly;
- avoid exposing Redis, databases, metrics, compiler, or runtime ports
  publicly;
- restrict executor and sink egress to required destinations;
- apply Kubernetes NetworkPolicies or equivalent network controls;
- separate public ingress from internal service authentication.

There is no application-level statement that can compensate for publishing an
internal port directly to an untrusted network.

## Deployment hardening

### Kubernetes

- Use separate service accounts for API, compiler, runtime, and sink
  components.
- Apply least-privilege RBAC and preserve the manager's per-slot NetworkPolicies.
- Set requests, limits, disruption budgets, and autoscaling for the expected
  workload.
- Pin manager/gateway and runner images by immutable digest, as `per_run` requires.
- Install the `runsc` runtime handler on execution nodes and use its RuntimeClass.
  Creating a RuntimeClass object alone does not install gVisor.
- Configure enforcing Cilium policies and `allow-localhost=policy`; verify denial
  of node, API and metadata access on the actual cluster.
- Protect metrics, logs, and tracing endpoints as operational data.

See [Kubernetes security](/self-hosting/kubernetes/security/).

### Docker Compose

- Bind public ports only where required.
- Place internal services on private networks.
- Replace development credentials and signing keys.
- Terminate TLS before public traffic reaches the API.
- Back up object storage and persistent service volumes.
- Keep images and configuration under change control.
- Configure gVisor with `--network=none` and `--host-uds=open`, and pin the runner
  and gateway images. The manager requires a local Unix Docker socket.
- Preserve the local manager SQLite volume and Redis replay/delivery state
  through drained upgrades. Reconcile uncertain work before resuming dispatch.

See [Docker Compose configuration](/self-hosting/docker-compose/configuration/)
and [troubleshooting](/self-hosting/docker-compose/troubleshooting/).

## Dependency and package trust

The repository checks in Rust and Bun lockfiles and maintains third-party
license inventories under `thirdparty/`. Container images can be pinned by
digest. These mechanisms improve reviewability but do not replace vulnerability
scanning, update policy, provenance checks, or deployment-specific image
verification.

External WASM packages add another supply-chain boundary. Review the package
source and requested permissions, use the narrowest trust duration available,
and review updates before relying on an existing package identity.

## Reporting vulnerabilities

Do not open a public GitHub issue for a suspected vulnerability. Follow the
repository [security policy](https://github.com/Rheosoph/flow-like/security/policy)
and report privately to
[security@great-co.de](mailto:security@great-co.de).

Include:

- the affected component and version or commit;
- reproduction steps or a proof of concept;
- expected and observed behavior;
- potential impact;
- any relevant deployment assumptions.
