#!/usr/bin/env python3
"""Preview or provision an isolated, scheduled audit worker with a locked GCS bucket."""

import argparse
import json
import re
import subprocess
import sys


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    for name in ("project", "region", "bucket", "image", "key-version", "api-service-account", "database-secret",
                 "entry-key-secret", "config-secret", "network", "subnet"):
        result.add_argument(f"--{name}", required=True)
    result.add_argument("--name", default="flow-like-audit-worker")
    result.add_argument("--encryption-secret")
    result.add_argument("--database-ca-secret", help="Mount a database CA PEM at /etc/audit-db/server-ca.pem")
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
                  args.database_secret, args.entry_key_secret, args.config_secret, args.encryption_secret, args.database_ca_secret):
        if value is not None and (not value or re.search(r"[\s,=]", value) or value.startswith("-")):
            raise ValueError("resource names must not contain whitespace, commas, equals signs or start with '-' ")

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
    for secret in (args.database_secret, args.config_secret):
        deny_api(f"//secretmanager.googleapis.com/projects/{args.project}/secrets/{secret}",
                 "secretmanager.versions.access", "secretmanager.versions.add", "secretmanager.secrets.setIamPolicy")
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
    secrets = {"DATABASE_URL": args.database_secret, "AUDIT_ENTRY_KEY": args.entry_key_secret,
               "FLOW_LIKE_CONFIG_JSON": args.config_secret}
    if args.encryption_secret:
        secrets["SINK_TOKEN_ENCRYPTION_KEY"] = args.encryption_secret
    if args.database_ca_secret:
        secrets["/etc/audit-db/server-ca.pem"] = args.database_ca_secret
    for secret in sorted(set(secrets.values())):
        steps.append({"run": command("secrets", "add-iam-policy-binding", secret,
                                     f"--member=serviceAccount:{worker}", "--role=roles/secretmanager.secretAccessor")})
        deny_worker(f"//secretmanager.googleapis.com/projects/{args.project}/secrets/{secret}",
                    "secretmanager.versions.add", "secretmanager.secrets.setIamPolicy")
    deny_worker(f"//storage.googleapis.com/projects/_/buckets/{args.bucket}",
                "storage.objects.delete", "storage.buckets.update", "storage.buckets.setIamPolicy")
    deny_worker("//cloudkms.googleapis.com/" + args.key_version.rsplit("/cryptoKeyVersions/", 1)[0],
                "cloudkms.cryptoKeys.setIamPolicy", "cloudkms.cryptoKeyVersions.destroy", "cloudkms.cryptoKeyVersions.update")
    environment = {"GCP_PROJECT_ID": args.project, "GCP_AUDIT_BUCKET": args.bucket,
                   "AUDIT_KMS_PROVIDER": "gcp", "AUDIT_KMS_KEY_ID": args.key_version,
                   "AUDIT_WORKER": "on"}
    steps.append({"run": command(
        "run", "jobs", "deploy", args.name, f"--region={args.region}", f"--image={args.image}",
        f"--service-account={worker}", "--args=--once", "--tasks=1", "--parallelism=1", "--max-retries=1",
        "--task-timeout=3600s", "--cpu=1", "--memory=1Gi", f"--network={args.network}",
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


def apply(steps):
    for step in steps:
        if "deny_api_permission" in step or "deny_worker_permission" in step:
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
