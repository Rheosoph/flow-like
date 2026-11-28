#!/usr/bin/env python3
"""Sends the Cognito email templates in templates.json to the hub's user pool.

The templates are rendered from the API's email layout (packages/api/src/mail/templates.rs); a
test there fails when this file is out of date. Regenerate it with

    UPDATE_COGNITO_TEMPLATES=1 cargo test -p flow-like-api --lib cognito

UpdateUserPool resets every setting it is not given, so this script copies the pool's current
settings into the request and changes only the two email templates: the code Cognito sends to
confirm an address or reset a password, and the sign-in details of an account an administrator
creates. Without --apply it prints what would change. With --apply it saves the current settings,
updates the pool, reads it back, and restores the saved settings if anything besides the
templates moved.

    python3 -I tools/cognito-email/apply.py
    python3 -I tools/cognito-email/apply.py --apply
"""

import argparse
import copy
import json
import pathlib
import subprocess
import sys
import tempfile

POOL = "eu-west-1_0fYxn5Pn7"
REGION = "eu-west-1"
TEMPLATES = pathlib.Path(__file__).resolve().parent / "templates.json"

# DescribeUserPool names on the left, the UpdateUserPool parameters they feed on the right.
UPDATABLE = {
    "Policies": "Policies",
    "DeletionProtection": "DeletionProtection",
    "LambdaConfig": "LambdaConfig",
    "AutoVerifiedAttributes": "AutoVerifiedAttributes",
    "SmsVerificationMessage": "SmsVerificationMessage",
    "EmailVerificationMessage": "EmailVerificationMessage",
    "EmailVerificationSubject": "EmailVerificationSubject",
    "VerificationMessageTemplate": "VerificationMessageTemplate",
    "SmsAuthenticationMessage": "SmsAuthenticationMessage",
    "UserAttributeUpdateSettings": "UserAttributeUpdateSettings",
    "MfaConfiguration": "MfaConfiguration",
    "DeviceConfiguration": "DeviceConfiguration",
    "EmailConfiguration": "EmailConfiguration",
    "SmsConfiguration": "SmsConfiguration",
    "UserPoolTags": "UserPoolTags",
    "AdminCreateUserConfig": "AdminCreateUserConfig",
    "UserPoolAddOns": "UserPoolAddOns",
    "AccountRecoverySetting": "AccountRecoverySetting",
    "Name": "PoolName",
    "UserPoolTier": "UserPoolTier",
}
TEMPLATE_FIELDS = {
    "VerificationMessageTemplate": {"EmailMessage", "EmailSubject"},
    "AdminCreateUserConfig": {"InviteMessageTemplate"},
    "EmailVerificationMessage": None,
    "EmailVerificationSubject": None,
}
VOLATILE = {"LastModifiedDate", "EstimatedNumberOfUsers"}
MAX_MESSAGE_CHARS = 20_000
MAX_SUBJECT_CHARS = 140


def aws(*args):
    result = subprocess.run(
        ["aws", "cognito-idp", *args, "--region", REGION, "--output", "json"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit(f"aws cognito-idp {args[0]} failed:\n{result.stderr.strip()}")
    return json.loads(result.stdout) if result.stdout.strip() else {}


def describe():
    return aws("describe-user-pool", "--user-pool-id", POOL)["UserPool"]


def mfa_config():
    config = aws("get-user-pool-mfa-config", "--user-pool-id", POOL)
    config.pop("ResponseMetadata", None)
    return config


def send(command, request):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as file:
        json.dump(request, file)
        path = pathlib.Path(file.name)
    try:
        aws(command, "--cli-input-json", f"file://{path}")
    finally:
        path.unlink(missing_ok=True)


def load_templates():
    templates = json.loads(TEMPLATES.read_text())
    verification, invitation = templates["verification"], templates["invitation"]
    problems = []
    if "{####}" not in verification["html"]:
        problems.append("the verification message has no {####}")
    if "{####}" not in invitation["html"] or "{username}" not in invitation["html"]:
        problems.append("the invitation needs {username} and {####}")
    for name, template in templates.items():
        if len(template["html"]) > MAX_MESSAGE_CHARS:
            problems.append(f"the {name} message has more than {MAX_MESSAGE_CHARS} characters")
        if len(template["subject"]) > MAX_SUBJECT_CHARS:
            problems.append(f"the {name} subject has more than {MAX_SUBJECT_CHARS} characters")
    if problems:
        raise SystemExit("templates.json can't be sent: " + "; ".join(problems))
    return verification, invitation


def update_request(pool, templates=None):
    """The UpdateUserPool request that keeps every setting of `pool`, with `templates` if given."""
    request = {"UserPoolId": POOL}
    for described, parameter in UPDATABLE.items():
        if described in pool:
            request[parameter] = copy.deepcopy(pool[described])
    admin = request.setdefault("AdminCreateUserConfig", {})
    password = request.get("Policies", {}).get("PasswordPolicy", {})
    if password.get("TemporaryPasswordValidityDays") is not None:
        # Cognito refuses the old field beside the one that replaced it.
        admin.pop("UnusedAccountValidityDays", None)
    if templates:
        verification, invitation = templates
        message = request.setdefault("VerificationMessageTemplate", {})
        message["EmailSubject"] = verification["subject"]
        message["EmailMessage"] = verification["html"]
        if "EmailVerificationSubject" in request:
            request["EmailVerificationSubject"] = verification["subject"]
        if "EmailVerificationMessage" in request:
            request["EmailVerificationMessage"] = verification["html"]
        invite = admin.setdefault("InviteMessageTemplate", {})
        invite["EmailSubject"] = invitation["subject"]
        invite["EmailMessage"] = invitation["html"]
    return request


def without_templates(pool):
    kept = {}
    for key, value in pool.items():
        if key in VOLATILE:
            continue
        fields = TEMPLATE_FIELDS.get(key, set())
        if fields is None:
            continue
        if fields and isinstance(value, dict):
            value = {field: inner for field, inner in value.items() if field not in fields}
            if key == "AdminCreateUserConfig":
                value.pop("UnusedAccountValidityDays", None)
        kept[key] = value
    return kept


def changes(before, after):
    keys = sorted(set(before) | set(after))
    return [key for key in keys if before.get(key) != after.get(key)]


def summary(request):
    message = request.get("VerificationMessageTemplate", {})
    invite = request.get("AdminCreateUserConfig", {}).get("InviteMessageTemplate", {})
    return {
        "verification subject": message.get("EmailSubject"),
        "verification message": f"{len(message.get('EmailMessage', ''))} characters",
        "invitation subject": invite.get("EmailSubject"),
        "invitation message": f"{len(invite.get('EmailMessage', ''))} characters",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--apply", action="store_true", help="update the pool")
    arguments = parser.parse_args()

    templates = load_templates()
    before, before_mfa = describe(), mfa_config()
    current, wanted = update_request(before), update_request(before, templates)
    moved = changes(current, wanted)
    print(f"Pool {POOL} ({before.get('Name')}), parameters that change: {', '.join(moved) or 'none'}")
    print(json.dumps(summary(wanted), indent=2))
    if not moved:
        print("The pool already sends these templates.")
        return
    if not arguments.apply:
        print("Dry run: pass --apply to update the pool.")
        return

    backup = pathlib.Path(tempfile.mkdtemp(prefix="cognito-email-"))
    (backup / "pool.json").write_text(json.dumps(before, indent=2, default=str))
    (backup / "mfa.json").write_text(json.dumps(before_mfa, indent=2))
    print(f"Saved the current settings in {backup}")

    send("update-user-pool", wanted)
    after, after_mfa = describe(), mfa_config()
    drifted = changes(without_templates(before), without_templates(after))
    verification, invitation = templates
    landed = (
        after.get("VerificationMessageTemplate", {}).get("EmailMessage") == verification["html"]
        and after.get("AdminCreateUserConfig", {})
        .get("InviteMessageTemplate", {})
        .get("EmailMessage")
        == invitation["html"]
    )
    if drifted or after_mfa != before_mfa:
        print(f"Settings besides the templates moved ({', '.join(drifted) or 'MFA'}); restoring them.")
        send("update-user-pool", update_request(before))
        if after_mfa != before_mfa:
            send("set-user-pool-mfa-config", {"UserPoolId": POOL, **before_mfa})
        restored = changes(without_templates(before), without_templates(describe()))
        print(f"Restored; still different: {', '.join(restored) or 'nothing'}")
        sys.exit(1)
    if not landed:
        sys.exit("The pool reports other templates than the ones sent.")
    print("Updated: only the email templates changed.")


if __name__ == "__main__":
    main()
