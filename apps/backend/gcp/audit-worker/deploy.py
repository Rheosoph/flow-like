#!/usr/bin/env python3
"""Preview or provision an isolated, scheduled audit worker with a locked GCS bucket."""

import argparse
import ipaddress
import json
import re
import subprocess
import sys


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    for name in ("project", "region", "bucket", "image", "key-version", "api-service-account",
                 "database-instance", "database-host", "database-name", "database-ca-secret",
                 "entry-key-secret", "encryption-secret", "config-secret", "network", "subnet"):
        result.add_argument(f"--{name}", required=True)
    result.add_argument("--name", default="flow-like-audit-worker")
    result.add_argument("--database-user", help="Existing Cloud SQL IAM username; defaults to the worker email without .gserviceaccount.com")
    result.add_argument("--previous-entry-key-secret", help="Previous shared entry key during coordinated rotation")
    result.add_argument("--audit-kid", help="Key ID present in --verifying-keys-secret; supply both to avoid startup public-key reads")
    result.add_argument("--verifying-keys-secret", help="Shared public AUDIT_VERIFYING_KEYS JSON map in Secret Manager")
    result.add_argument("--retention-days", type=int, default=1461)
    result.add_argument("--apply", action="store_true",
                        help="Create resources and irreversibly lock the bucket retention policy")
    return result


def plan(args):
    if not re.fullmatch(r"[^\s]+@sha256:[0-9a-f]{64}", args.image):
        raise ValueError("--image must be an immutable image digest")
    match = re.fullmatch(
        r"projects/([^/]+)/locations/([^/]+)/keyRings/([^/]+)/cryptoKeys/([^/]+)/cryptoKeyVersions/([1-9][0-9]*)",
        args.key_version,
    )
    if not match:
        raise ValueError("--key-version must name a Cloud KMS cryptoKeyVersion")
    if args.retention_days < 1461:
        raise ValueError("--retention-days must cover at least the default archive horizon (1461 days)")
    if not re.fullmatch(r"[a-z][a-z0-9-]{4,20}[a-z0-9]", args.name):
        raise ValueError("--name must be a 6-22 character service account ID, leaving room for '-trigger'")
    for value in (args.project, args.region, args.bucket, args.name, args.network, args.subnet,
                  args.database_instance, args.database_host, args.database_name, args.database_user,
                  args.entry_key_secret, args.config_secret, args.encryption_secret, args.database_ca_secret,
                  args.previous_entry_key_secret, args.audit_kid, args.verifying_keys_secret):
        if value is not None and (not value or re.search(r"[\s,=]", value) or value.startswith("-")):
            raise ValueError("resource names must not contain whitespace, commas, equals signs or start with '-' ")
    if bool(args.audit_kid) != bool(args.verifying_keys_secret):
        raise ValueError("--audit-kid and --verifying-keys-secret must be supplied together")
    database_host = normalize_database_host(args.database_host)

    def command(*parts, project=None):
        return ["gcloud", *parts, f"--project={project or args.project}", "--quiet", "--format=json"]

    steps = []

    def ensure(check, create):
        steps.append({"ensure": check, "run": create})

    def deny_api(resource, *permissions):
        for permission in permissions:
            steps.append({"deny_api_permission": permission,
                          "inspect": command("policy-intelligence", "troubleshoot-policy", "iam", resource,
                                             f"--principal-email={args.api_service_account}", f"--permission={permission}")})

    def deny_worker(resource, *permissions):
        for permission in permissions:
            steps.append({"deny_worker_permission": permission,
                          "inspect": command("policy-intelligence", "troubleshoot-policy", "iam", resource,
                                             f"--principal-email={worker}", f"--permission={permission}")})

    worker = f"{args.name}@{args.project}.iam.gserviceaccount.com"
    database_user = worker.removesuffix(".gserviceaccount.com")
    if args.database_user and args.database_user != database_user:
        raise ValueError("--database-user must match the worker service account without .gserviceaccount.com")
    secrets = {"GCP_POSTGRES_SERVER_CA": args.database_ca_secret,
               "AUDIT_ENTRY_KEY": args.entry_key_secret,
               "SINK_TOKEN_ENCRYPTION_KEY": args.encryption_secret,
               "FLOW_LIKE_CONFIG_JSON": args.config_secret}
    shared_secrets = {args.entry_key_secret, args.encryption_secret}
    if args.previous_entry_key_secret:
        secrets["AUDIT_ENTRY_KEY_PREVIOUS"] = args.previous_entry_key_secret
        shared_secrets.add(args.previous_entry_key_secret)
    if args.verifying_keys_secret:
        secrets["AUDIT_VERIFYING_KEYS"] = args.verifying_keys_secret
        shared_secrets.add(args.verifying_keys_secret)
    if len(set(secrets.values())) != len(secrets):
        raise ValueError("CA, configuration, entry, encryption and verifying-key secrets must be distinct")
    trigger_name = f"{args.name}-trigger"
    trigger = f"{trigger_name}@{args.project}.iam.gserviceaccount.com"
    if args.api_service_account in (worker, trigger):
        raise ValueError("worker and scheduler identities must be separate from the API")
    # Separate trigger credentials cannot sign, read secrets, or access the audit bucket.
    for name, email in ((args.name, worker), (trigger_name, trigger)):
        ensure(command("iam", "service-accounts", "describe", email),
               command("iam", "service-accounts", "create", name))
        deny_api(f"//iam.googleapis.com/projects/{args.project}/serviceAccounts/{email}",
                 "iam.serviceAccounts.getAccessToken", "iam.serviceAccounts.signJwt", "iam.serviceAccounts.signBlob",
                 "iam.serviceAccountKeys.create", "iam.serviceAccounts.actAs", "iam.serviceAccounts.setIamPolicy")
    deny_api(f"//cloudresourcemanager.googleapis.com/projects/{args.project}", "resourcemanager.projects.setIamPolicy")
    deny_api("//cloudkms.googleapis.com/" + args.key_version.rsplit("/cryptoKeyVersions/", 1)[0],
             "cloudkms.cryptoKeyVersions.useToSign", "cloudkms.cryptoKeys.setIamPolicy")
    for secret in (args.config_secret,):
        deny_api(f"//secretmanager.googleapis.com/projects/{args.project}/secrets/{secret}",
                 "secretmanager.versions.access", "secretmanager.versions.add", "secretmanager.secrets.setIamPolicy")
    # SQL identity and grants are provisioned by the database owner before this job.
    steps.append({"validate_database": {"host": database_host},
                  "inspect": command("sql", "instances", "describe", args.database_instance)})
    steps.append({"validate_database_user": database_user,
                  "inspect": command("sql", "users", "list", f"--instance={args.database_instance}")})
    steps.append({"run": command("sql", "databases", "describe", args.database_name,
                                 f"--instance={args.database_instance}")})
    steps.append({"run": command("projects", "add-iam-policy-binding", args.project,
                                 f"--member=serviceAccount:{worker}", "--role=roles/cloudsql.instanceUser",
                                 "--condition=None")})
    bucket = f"gs://{args.bucket}"
    ensure(command("storage", "buckets", "describe", bucket, "--raw"),
           command("storage", "buckets", "create", bucket, f"--location={args.region}",
                   "--uniform-bucket-level-access", "--public-access-prevention",
                   f"--retention-period={args.retention_days}d", "--default-storage-class=STANDARD"))
    deny_api(f"//storage.googleapis.com/projects/_/buckets/{args.bucket}",
             "storage.objects.create", "storage.objects.get", "storage.objects.delete", "storage.buckets.setIamPolicy")
    steps.append({"lock_bucket": bucket, "minimum_days": args.retention_days,
                  "inspect": command("storage", "buckets", "describe", bucket, "--raw"),
                  "run": command("storage", "buckets", "update", bucket, "--lock-retention-period")})
    for role in ("roles/storage.objectCreator", "roles/storage.objectViewer"):
        steps.append({"run": command("storage", "buckets", "add-iam-policy-binding", bucket,
                                     f"--member=serviceAccount:{worker}", f"--role={role}")})
    key_project, location, ring, key, _ = match.groups()
    steps.append({"run": command("kms", "keys", "add-iam-policy-binding", key,
                                 f"--location={location}", f"--keyring={ring}",
                                 f"--member=serviceAccount:{worker}", "--role=roles/cloudkms.signerVerifier",
                                 project=key_project)})
    for secret in sorted(set(secrets.values())):
        steps.append({"run": command("secrets", "add-iam-policy-binding", secret,
                                     f"--member=serviceAccount:{worker}", "--role=roles/secretmanager.secretAccessor")})
        deny_worker(f"//secretmanager.googleapis.com/projects/{args.project}/secrets/{secret}",
                    "secretmanager.versions.add", "secretmanager.secrets.setIamPolicy")
        if secret in shared_secrets:
            steps.append({"run": command("secrets", "add-iam-policy-binding", secret,
                                         f"--member=serviceAccount:{args.api_service_account}",
                                         "--role=roles/secretmanager.secretAccessor")})
            deny_api(f"//secretmanager.googleapis.com/projects/{args.project}/secrets/{secret}",
                     "secretmanager.versions.add", "secretmanager.secrets.setIamPolicy")
    deny_worker(f"//storage.googleapis.com/projects/_/buckets/{args.bucket}",
                "storage.objects.delete", "storage.buckets.update", "storage.buckets.setIamPolicy")
    deny_worker("//cloudkms.googleapis.com/" + args.key_version.rsplit("/cryptoKeyVersions/", 1)[0],
                "cloudkms.cryptoKeys.setIamPolicy", "cloudkms.cryptoKeyVersions.destroy", "cloudkms.cryptoKeyVersions.update")
    environment = {"GCP_PROJECT_ID": args.project, "GCP_AUDIT_BUCKET": args.bucket,
                   "GCP_POSTGRES_HOST": database_host, "GCP_POSTGRES_DATABASE": args.database_name,
                   "GCP_POSTGRES_USER": database_user,
                   "AUDIT_KMS_PROVIDER": "gcp", "AUDIT_KMS_KEY_ID": args.key_version,
                   "AUDIT_WORKER": "on"}
    if args.audit_kid:
        environment["AUDIT_KID"] = args.audit_kid
    steps.append({"run": command(
        "run", "jobs", "deploy", args.name, f"--region={args.region}", f"--image={args.image}",
        f"--service-account={worker}", "--args=--once", "--tasks=1", "--parallelism=1", "--max-retries=1",
        "--task-timeout=3600s", "--cpu=1", "--memory=2Gi", f"--network={args.network}",
        f"--subnet={args.subnet}", "--vpc-egress=private-ranges-only",
        "--set-env-vars=" + ",".join(f"{key}={value}" for key, value in environment.items()),
        "--set-secrets=" + ",".join(f"{key}={value}:latest" for key, value in secrets.items()),
    )})
    deny_api(f"//run.googleapis.com/projects/{args.project}/locations/{args.region}/jobs/{args.name}",
             "run.jobs.update", "run.jobs.runWithOverrides", "run.jobs.setIamPolicy")
    deny_worker(f"//run.googleapis.com/projects/{args.project}/locations/{args.region}/jobs/{args.name}",
                "run.jobs.update", "run.jobs.runWithOverrides", "run.jobs.setIamPolicy")
    steps.append({"run": command("run", "jobs", "add-iam-policy-binding", args.name,
                                 f"--region={args.region}", f"--member=serviceAccount:{trigger}",
                                 "--role=roles/run.invoker")})
    schedule = [f"--location={args.region}", "--schedule=* * * * *", "--time-zone=Etc/UTC",
                "--http-method=POST", "--message-body={}",
                f"--uri=https://run.googleapis.com/v2/projects/{args.project}/locations/{args.region}/jobs/{args.name}:run",
                f"--oauth-service-account-email={trigger}",
                "--oauth-token-scope=https://www.googleapis.com/auth/cloud-platform"]
    steps.append({"ensure": command("scheduler", "jobs", "describe", args.name, f"--location={args.region}"),
                  "run": command("scheduler", "jobs", "create", "http", args.name, *schedule),
                  "update": command("scheduler", "jobs", "update", "http", args.name, *schedule)})
    return steps


def run(command, check=True):
    result = subprocess.run(command, text=True, capture_output=True)
    if check and result.returncode:
        # Secret values never appear in our argv or output, including CLI diagnostics.
        raise RuntimeError(f"cloud command failed: {' '.join(command[:4])}; inspect it with your deployment identity")
    return result


def validate_lock(metadata, minimum_days):
    policy = metadata.get("retentionPolicy", {})
    if int(policy.get("retentionPeriod", 0)) < minimum_days * 86400:
        raise ValueError("existing bucket retention is shorter than requested; extend it before deployment")
    iam = metadata.get("iamConfiguration", {})
    if not iam.get("uniformBucketLevelAccess", {}).get("enabled") or iam.get("publicAccessPrevention") != "enforced":
        raise ValueError("audit bucket must enforce uniform bucket access and public access prevention")
    return policy.get("isLocked") is True


def normalize_database_host(host):
    try:
        return str(ipaddress.ip_address(host))
    except ValueError:
        # Cloud SQL's DNS mappings are absolute names with a trailing dot.
        hostname = host.removesuffix(".").lower()
        if len(hostname) > 253 or any(not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?", label)
                                      for label in hostname.split(".")):
            raise ValueError("--database-host must be a Cloud SQL private IP or certificate hostname")
        return hostname


def validate_database(metadata, host):
    if not metadata.get("databaseVersion", "").startswith("POSTGRES_"):
        raise ValueError("the audit database must be a Cloud SQL PostgreSQL instance")
    settings = metadata.get("settings", {})
    flags = {item.get("name"): item.get("value") for item in settings.get("databaseFlags", [])}
    if flags.get("cloudsql.iam_authentication") != "on":
        raise ValueError("enable cloudsql.iam_authentication on the database before deployment")
    ip_configuration = settings.get("ipConfiguration", {})
    if ip_configuration.get("sslMode") != "ENCRYPTED_ONLY":
        raise ValueError("the IAM launcher requires ENCRYPTED_ONLY TLS; client-certificate-only mode is unsupported")
    # The API defines CA_MODE_UNSPECIFIED as GOOGLE_MANAGED_INTERNAL_CA.
    ca_mode = ip_configuration.get("serverCaMode", "CA_MODE_UNSPECIFIED")
    internal_ca = ca_mode in {"GOOGLE_MANAGED_INTERNAL_CA", "CA_MODE_UNSPECIFIED"}
    if not internal_ca and ca_mode not in {"GOOGLE_MANAGED_CAS_CA", "CUSTOMER_MANAGED_CAS_CA"}:
        raise ValueError("the Cloud SQL server CA mode must be known before deployment")
    private_addresses = {normalize_database_host(item["ipAddress"]) for item in metadata.get("ipAddresses", [])
                         if item.get("type") == "PRIVATE" and item.get("ipAddress")}
    host = normalize_database_host(host)
    try:
        ipaddress.ip_address(host)
    except ValueError:
        # Per-instance CA certificates lack DNS SANs for private services access.
        if internal_ca:
            raise ValueError("use this instance's private IP with its per-instance CA; DNS over private services access requires a shared or customer-managed CA")
        names = {normalize_database_host(item["name"]) for item in metadata.get("dnsNames", [])
                 if item.get("name") and item.get("dnsScope") == "INSTANCE"
                 and item.get("connectionType") == "PRIVATE_SERVICES_ACCESS"}
        # Older instance responses expose only the instance-level dnsName.
        if "dnsNames" not in metadata and metadata.get("dnsName"):
            names.add(normalize_database_host(metadata["dnsName"]))
        if not private_addresses or host not in names:
            raise ValueError("--database-host must match this instance's private services access certificate DNS name")
    else:
        if not internal_ca:
            raise ValueError("shared or customer-managed Cloud SQL CAs require the certificate DNS hostname and verify-full; raw IP would verify only the CA")
        if host not in private_addresses:
            raise ValueError("--database-host must match this instance's private IP")


def validate_database_user(users, expected_user):
    matches = [user for user in users
               if user.get("name", "").removesuffix(".gserviceaccount.com") == expected_user]
    if not matches or any(user.get("type") != "CLOUD_IAM_SERVICE_ACCOUNT" for user in matches):
        raise ValueError("create the worker's CLOUD_IAM_SERVICE_ACCOUNT database user and audit grants before deployment")


def apply(steps):
    for step in steps:
        if "validate_database" in step:
            validate_database(json.loads(run(step["inspect"]).stdout), step["validate_database"]["host"])
        elif "validate_database_user" in step:
            validate_database_user(json.loads(run(step["inspect"]).stdout), step["validate_database_user"])
        elif "deny_api_permission" in step or "deny_worker_permission" in step:
            result = json.loads(run(step["inspect"]).stdout)
            # An unknown result is not evidence of isolation, including conditions
            # and ancestor policies the deployment identity cannot read.
            if result.get("overallAccessState") != "CANNOT_ACCESS":
                actor = "API" if "deny_api_permission" in step else "Worker"
                permission = step.get("deny_api_permission", step.get("deny_worker_permission"))
                raise ValueError(f"{actor} isolation not established for {permission}; remove inherited grants or complete policy visibility")
        elif "lock_bucket" in step:
            metadata = json.loads(run(step["inspect"]).stdout)
            if not validate_lock(metadata, step["minimum_days"]):
                run(step["run"])
            if not validate_lock(json.loads(run(step["inspect"]).stdout), step["minimum_days"]):
                raise ValueError("bucket retention did not become locked; refusing to deploy the worker")
        elif "ensure" in step and run(step["ensure"], check=False).returncode == 0:
            if "update" in step:
                run(step["update"])
        else:
            run(step["run"])


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        steps = plan(args)
        if args.apply:
            apply(steps)
            print("Audit worker deployed. Verify a signed checkpoint and copy it to independent storage.")
        else:
            print(json.dumps(steps, indent=2))
        return 0
    except (ValueError, RuntimeError, OSError) as error:
        print(f"audit worker deployment: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
