"""Validate the optional identity provider without displaying credentials."""
import json
from pathlib import Path

from oidc_setup import CLIENT, REALM, origin, verify_certificates

ROOT = Path(__file__).resolve().parents[1]


def validate(values, config):
    services = config["services"]
    mode = values.get("OIDC_MODE", "external")
    bundled = {"oidc", "oidc-db"} & services.keys()
    if mode == "external":
        return ["OIDC_MODE=external must not include the bundled OIDC overlay"] if bundled else []
    if mode != "bundled":
        return ["OIDC_MODE must be external or bundled"]
    if bundled != {"oidc", "oidc-db"}:
        return ["OIDC_MODE=bundled requires docker-compose.oidc.yml in COMPOSE_FILE"]
    errors = []
    try:
        public_url, hostname, port = origin(values.get("OIDC_PUBLIC_URL", ""))
    except ValueError as error:
        return [str(error)]
    oidc = services["oidc"]
    identity = oidc.get("environment", {})
    api = services["api"].get("environment", {})
    if (values.get("OIDC_HOSTNAME") != hostname or str(values.get("OIDC_PORT")) != str(port)
            or identity.get("KC_HOSTNAME") != public_url or str(identity.get("KC_HTTPS_PORT")) != str(port)
            or hostname not in oidc.get("networks", {}).get("app", {}).get("aliases", [])):
        errors.append("OIDC hostname and TLS port must match OIDC_PUBLIC_URL inside and outside Compose")
    ports = oidc.get("ports", [])
    if len(ports) != 1 or str(ports[0].get("target")) != str(port) or str(ports[0].get("published")) != str(port):
        errors.append("Bundled OIDC must publish only its configured HTTPS port")
    if hostname.endswith(".localhost") and (len(ports) != 1 or ports[0].get("host_ip") != "127.0.0.1"):
        errors.append("Local OIDC certificates require OIDC_BIND_ADDRESS=127.0.0.1")
    if identity.get("KC_HTTP_ENABLED") != "false" or oidc.get("command") != ["start", "--import-realm"]:
        errors.append("Bundled OIDC requires HTTPS and Keycloak start --import-realm")
    if identity.get("KC_HTTP_MANAGEMENT_HOST") != "127.0.0.1" or services["oidc-db"].get("ports"):
        errors.append("OIDC management and database listeners must remain private")
    credentials = [values.get(key, "") for key in ("OIDC_DB_PASSWORD", "OIDC_ADMIN_PASSWORD", "OIDC_BOOTSTRAP_PASSWORD")]
    if any(len(value) < 32 for value in credentials) or len(set(credentials)) != 3:
        errors.append("OIDC database, administrator and first user require distinct generated passwords")
    if api.get("OIDC_CA_CERT_FILE") != "/etc/flow-like/oidc-ca.pem":
        errors.append("Bundled OIDC requires the API's scoped CA mount")
    if (api.get("FLOW_LIKE_CONFIG_FILE") != "/app/flow-like.config.json"
            or api.get("FLOW_LIKE_CONFIG_JSON") or api.get("FLOW_LIKE_CONFIG_SECRET_REF")):
        errors.append("Bundled OIDC requires its generated file-based hub configuration")
    for name, service in services.items():
        if name not in {"oidc", "oidc-db"} and any(value and value in service.get("environment", {}).values() for value in credentials):
            errors.append(f"{name} must not receive OIDC database or bootstrap passwords")
    errors.extend(validate_files(values, config, public_url, hostname))
    return errors


def validate_files(values, config, public_url, hostname):
    required = ("OIDC_REALM_FILE", "OIDC_TLS_CERT_FILE", "OIDC_TLS_KEY_FILE", "OIDC_CA_CERT_FILE")
    paths = {key: ROOT / values.get(key, "") for key in required}
    missing = [key for key, path in paths.items() if not path.is_file()]
    if missing:
        return ["Bundled OIDC requires regular files for " + ", ".join(missing)]
    try:
        verify_certificates(paths["OIDC_TLS_CERT_FILE"], paths["OIDC_TLS_KEY_FILE"], paths["OIDC_CA_CERT_FILE"], hostname)
        runtime_file = config.get("configs", {}).get("flowlike_runtime_config", {}).get("file", "")
        hub = json.loads(Path(runtime_file).read_text())
        openid = hub["authentication"]["openid"]
        if not isinstance(openid, dict):
            raise ValueError("OpenID configuration must be an object")
        issuer = public_url + "/realms/" + REALM
        expected = {"authority": issuer, "issuer": issuer, "client_id": CLIENT, "audience": CLIENT,
                    "discovery_url": issuer + "/.well-known/openid-configuration",
                    "jwks_url": issuer + "/protocol/openid-connect/certs",
                    "user_info_url": issuer + "/protocol/openid-connect/userinfo"}
        if any(openid.get(key) != value for key, value in expected.items()):
            return ["Bundled OIDC hub configuration must match its issuer, client, audience and endpoints"]
        realm = json.loads(paths["OIDC_REALM_FILE"].read_text())
        if not isinstance(realm, dict):
            raise ValueError("OIDC realm must be an object")
        if realm.get("realm") != REALM:
            return ["The bundled OIDC import must name the flow-like realm"]
    except (OSError, ValueError, KeyError, TypeError):
        return ["Bundled OIDC TLS or hub configuration is invalid; check certificates, hostname and generated files"]
    return []
