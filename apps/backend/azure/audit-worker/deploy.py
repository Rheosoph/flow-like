#!/usr/bin/env python3
"""Preview or provision an isolated audit job with locked Azure Blob retention."""

import argparse
import fnmatch
import json
import re
import subprocess
import sys
import uuid
from urllib.parse import urlparse


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    for name in ("subscription", "resource-group", "location", "environment-id", "api-identity-id",
                 "storage-account", "image", "key-id", "key-vault-id", "secrets-vault-id",
                 "database-secret-uri", "entry-key-secret-uri", "config-secret-uri"):
        result.add_argument(f"--{name}", required=True)
    result.add_argument("--name", default="flow-like-audit-worker")
    result.add_argument("--container", default="audit")
    result.add_argument("--encryption-secret-uri")
    result.add_argument("--retention-days", type=int, default=1461)
    result.add_argument("--apply", action="store_true",
                        help="Create resources and irreversibly lock container retention")
    return result


def versioned_uri(value, kind):
    parsed = urlparse(value)
    parts = parsed.path.strip("/").split("/")
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.query or parsed.fragment:
        raise ValueError(f"expected an HTTPS Key Vault {kind} URI without credentials, query or fragment")
    if len(parts) != 3 or parts[0] != kind or not all(re.fullmatch(r"[A-Za-z0-9-]+", part) for part in parts[1:]):
        raise ValueError(f"expected a versioned /{kind}/name/version URI")
    return parts[1]


def plan(args):
    if not re.fullmatch(r"[^\s]+@sha256:[0-9a-f]{64}", args.image):
        raise ValueError("--image must be an immutable image digest")
    if args.retention_days < 1461:
        raise ValueError("--retention-days must cover at least the default archive horizon (1461 days)")
    scope = f"/subscriptions/{args.subscription}/resourceGroups/{args.resource_group}"
    identity_id = f"{scope}/providers/Microsoft.ManagedIdentity/userAssignedIdentities/{args.name}"
    if identity_id.lower() == args.api_identity_id.lower():
        raise ValueError("the audit worker must have an identity separate from the API")
    for vault in (args.key_vault_id, args.secrets_vault_id):
        if not re.fullmatch(r"/subscriptions/[^/]+/resourceGroups/[^/]+/providers/Microsoft.KeyVault/vaults/[^/]+", vault, re.I):
            raise ValueError("vault IDs must be complete Azure resource IDs")
    key_name = versioned_uri(args.key_id, "keys")
    if urlparse(args.key_id).hostname.split(".")[0].lower() != args.key_vault_id.rsplit("/", 1)[1].lower():
        raise ValueError("--key-id must belong to --key-vault-id")

    def command(*parts):
        return ["az", *parts, "--subscription", args.subscription, "--only-show-errors", "--output", "json"]

    storage_id = f"{scope}/providers/Microsoft.Storage/storageAccounts/{args.storage_account}"
    container_id = f"{storage_id}/blobServices/default/containers/{args.container}"
    group = ["--resource-group", args.resource_group]
    storage = ["--account-name", args.storage_account, "--container-name", args.container, *group]
    steps = [{"identity": True,
              "run": command("identity", "create", "--name", args.name, *group, "--location", args.location)}]
    steps.append({"ensure": command("storage", "account", "show", "--name", args.storage_account, *group),
                  "run": command("storage", "account", "create", "--name", args.storage_account, *group,
                                 "--location", args.location, "--sku", "Standard_LRS", "--kind", "StorageV2",
                                 "--https-only", "true", "--min-tls-version", "TLS1_2",
                                 "--allow-shared-key-access", "false", "--allow-blob-public-access", "false")})
    steps.append({"verify_account": True,
                  "inspect": command("storage", "account", "show", "--name", args.storage_account, *group)})
    private_secrets = [args.database_secret_uri, args.config_secret_uri]
    secret_scopes = [f"{args.secrets_vault_id}/secrets/{versioned_uri(uri, 'secrets')}" for uri in private_secrets]
    job_scope = f"{scope}/providers/Microsoft.App/jobs/{args.name}"
    steps.append({"verify_api_access": True, "api_identity": args.api_identity_id,
                  "vaults": sorted({args.key_vault_id, args.secrets_vault_id}),
                  "scopes": [storage_id, container_id, f"{args.key_vault_id}/keys/{key_name}",
                             *secret_scopes, identity_id, job_scope],
                  "subscription": args.subscription})
    steps.append({"run": command("storage", "container-rm", "create", "--name", args.container,
                                 "--storage-account", args.storage_account, *group)})
    steps.append({"lock_container": True, "minimum_days": args.retention_days,
                  "inspect": command("storage", "container", "immutability-policy", "show", *storage),
                  "create": command("storage", "container", "immutability-policy", "create", *storage,
                                    "--period", str(args.retention_days), "--allow-protected-append-writes", "false"),
                  "run": command("storage", "container", "immutability-policy", "lock", *storage,
                                 "--if-match", "<policy-etag>")})
    # Custom roles omit deletion, retention administration and unrelated key operations.
    for role, permissions, resource in (
        ("objects", ["Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read",
                     "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write"], container_id),
        ("sign", ["Microsoft.KeyVault/vaults/keys/read", "Microsoft.KeyVault/vaults/keys/sign/action"],
         f"{args.key_vault_id}/keys/{key_name}"),
    ):
        role_id = str(uuid.uuid5(uuid.NAMESPACE_URL, f"{scope}/{args.name}/{role}"))
        # Assignable scopes include the key vault when it lives outside the job resource group.
        assignable = sorted({scope, args.key_vault_id.rsplit("/providers/", 1)[0]}) if role == "sign" else [scope]
        definition = {"properties": {"roleName": f"{args.name}-{role}-{role_id[:8]}", "type": "CustomRole",
                                     "description": f"Audit worker {role} access", "assignableScopes": assignable,
                                     "permissions": [{"actions": [], "notActions": [], "dataActions": permissions,
                                                      "notDataActions": []}]}}
        # `az role definition create` ignores a supplied ID and generates one.
        # PUT binds the stable ID used by the assignments and safely supports retries.
        steps.append({"run": command("rest", "--method", "put", "--url",
                                     f"https://management.azure.com/subscriptions/{args.subscription}/providers/Microsoft.Authorization/roleDefinitions/{role_id}?api-version=2022-04-01",
                                     "--body", json.dumps(definition))})
        steps.append({"run": command("role", "assignment", "create", "--assignee-object-id", "<worker-principal-id>",
                                     "--assignee-principal-type", "ServicePrincipal", "--role", role_id, "--scope", resource)})
    secrets = {"database": ("DATABASE_URL", args.database_secret_uri),
               "entry-key": ("AUDIT_ENTRY_KEY", args.entry_key_secret_uri),
               "config": ("FLOW_LIKE_CONFIG_JSON", args.config_secret_uri)}
    if args.encryption_secret_uri:
        secrets["encryption"] = ("SINK_TOKEN_ENCRYPTION_KEY", args.encryption_secret_uri)
    for _, uri in secrets.values():
        name = versioned_uri(uri, "secrets")
        if urlparse(uri).hostname.split(".")[0].lower() != args.secrets_vault_id.rsplit("/", 1)[1].lower():
            raise ValueError("all secret URIs must belong to --secrets-vault-id")
        steps.append({"run": command("role", "assignment", "create", "--assignee-object-id", "<worker-principal-id>",
                                     "--assignee-principal-type", "ServicePrincipal", "--role", "Key Vault Secrets User",
                                     "--scope", f"{args.secrets_vault_id}/secrets/{name}")})
    steps.append({"verify_worker_access": True, "api_identity": identity_id,
                  "vaults": sorted({args.key_vault_id, args.secrets_vault_id}),
                  "scopes": [storage_id, container_id, f"{args.key_vault_id}/keys/{key_name}",
                             *secret_scopes, identity_id, job_scope],
                  "subscription": args.subscription})
    environment = [{"name": name, "secretRef": secret} for secret, (name, _) in secrets.items()]
    environment.extend({"name": name, "value": value} for name, value in {
        "AZURE_CLIENT_ID": "<worker-client-id>", "AZURE_STORAGE_ACCOUNT_NAME": args.storage_account,
        "AZURE_AUDIT_CONTAINER": args.container, "AUDIT_KMS_PROVIDER": "azure",
        "AUDIT_KMS_KEY_ID": args.key_id, "AUDIT_WORKER": "on",
    }.items())
    job = {"location": args.location,
           "identity": {"type": "UserAssigned", "userAssignedIdentities": {identity_id: {}}},
           "properties": {"environmentId": args.environment_id,
                          "configuration": {"triggerType": "Schedule", "replicaTimeout": 3600, "replicaRetryLimit": 1,
                                            "scheduleTriggerConfig": {"cronExpression": "* * * * *", "parallelism": 1,
                                                                      "replicaCompletionCount": 1},
                                            "secrets": [{"name": name, "keyVaultUrl": uri, "identity": identity_id}
                                                        for name, (_, uri) in secrets.items()]},
                          "template": {"containers": [{"name": "audit-worker", "image": args.image,
                                                       "args": ["--once"], "env": environment,
                                                       "resources": {"cpu": 1, "memory": "2Gi"}}]}}}
    steps.append({"run": command("rest", "--method", "put", "--url",
                                 f"https://management.azure.com{scope}/providers/Microsoft.App/jobs/{args.name}?api-version=2024-03-01",
                                 "--body", json.dumps(job))})
    return steps


def run(command, check=True):
    result = subprocess.run(command, text=True, capture_output=True)
    if check and result.returncode:
        raise RuntimeError(f"cloud command failed: {' '.join(command[:4])}; inspect it with your deployment identity")
    return result


def validate_lock(metadata, minimum_days):
    policy = metadata.get("properties", metadata)
    if int(policy.get("immutabilityPeriodSinceCreationInDays", 0)) < minimum_days:
        raise ValueError("existing container retention is shorter than requested; extend it before deployment")
    if policy.get("allowProtectedAppendWrites") or policy.get("allowProtectedAppendWritesAll"):
        raise ValueError("audit storage must not allow protected append writes")
    return policy.get("state") == "Locked"


def unsafe_api_role(definition, worker=False):
    operations = {
        "dataActions": ["Microsoft.Storage/storageAccounts/blobServices/containers/blobs/read",
                        "Microsoft.Storage/storageAccounts/blobServices/containers/blobs/write",
                        "Microsoft.KeyVault/vaults/keys/sign/action", "Microsoft.KeyVault/vaults/secrets/getSecret/action",
                        "Microsoft.KeyVault/vaults/secrets/setSecret/action"],
        "actions": ["Microsoft.Authorization/roleAssignments/write", "Microsoft.Authorization/roleDefinitions/write",
                    "Microsoft.Storage/storageAccounts/write", "Microsoft.Storage/storageAccounts/listkeys/action",
                    "Microsoft.KeyVault/vaults/write", "Microsoft.ManagedIdentity/userAssignedIdentities/assign/action",
                    "Microsoft.ManagedIdentity/userAssignedIdentities/federatedIdentityCredentials/write",
                    "Microsoft.App/jobs/write", "Microsoft.App/jobs/listSecrets/action", "Microsoft.App/jobs/start/action"],
    }
    if worker:
        operations["dataActions"] = ["Microsoft.Storage/storageAccounts/blobServices/containers/blobs/delete",
                                     "Microsoft.KeyVault/vaults/keys/delete", "Microsoft.KeyVault/vaults/keys/create/action",
                                     "Microsoft.KeyVault/vaults/keys/import/action", "Microsoft.KeyVault/vaults/keys/purge/action",
                                     "Microsoft.KeyVault/vaults/secrets/setSecret/action"]
        operations["actions"].extend(["Microsoft.Storage/storageAccounts/blobServices/containers/immutabilityPolicies/write",
                                       "Microsoft.Storage/storageAccounts/blobServices/containers/delete"])
    for permission in definition.get("permissions", []):
        for kind, candidates in operations.items():
            excluded = "not" + kind[0].upper() + kind[1:]
            for operation in candidates:
                allowed = any(fnmatch.fnmatchcase(operation.lower(), pattern.lower()) for pattern in permission.get(kind, []))
                denied = any(fnmatch.fnmatchcase(operation.lower(), pattern.lower()) for pattern in permission.get(excluded, []))
                if allowed and not denied:
                    return True
    return False


def verify_api_access(step):
    def az(*parts):
        return ["az", *parts, "--subscription", step["subscription"], "--only-show-errors", "--output", "json"]

    for vault_id in step["vaults"]:
        vault = json.loads(run(az("resource", "show", "--ids", vault_id)).stdout)
        if vault.get("properties", {}).get("enableRbacAuthorization") is not True:
            raise ValueError("audit key and secret vaults must use Azure RBAC; legacy access policies are not accepted")
    identity = json.loads(run(az("identity", "show", "--ids", step["api_identity"])).stdout)
    principal = identity["principalId"]
    # Expand service-principal groups explicitly; --include-groups documents user membership only.
    membership = json.loads(run(az("rest", "--method", "post", "--url",
                                   f"https://graph.microsoft.com/v1.0/directoryObjects/{principal}/getMemberGroups",
                                   "--body", '{"securityEnabledOnly":true}')).stdout)
    if not isinstance(membership.get("value"), list):
        raise ValueError("cannot establish API group memberships")
    role_cache = {}
    for resource in step["scopes"]:
        for object_id in [principal, *membership["value"]]:
            assignments = json.loads(run(az("role", "assignment", "list", "--assignee-object-id", object_id,
                                            "--scope", resource, "--include-inherited",
                                            "--fill-principal-name", "false", "--fill-role-definition-name", "false")).stdout)
            for assignment in assignments:
                role_id = assignment["roleDefinitionId"]
                if role_id not in role_cache:
                    definition = json.loads(run(az("role", "definition", "show", "--id", role_id)).stdout)
                    if not isinstance(definition.get("permissions"), list):
                        raise ValueError("cannot resolve an API role definition")
                    role_cache[role_id] = definition
                # Conditional grants are conservatively rejected too. The deployment
                # identity must see inherited grants, including management-group scopes.
                if unsafe_api_role(role_cache[role_id], worker="verify_worker_access" in step):
                    actor = "worker" if "verify_worker_access" in step else "API"
                    raise ValueError(f"{actor} retains excess audit authority at {resource}; remove the direct or inherited role assignment")


def apply(steps):
    replacements = {}
    for step in steps:
        if "identity" in step:
            identity = json.loads(run(step["run"]).stdout)
            replacements = {"<worker-principal-id>": identity["principalId"], "<worker-client-id>": identity["clientId"]}
        elif "verify_account" in step:
            account = json.loads(run(step["inspect"]).stdout)
            if account.get("allowSharedKeyAccess") is not False or account.get("allowBlobPublicAccess") is not False:
                raise ValueError("audit account must disable shared keys and public blob access")
        elif "verify_api_access" in step or "verify_worker_access" in step:
            verify_api_access(step)
        elif "lock_container" in step:
            current = run(step["inspect"], check=False)
            policy = json.loads(current.stdout) if current.returncode == 0 else json.loads(run(step["create"]).stdout)
            if not validate_lock(policy, step["minimum_days"]):
                command = [value.replace("<policy-etag>", policy["etag"]) for value in step["run"]]
                run(command)
            if not validate_lock(json.loads(run(step["inspect"]).stdout), step["minimum_days"]):
                raise ValueError("container retention did not become locked; refusing to deploy the worker")
        elif "ensure" not in step or run(step["ensure"], check=False).returncode != 0:
            command = step["run"]
            for before, after in replacements.items():
                command = [value.replace(before, after) for value in command]
            run(command)


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        steps = plan(args)
        if args.apply:
            apply(steps)
            print("Audit job deployed. Verify a signed checkpoint and copy it to independent storage.")
        else:
            print(json.dumps(steps, indent=2))
        return 0
    except (ValueError, RuntimeError, OSError) as error:
        print(f"audit worker deployment: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
