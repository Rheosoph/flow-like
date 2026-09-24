---
title: Package Registry
description: Publish, review, install, and manage WASM packages
sidebar:
  order: 27
---

The package registry stores versioned WASM node packages, their metadata, permissions, compiled artifacts, access rules, and review history.

## Browse and install

Use the desktop app's **Store → Packages** view to search the registry and open a package detail page. A detail page can show:

- package metadata and current status;
- exported nodes;
- resource tiers, allowed hosts, OAuth scopes, and the capabilities derived from its nodes;
- available versions;
- access or purchase requirements;
- installation state.

Installing a package downloads the selected version into the local registry cache. Installed packages appear under **Library → Packages**, where you can check for updates, update a package, or uninstall it.

:::note
Only active versions that the current user can access are downloadable. Private and request-access packages apply their package membership rules before download.
:::

## Publish a package

Open **Library → Packages → Publish**. The current wizard has four steps:

1. **Upload WASM** — select a file with a valid WebAssembly header.
2. **Manifest** — enter the package ID, name, version, description, license, links, and keywords.
3. **Permissions** — choose the memory and timeout tiers and the allowed hosts. Capabilities come from the node definitions in the binary.
4. **Review** — verify the binary and metadata, then submit.

The client uploads the binary and submits a versioned manifest to the registry. The backend hashes the binary, rejects duplicate package versions, extracts node definitions, and prepares a platform artifact when compilation is configured. When compilation succeeds, the registry derives the package's capability listing from the permissions those node definitions declare.

### Package identity

- Use a stable reverse-domain ID such as `com.example.image-tools`.
- Increment the semantic version for every published artifact.
- A package ID and version pair is immutable.
- Declare capabilities on the nodes, not in the manifest. The manifest authors only the memory and timeout tiers, `allowed_hosts`, and OAuth scopes.

## Private packages and publication

New packages are private by default. A successfully compiled private version can become active for its members without public publication.

Making a package public is a separate governance step:

1. Complete the package metadata and README.
2. Request publication.
3. The package or version enters `pending_review`.
4. An administrator with the global `ManagePackages` permission reviews it.
5. The reviewer can approve, reject, request changes, comment, or flag the submission.

There is no guaranteed review time. Use the package detail and publication-review history as the source of truth.

## Sell a package

The package owner sets the price on the package's **Overview** tab under **Package price**. Prices are in euros. A price of 0 makes the package free. A paid price must fall within the range the platform shows next to the price field.

- You can set a price on a private package ahead of publication. People can only buy it once it is public.
- Packages whose maintainers approve access requests cannot be sold. Approving a request already grants access.
- On the marketplace, payments go to your payout account minus the platform fee. You need a ready payout account (**Account → Payouts**) and must accept the seller terms before you can set a paid price.
- Changing the price never takes the package away from people who already have it.

Buyers who get a refund lose their access to the package. Projects they licensed the package for then hand the licence to another admin or the owner who holds it, or start a 30-day grace period. See [project licences](/start/packages-store/#paid-packages-and-project-licences).

## Statuses

| Status | Meaning |
|---|---|
| `pending_review` | A package or version is waiting for a publication decision |
| `active` | The version is usable by users who have access |
| `rejected` | The submitted version did not pass review |
| `deprecated` | Still represented in the registry but discouraged |
| `disabled` | Not usable |
| `yanked` | A specific released version is excluded from normal version selection |

Status and visibility are different. An active package can still be private.

## What reviewers evaluate

| Area | Evidence to check |
|---|---|
| Binary | Valid WASM, node extraction, and compilation result |
| Permissions | Node-declared capabilities match the implementation and description |
| Node contract | Pins, schemas, defaults, and permissions are internally consistent |
| Metadata | ID, version, description, categories, links, and release notes are accurate |
| Safety | External hosts, storage access, OAuth scopes, and model access are justified |
| Maintenance | Repository, license, documentation, and ownership are clear |

A `verified` flag is registry metadata set by administrators. Treat it as an additional trust signal, not a replacement for reviewing permissions and package provenance.

## Version review behavior

Publishing an update creates a new version record without immediately replacing the package's current active artifact. On approval, the registry promotes the reviewed version and its extracted node definitions, and derives the capability listing from them again. Rejecting the pending version leaves an already-active package version available.

This separation prevents an unreviewed update from silently changing a public package.

## Permissions shown to users

The registry displays two kinds of information. The manifest authors the settings a node cannot state in code. The capabilities are derived from the permissions declared by the compiled node definitions, which are the permissions the sandbox enforces per node. Capability flags authored in a manifest are replaced by the derived ones.

| Area | Examples | Source |
|---|---|---|
| Resources | Memory and timeout tier | Manifest |
| Allowed hosts | Package-wide outbound host allowlist | Manifest |
| OAuth scopes | Provider, scopes, reason, required | Manifest |
| Network | HTTP, WebSocket, TCP, UDP, DNS | Nodes |
| Storage | Node, user, upload, and cache storage | Nodes |
| Database | Read and write | Nodes |
| Runtime context | Variables, cache, streaming, A2UI | Nodes |
| Services | Model access | Nodes |

A package whose nodes declare `storage:read` or `storage:write` lists all four storage areas, because the sandbox gates them on a single storage capability.

Declare the smallest useful set on each node. An empty allowed-host list means unrestricted hosts, not no hosts. The desktop loader for installed packages applies a non-empty list to WebSocket connects and WASI sockets; the Flow-Like HTTP host function does not consult it, and the server executor does not apply it. See [Package Manifest](/dev/wasm-nodes/manifest/) for details.

## Package-author checklist

Before publishing:

- run the template tests and load the package in a representative flow;
- verify every exported node and pin;
- remove unused permissions;
- confirm that secrets are never embedded in the binary or manifest;
- document external data transfer and expected costs;
- publish a new version instead of replacing an existing artifact.

## Related

- [Custom WASM Nodes](/dev/wasm-nodes/overview/)
- [Component Model vs Core Modules](/dev/wasm-nodes/runtime-models/)
- [Package Manifest](/dev/wasm-nodes/manifest/)
- [Sandboxing & Permissions](/dev/wasm-nodes/sandboxing/)
