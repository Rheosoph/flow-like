# Flow-Like Azure API

The Azure image uses a user-assigned managed identity for PostgreSQL. It does
not accept a PostgreSQL password or `DATABASE_URL`.

## Secure image build

The API embeds reviewed, non-secret identity and OAuth-provider metadata at
compile time; JWKS are fetched through the bounded runtime cache. Pass the
Azure configuration contents through a BuildKit secret, and pass only its
non-secret SHA-256 digest as a build argument so a metadata change invalidates
the cached compile layer. A local `flow-like.azure.config.json` is gitignored
and dockerignored, so it stays out of version control and the build context. The tracked repo-root
`flow-like.config.json` is the committed public default that builds require; it
does enter the context, the builder overwrites it with the secret for the
duration of the build `RUN`, and the same `RUN` removes the standalone copy.
The reviewed non-secret contents are intentionally embedded in `/app/api`, so
this input must never contain client secrets:

```sh
CONFIG_PATH=/secure/path/flow-like.azure.config.json
CONFIG_SHA256="$(openssl dgst -sha256 "$CONFIG_PATH" | awk '{print $NF}')"
docker buildx build \
  --secret id=flow_like_config,src="$CONFIG_PATH" \
  --build-arg FLOW_LIKE_CONFIG_SHA256="$CONFIG_SHA256" \
  -f apps/backend/azure/api/Dockerfile \
  .
```

The Dockerfile verifies the digest and rejects a config whose provider is not `azure`. Keep the source
under the protected CI workspace, record its digest in release evidence, and do
not include client secrets in it.

[`entra-external-id.fragment.example.json`](entra-external-id.fragment.example.json)
documents the Azure-specific fragment, not a complete Hub configuration. It is
deliberately not deployment-ready: replace every example tenant/application/URL,
validate the discovery document, issuer, audience and JWKS, and merge the
fragment into the organization's complete reviewed feature/tier configuration
before using that full file as the BuildKit secret.

## Required database environment

```text
AZURE_CLIENT_ID=<API user-assigned identity client UUID>
AZURE_POSTGRES_AUTH_MODE=managed_identity
AZURE_POSTGRES_HOST=<Flexible Server FQDN>
AZURE_POSTGRES_DATABASE=flow_like
AZURE_POSTGRES_USER=<name_prefix>-api-identity
```

Azure Container Apps must also inject its standard `IDENTITY_ENDPOINT` and
`IDENTITY_HEADER` values. Startup rejects a non-loopback identity endpoint,
alternate MSI/IMDS endpoints, and authority-host overrides so the rotating
SSRF header cannot be forwarded to an operator-supplied token service. Proxy
environment variables are also forbidden so a generic HTTP proxy cannot
receive the local managed-identity request or its header.

For the Terraform identity module, the PostgreSQL user is exactly
`${var.name_prefix}-api-identity`; with the development default it is
`flowlike-dev-api-identity`. It is the managed identity's display/resource
name, not its client ID or principal ID.

The process requests an Entra token for
`https://ossrdbms-aad.database.windows.net/.default`, places it only in the
SQLx connection options, and enforces TLS `verify-full`. SQLx 0.8 has no async
password callback for new pooled connections, so the API marks readiness
unavailable five to eight minutes before expiry and terminates after a drain
window. Per-process jitter staggers replicas that received tokens together.
Azure Container Apps must keep the app restartable (and production should use
at least two replicas) so each replacement starts with a fresh token.

## One-time role bootstrap

Azure RBAC on the server does not create a PostgreSQL role. As the configured
Flexible Server Entra administrator, connect to the `postgres` database and
create the API principal. Use the principal/object ID from
`module.identity.principal_ids.api`; binding by object ID avoids ambiguity if a
display name is ever duplicated in the tenant:

```sql
select * from pg_catalog.pgaadauth_create_principal_with_oid(
  '<name_prefix>-api-identity',
  '<API managed identity principal ID>',
  'service',
  false,
  false
);
```

Then connect to `flow_like` and grant only runtime DML rights. Replace the two
quoted role names with the exact Terraform identity names:

```sql
grant connect on database flow_like to "<name_prefix>-api-identity";
grant usage on schema public to "<name_prefix>-api-identity";
grant select, insert, update, delete on all tables in schema public
  to "<name_prefix>-api-identity";
grant usage, select, update on all sequences in schema public
  to "<name_prefix>-api-identity";

alter default privileges for role "<name_prefix>-migration-identity"
  in schema public
  grant select, insert, update, delete on tables
  to "<name_prefix>-api-identity";
alter default privileges for role "<name_prefix>-migration-identity"
  in schema public
  grant usage, select, update on sequences
  to "<name_prefix>-api-identity";
```

Run schema migrations as the separate migration identity. Do not grant the API
identity DDL, role administration, server administration, or database-owner
rights.

## Azure Communication Services Email

The Azure image selects the managed-identity-only ACS Email backend at runtime:

```text
MAIL_PROVIDER=azure_communication_services
ACS_EMAIL_ENDPOINT=https://<communication-resource>.communication.azure.com
ACS_EMAIL_SENDER=DoNotReply@<azure-managed-domain>.azurecomm.net
AZURE_CLIENT_ID=<API user-assigned identity client UUID>
```

`ACS_EMAIL_ENDPOINT` and `ACS_EMAIL_SENDER` are identifiers, not secrets. The
sender must be the verified address exposed by the Terraform communications
module (`default_sender_address`); an Azure-managed domain provides the exact
`DoNotReply@...` address. The API acquires the ACS audience token from the
user-assigned managed identity, disables engagement tracking in every request,
and waits for the long-running send operation to succeed before reporting
success.

The API refuses connection strings, access keys, and `AZURE_CLIENT_SECRET`.
Assign its managed identity the communications module's scoped sender role;
Microsoft currently requires
`Microsoft.Communication/CommunicationServices/Read` and
`Microsoft.Communication/CommunicationServices/Write` for managed-identity
email sending. Local authentication is disabled on the ACS resource.

## Health contract

- `/health/live` checks only the process.
- `/health/ready` and `/health/startup` require an accepting token lifecycle
  and a successful PostgreSQL ping.
- `/health` is a compatibility alias for readiness used by the current Azure
  probes and Front Door origin health check.

## Azure references

- [Container Apps managed identity endpoint](https://learn.microsoft.com/en-us/azure/container-apps/managed-identity)
- [PostgreSQL managed-identity connectivity](https://learn.microsoft.com/en-us/azure/postgresql/security/security-connect-with-managed-identity)
- [PostgreSQL Entra role management](https://learn.microsoft.com/en-us/azure/postgresql/security/security-manage-entra-users)
- [PostgreSQL TLS verification](https://learn.microsoft.com/en-us/azure/postgresql/security/security-tls-how-to-connect)
