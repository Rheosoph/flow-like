---
title: Container releases
description: Build backend images, select release digests and configure them at startup
---

The [Backend containers workflow](https://github.com/Rheosoph/flow-like/blob/dev/.github/workflows/containers.yml)
builds images from one source commit, checks each image before publishing it to
GitHub Container Registry (GHCR), assembles a manifest of immutable digests, and
then publishes one multi-architecture index per self-hosted repository together
with an immutable build tag and guarded channel tags such as `dev` and `latest`.
Follow a channel tag for development hosts; pin a digest or build tag for
production and retain it with the deployment configuration.

## Published image sets

Repository names follow `ghcr.io/<owner>/flow-like-<target>-<workload>`; the owner
is the lowercased GitHub repository owner. Forks publish to their own namespace.
The checked-in matrix contains 58 platform builds for 42 image repositories:

| Target | Workloads | Platform |
| --- | --- | --- |
| AWS | API, compiler, file-tracker, media-transformer, event-bridge, maintenance, signaling, migration | ARM64 |
| AWS | executor, executor-async | AMD64 |
| AWS | api-ecs | AMD64 and ARM64 |
| GCP | API, queue-worker, executor, signaling, migration, scheduler, maintenance | AMD64 |
| Azure | API, queue-worker, executor, maintenance, scheduler, migration, signaling, otel-collector | AMD64 |
| Docker Compose | API, runtime, compiler, execution-manager, sink-services, signaling, db-init, object-store-init, web | AMD64 and ARM64 |
| Kubernetes | API, executor, execution-manager, sink-trigger, migration, web | AMD64 and ARM64 |

Workload names in image repositories are lowercase. Kubernetes also uses the
Compose runtime, compiler, signaling and object-store initializer images. The
workflow leaves PostgreSQL, Redis, RustFS and monitoring tools as upstream images.
See [the target matrix](https://github.com/Rheosoph/flow-like/blob/dev/.github/scripts/container_images.py)
for exact Dockerfile paths and platforms.

Every build record identifies a single-platform image. AMD64 and ARM64 are
separate records, such as `docker-compose-runtime-arm64`. After the complete
set is published, the workflow combines both records of each self-hosted
repository into one multi-architecture image index, so a Compose host or a
mixed-architecture Kubernetes cluster pulls the index by tag or digest and
receives the matching platform. Cloud repositories stay single-image manifests
because Lambda rejects an index. The AWS ECS API never runs on Lambda, so its two
records become an index as well. See [Tags and visibility](#tags-and-visibility).

The fifteen self-hosted packages are public: anyone can pull them without a
registry login. Cloud packages (AWS, GCP, Azure) stay private to the owner
organization. Nothing in the repository changes visibility; it is a package
setting in the GitHub UI, and forks publish private packages by default.

## Build locally

Run Docker builds from the repository root, with Docker and Buildx available.
Use the target's native architecture when compiling Rust. For the AWS API:

```sh
docker buildx build \
  --platform linux/arm64 \
  --provenance=false \
  --load \
  --tag flow-like-aws-api:local \
  --file apps/backend/aws/api/Dockerfile \
  .
```

The final `.` supplies the root workspace, lockfile and packages.
`--load` imports this single-platform result into the local Docker image store.
Choose another recipe and platform from the matrix when building another image.
`CARGO_BUILD_JOBS` can limit Rust compilation to fit builder memory.

The AWS API and file tracker use an empty `CRDB_CA_URL` for DSQL. For an Aurora
database in `eu-west-1`, add:

```sh
--build-arg CRDB_CA_URL=https://truststore.pki.rds.amazonaws.com/eu-west-1/eu-west-1-bundle.pem
```

An external database may require its own CA bundle. Runtime hub configuration
does not replace certificates baked into an image. Database passwords, signing
keys and deployment storage credentials belong in runtime configuration and are
not required to compile the standard image matrix.

GCP and Azure API recipes also accept an optional legacy custom compiled
default through the `flow_like_config` BuildKit secret and its
`FLOW_LIKE_CONFIG_SHA256` build argument. The resulting executable contains that
document. A secret mount avoids a loose copy in a build layer; it does not hide
the embedded content from someone who can pull the image. Prefer runtime config
for installation-specific settings.

## Runtime API configuration

Set one nonempty source on the API container:

| Variable | Value |
| --- | --- |
| `FLOW_LIKE_CONFIG_JSON` | A complete JSON configuration, including a deployment-secret injection |
| `FLOW_LIKE_CONFIG_FILE` | A readable mounted JSON file |
| `FLOW_LIKE_CONFIG_SECRET_REF` | A key or `secret://...` reference resolved by the API's configured secret store |

An override replaces the entire document. It is never merged with the compiled
default. With no override, the default is used. Empty environment values count
as unset; whitespace-only values, conflicting sources, unreadable files,
unresolved secrets and invalid documents stop startup without selecting a
fallback. Input must be UTF-8 and no larger than 4 MiB. Deployment platforms and
secret providers may impose smaller limits; mounted files avoid environment
size limits.

The API reads the document once. Restart or roll out API processes after changing
it. For example, supply the normal API environment and a read-only config mount:

```sh
docker run --env-file ./api.env \
  --mount type=bind,src=/absolute/path/flow-like.config.json,dst=/run/secrets/flow-like.config.json,readonly \
  --env FLOW_LIKE_CONFIG_FILE=/run/secrets/flow-like.config.json \
  ghcr.io/rheosoph/flow-like-docker-compose-api@sha256:<selected-platform-digest>
```

The selected document configures Hub metadata, OpenID issuer/audience/client
validation and third-party OAuth endpoints. Keep OAuth client secrets in the
secret store, referenced by `client_secret_env`; literal
`oauth_providers.*.client_secret` values are rejected. Public Hub fields remain
visible to clients even when the document comes from a secret. Startup errors
do not include the document contents or secret reference.

Secret references use the API's existing provider configuration: AWS Parameter
Store, GCP Secret Manager or Azure Key Vault where configured. File mounts and
secret-injected environment values work for self-hosting. This selection grants
no new cloud permissions and does not change compiled provider features.

Compose and Kubernetes API releases include the corresponding self-hosting
example with placeholder OIDC settings. Supply the installation's complete
document at startup. Their release records use
`runtime-config-self-hosted-default`, hash that fallback, and identify the
`full-document-v1` runtime contract. Follow the
[Compose configuration](/self-hosting/docker-compose/configuration/#hub-configuration-and-identity)
or [Helm configuration](/self-hosting/kubernetes/configuration/#runtime-api-and-web-configuration)
instructions to supply it.

## Runtime web configuration

Both self-hosted web images accept these public settings at container startup:

| Variable | Meaning |
| --- | --- |
| `FLOW_LIKE_WEB_API_URL` | Required browser-reachable HTTP(S) API base URL |
| `FLOW_LIKE_WEB_REDIRECT_URL` | Optional login callback; defaults to the browser origin plus `/callback` |
| `FLOW_LIKE_WEB_LOGOUT_URL` | Optional logout destination; defaults to the browser origin plus `/` |

Compose maps its `NEXT_PUBLIC_*` deployment values into these variables. Helm
uses `web.runtimeConfig.apiUrl`, falling back to `api.publicUrl`, plus optional
`redirectUrl` and `logoutUrl`. Recreate containers or roll out Pods after changes;
the same image can serve another installation without rebuilding.

The entrypoint validates URLs and writes only these allowlisted public values
to `/tmp/flow-like-web/runtime-config.js`. Nginx serves it with `no-store` and the
app loads it before hydration. A missing API URL, URL credentials, query strings
or fragments stop startup. The image runs without root and supports a read-only
root filesystem with writable `/tmp`.

Non-container web and desktop builds retain their build-time behavior. Static
SEO/social metadata also keeps its build defaults. Third-party integration OAuth
still uses the hosted `flow-like.com/thirdparty/callback` relay and may select
the stored profile's Hub. These runtime variables do not configure that separate
relay or its provider registrations.

## Tags and visibility

Each repository receives three kinds of references. Every tag of one release
resolves to the same digest, so a tag is only a name for a digest.

| Reference | Example | Moves? |
| --- | --- | --- |
| Digest | `ghcr.io/rheosoph/flow-like-docker-compose-api@sha256:…` | Never; pin this in production |
| Build tag | `sha-<40-char commit>-run-<run id>-<attempt>` | Never; one per workflow attempt |
| Channel tag | `dev`, `main`, `alpha`, `beta`, `latest`, `1`, `1.2`, `1.2.3`, `1.2.3-beta` | Forward only, see the guard below |

The build job also pushes a per-architecture tag,
`sha-<commit>-<amd64|arm64>-run-<run id>-<attempt>`, for each scanned
single-platform image. Those remain available for a platform-specific pull, but
the references above are the ones to consume.

For the fifteen self-hosted repositories the digest and every tag name a
multi-architecture index containing the AMD64 and ARM64 images built from that
commit. The workflow requests the index annotations
`org.opencontainers.image.revision`, `org.opencontainers.image.source` and
`org.opencontainers.image.version`; an OCI index keeps them, while a Docker
manifest list (produced when the pushed images use Docker media types) cannot
carry annotations. Apart from the ECS API, the twenty-six cloud repositories are
single-platform: the tags point at the image manifest itself, never at an index,
because Lambda requires a single-image manifest. The workflow asserts this after every copy.
The image config labels carry the revision and source for both kinds and are the
fallback the guard reads.

Which channel tags a run applies depends on the Git ref that triggered it:

| Ref | Channel tags | `version` annotation |
| --- | --- | --- |
| `dev`, `main`, `alpha` branch | `dev`, `main` or `alpha` | branch name |
| `v1.2.3` | `1.2.3`, `1.2`, `1`, `latest` | `1.2.3` |
| `v0.4.0` | `0.4.0`, `0.4`, `latest` (no `0` major tag) | `0.4.0` |
| `v1.2.3-rc.1` | `1.2.3-rc.1` only | `1.2.3-rc.1` |
| `beta-v0.4.0` (or any `<channel>-v…`) | `0.4.0-beta`, `beta` | `0.4.0` |
| any other ref | none; the build tag only | none |

The desktop release workflow creates `beta-v*` tags, so each desktop beta also
publishes backend images under `<version>-beta` and `beta`. Semantic versions
follow `MAJOR.MINOR.PATCH` with an optional `-prerelease` part; build metadata is
not a valid Docker tag and is not accepted.
`python3 .github/scripts/container_images.py tags --ref refs/tags/v1.2.3`
prints the tags for a ref.

Moving channel tags are guarded so a re-run of an old ref, or a release branch
that lost commits, never makes a package point backwards:

- `dev`, `main`, `alpha`, `beta` and other channel names move only when the
  commit recorded on the existing tag is the same as, or an ancestor of, the
  commit being published (`git merge-base --is-ancestor`).
- `latest`, `<major>` and `<major>.<minor>` move only when the new semantic
  version is greater than or equal to the version recorded on the existing tag.
  When an index carries no version annotation (a Docker manifest list drops
  annotations; single-image cloud manifests never have one), the guard reads the
  `v*` tag on the recorded commit from Git history instead.
- Exact version tags such as `1.2.3` and `1.2.3-beta` are not guarded.
- Missing tags, unreadable metadata or a recorded commit that no longer exists in
  the repository let the tag move; the job logs the reason.

A skipped tag never fails the job. Each skip is logged as a workflow warning
and recorded in `container-indexes.json` under `skipped_tags` with its reason.
The build tag and the digest are always published, so a release whose channel
tag was kept is still consumable by its immutable references.

Self-hosted packages are public and need no `docker login`. Cloud packages are
private; pulling them, or the packages of a private fork, needs a login with a
token that has `read:packages`. The workflow never changes visibility.

## Publish and select a release

The workflow needs `contents: read`, `packages: write` and permission to create
organization packages. It authenticates with `GITHUB_TOKEN` after image checks
pass. Existing GHCR packages must also grant this source repository Actions
write access. It does not need deployment-cloud credentials or a custom registry token.
Protect `dev`, `main`, `alpha`, `v*` and `beta-v*` because pushes to them publish
images and move channel tags.

1. Push the reviewed source commit or tag, or select **Actions > Backend
   containers > Run workflow** when a manual release is needed. A manual run
   from `dev`, `main` or `alpha` moves that branch's channel tag.
2. Select `aws`, `gcp`, `azure`, `docker-compose`, `kubernetes`, `self-hosted` or
   `all`. `kubernetes` includes its shared Compose helpers; `self-hosted` selects
   all fifteen self-hosted repositories on both architectures.
3. After the complete set succeeds, the `manifest` job uploads
   `container-images-<commit>-<run>-<attempt>`. Its `container-images.json` maps
   workloads and platforms to single-platform digest references.
4. The `index` job then creates the per-repository index or manifest copy, applies
   the build tag and the guarded channel tags, and uploads
   `container-indexes-<commit>-<run>-<attempt>`. Its `container-indexes.json`
   lists, per repository, the `kind` (`index` or `manifest`), the digest to pin,
   the platforms, the applied `tags` and any `skipped_tags`.
5. Set package visibility and consumer read access explicitly.

Records bind the Dockerfile and relevant config/CA identity. The `manifest` job
rejects missing, duplicate, wrong-platform and mixed-build records, and the
`index` job re-validates the manifest before touching the registry. A failed
matrix can leave individually checked images in GHCR without a complete manifest
and without channel tags. Use **Re-run all jobs** so every record comes from one
attempt. Pin digests for deployments; save both manifests before their 90-day
artifact retention expires.

For production, pin the index digest from `container-indexes.json` or the build
tag; both are immutable. Use a channel tag only where following a channel is the
intent, such as a development host on `dev`, and keep `pullPolicy: Always` or an
explicit re-pull step so that host receives the moved tag.

Compose reads `FLOW_LIKE_IMAGE_TAG` (default `dev`) and the per-workload
`API_IMAGE`, `RUNTIME_IMAGE`, `EXECUTION_MANAGER_IMAGE`, `COMPILER_IMAGE`,
`SINK_SERVICES_IMAGE`, `SIGNALING_IMAGE`, `DB_INIT_IMAGE`,
`OBJECT_STORE_INIT_IMAGE` and `WEB_IMAGE` overrides. `scripts/pull-images.py`
pulls one tag and rewrites the overrides, plus the sandbox pins, to
`repository@sha256:…` digests; `scripts/prepare-images.py` builds the runtime and
execution manager locally instead. See
[Compose installation](/self-hosting/docker-compose/installation/).

Helm defaults every first-party image to its published repository at tag `dev`
and accepts a `digest` per image. `scripts/resolve-images.py` resolves one tag to
digests for every image, including the execution manager and sandbox, and can add
pull secrets and an architecture node selector. See
[Kubernetes installation](/self-hosting/kubernetes/installation/).
Database migration images require a reviewed migration operation after
publication. In particular, the Kubernetes migration command uses
`--accept-data-loss`; review schema changes and recovery before deploying it.
See [Database](/self-hosting/kubernetes/database/#schema-application).

For cloud deployment, copy the selected image into the installation's ECR,
Artifact Registry or ACR, verify the destination digest and pass that reference
to deployment tooling. Keep one image digest across environments and version
each environment's API configuration separately. Verify the deployment tooling
accepts prebuilt images before removing any local build steps.

For Lambda, the destination ECR repository must be in the function's region.
Keep the deployable image single-architecture and retain `--provenance=false`
when building it so an attestation image index does not replace the image manifest.

## Publication checks and their limits

Each job builds once and loads the result locally. The
[filesystem helper](https://github.com/Rheosoph/flow-like/blob/dev/.github/scripts/container_publication.py)
checks platform, the included root license and common credential paths without
starting the container. Pinned Trivy `v0.69.3` scans image files, configuration
and history. A second scan checks printable text extracted from every ELF binary
because this scanner version skips ELF files. The job then pushes that same
checked image and verifies that the registry manifest identifies its config.

These are recognized-secret-pattern gates. Compressed, encoded, encrypted and
unrecognized credentials can evade them. They do not establish a CVE policy,
complete dependency-license compliance, signing or cloud promotion. Images
include `/usr/share/licenses/flow-like/LICENSE`; its full Business Source License
parameters govern use.

Image consumers can read executable text and layers. API binaries contain their
default config, including public client IDs, domains and secret-reference names.
Signaling images contain selected TypeScript sources; migration images contain
schemas and migration code. Review those contents for the intended audience.
Keep deployment credentials out of build contexts and runtime configuration
outside image builds.

The supplemental binary scan has two narrow treatments for reviewed public
fixtures:

- Complete GnuTLS self-test PEM blocks are replaced in scanner input only when
  their exact SHA-256 and byte length match the reviewed fingerprints in
  [the fixture catalog](https://github.com/Rheosoph/flow-like/blob/dev/.github/scripts/public_secret_fixtures.json).
  Unknown, modified, truncated or differently encoded keys remain in the input.
- A complete reviewed sequence of adjacent public Copilot redaction literals
  receives a line break at its source boundary. This prevents the Slack prefix
  and adjacent field names from forming a false token. All original bytes remain;
  partial or changed sequences are not normalized.

Neither treatment excludes an image path, library or detection rule, and neither
changes the image. Trivy reports and extracted strings can contain matched
secrets, so they stay in temporary runner storage and are not uploaded or printed.
Reproduce failures in a trusted environment and handle raw reports as sensitive.

## Cache and build capacity

The workflow uses native runners, at most sixteen parallel jobs, and two Cargo
compilation jobs per runner. ARM64 runner access is required. Measure peak memory,
disk and build time before raising those limits.

Each image has a separate GitHub Actions layer-cache scope. Builder layers copy
the source checkout, so do not export them to a public registry build-cache
package. Actions caches are also available to eligible PR workflows; credentials
and private dependency source do not belong there.

Rust recipes use `RUN --mount=type=cache` for crates and build targets. Ordinary
layer-cache export does not persist these mounts. Repository variable
`CONTAINER_CARGO_CACHE=true` enables the optional mount restore/extraction steps;
it is off by default. Budget storage before enabling it. A source change may
otherwise trigger a cold compile even when unchanged layers are cached. The
optional Cargo steps do not export the web builder's `.next` mounts.

Several AWS recipes use `sharing=locked` on a Cargo target cache and can serialize
on one BuildKit worker. Separate concurrent workers or cache namespaces when
needed. Retained native BuildKit workers can preserve large Cargo mounts for
frequent releases, but the current workflow uses ephemeral GitHub-hosted builders.
Keep trusted release caches separate from untrusted builds.
