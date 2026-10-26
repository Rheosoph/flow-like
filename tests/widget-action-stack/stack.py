#!/usr/bin/env python3
"""Bootstrap isolated infrastructure for the real API and executor benchmark."""

import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import sys


HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DEFAULT_STATE = Path("/tmp/flow-widget-stack")


def run(command, **kwargs):
    return subprocess.run(command, check=True, **kwargs)


def private_write(path, value):
    path.write_text(value)
    path.chmod(0o600)


def load(state):
    marker = state / "stack.json"
    if not marker.is_file():
        raise ValueError(f"Run {Path(__file__).name} --state {state} prepare first")
    config = json.loads(marker.read_text())
    if config.get("fixture") != "flow-like-widget-action-stack-v1":
        raise ValueError("The state directory does not belong to this fixture")
    return config


def environment(state):
    # A developer's exported deployment credentials must not affect this local stack.
    env = {key: value for key, value in os.environ.items()
           if key in ("PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "TERM",
                      "RUSTUP_HOME", "CARGO_HOME", "CARGO_TARGET_DIR", "DOCKER_HOST",
                      "DOCKER_CONTEXT", "DOCKER_CONFIG", "SSL_CERT_FILE")}
    env.update(load(state)["environment"])
    if (state / "platform.json").is_file():
        env["FLOW_LIKE_CONFIG_FILE"] = str(state / "platform.json")
    if (state / "oidc-ca.pem").is_file():
        env["SSL_CERT_FILE"] = str(state / "oidc-ca.pem")
    return env


def dependency_environment(state):
    env = environment(state)
    local_ca = (state / "oidc-ca.pem").resolve()
    for name in ("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE", "NODE_EXTRA_CA_CERTS"):
        value = os.environ.get(name)
        if value and Path(value).resolve() != local_ca:
            env[name] = value
        else:
            env.pop(name, None)
    if os.environ.get("SSL_CERT_DIR"):
        env["SSL_CERT_DIR"] = os.environ["SSL_CERT_DIR"]
    return env


def compose(state, *args, **kwargs):
    config = load(state)
    return run(["docker", "compose", "--project-name", config["project"],
                "--env-file", str(state / "native.env"), "-f", str(HERE / "compose.yml"),
                *args], env=environment(state), **kwargs)


def prepare(state, args):
    if (state / "stack.json").exists():
        load(state)
        print(f"Existing fixture configuration: {state}")
        return
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    state.chmod(0o700)
    private = run(["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt",
                   "ec_paramgen_curve:P-256"], capture_output=True).stdout
    public = run(["openssl", "pkey", "-pubout"], input=private, capture_output=True).stdout
    s3_url = f"http://127.0.0.1:{args.s3_port}"
    api_url = f"http://127.0.0.1:{args.api_port}"
    password = secrets.token_hex(24)
    env = {
        "BENCH_POSTGRES_PASSWORD": password,
        "BENCH_POSTGRES_PORT": str(args.postgres_port),
        "BENCH_REDIS_PORT": str(args.redis_port),
        "BENCH_S3_PORT": str(args.s3_port),
        "RUSTFS_ROOT_USER": "bench-root-" + secrets.token_hex(6),
        "RUSTFS_ROOT_PASSWORD": secrets.token_hex(24),
        "AWS_ACCESS_KEY_ID": "bench-api-" + secrets.token_hex(6),
        "AWS_SECRET_ACCESS_KEY": secrets.token_hex(24),
        "STS_ISSUER_ACCESS_KEY": "bench-issuer-" + secrets.token_hex(6),
        "STS_ISSUER_SECRET_KEY": secrets.token_hex(24),
        "AWS_REGION": "us-east-1", "AWS_DEFAULT_REGION": "us-east-1",
        "AWS_EC2_METADATA_DISABLED": "true", "AWS_ALLOW_HTTP": "true",
        "AWS_USE_PATH_STYLE": "true", "STORAGE_PROVIDER": "aws",
        "RUNTIME_CREDENTIALS_PROVIDER": "aws", "S3_STS_PROVIDER": "rustfs",
        "S3_INTERNAL_ENDPOINT": s3_url, "S3_PUBLIC_ENDPOINT": s3_url,
        "AWS_ENDPOINT": s3_url, "AWS_ENDPOINT_URL": s3_url,
        "STS_ENDPOINT_URL": s3_url,
        "META_BUCKET": "widget-bench-meta", "CONTENT_BUCKET": "widget-bench-content",
        "LOG_BUCKET": "widget-bench-logs", "CDN_BUCKET_NAME": "widget-bench-content",
        "CDN_BUCKET_ENDPOINT": s3_url,
        "DATABASE_URL": f"postgresql://widget_bench:{password}@127.0.0.1:{args.postgres_port}/widget_bench",
        "REDIS_URL": f"redis://127.0.0.1:{args.redis_port}",
        "REDIS_EXECUTION_QUEUE": "widget-bench:jobs",
        "EXECUTION_BACKEND": "http", "ASYNC_EXECUTION_BACKEND": "http",
        "EXECUTOR_URL": f"http://127.0.0.1:{args.runtime_port}",
        "API_PORT": str(args.api_port), "PORT": str(args.api_port),
        "RUNTIME_PORT": str(args.runtime_port), "API_URL": api_url,
        "API_BASE_URL": api_url, "CALLBACK_BASE_URL": api_url,
        "QUEUE_WORKER_ENABLED": "false", "MAX_CONCURRENT_EXECUTIONS": str(args.slots),
        "BACKEND_KEY": base64.b64encode(private).decode(),
        "BACKEND_PUB": base64.b64encode(public).decode(),
        "BACKEND_KID": "widget-bench-local",
        "SINK_TOKEN_ENCRYPTION_KEY": secrets.token_hex(32),
        "MAINTENANCE_TOKEN": secrets.token_hex(32),
        "CORS_ALLOWED_ORIGINS": "http://localhost:3440,http://127.0.0.1:3440",
        "REALTIME_ALLOWED_ORIGINS": "http://localhost:3440,http://127.0.0.1:3440",
        "RUST_LOG": "info",
    }
    env["CDN_BUCKET_ACCESS_KEY_ID"] = env["AWS_ACCESS_KEY_ID"]
    env["CDN_BUCKET_SECRET_ACCESS_KEY"] = env["AWS_SECRET_ACCESS_KEY"]
    project = "flow-widget-bench-" + secrets.token_hex(4)
    private_write(state / "stack.json", json.dumps({
        "fixture": "flow-like-widget-action-stack-v1", "project": project,
        "environment": env,
    }, indent=2) + "\n")
    private_write(state / "native.env", "".join(f"{key}={value}\n" for key, value in env.items()))
    private_write(state / "native.sh", "".join(f"export {key}={shlex.quote(value)}\n" for key, value in env.items()))
    print(f"Prepared {project} at {state}; credentials remain in that private directory.")


def migrate(state):
    destination = state / "prisma" / "schema"
    destination.mkdir(parents=True, exist_ok=True)
    for source in (REPO / "packages/api/prisma/schema").glob("*.prisma"):
        content = source.read_text().replace('provider = "cockroachdb"', 'provider = "postgresql"')
        (destination / source.name).write_text(content)
    config = state / "prisma.config.ts"
    config.write_text("export default { schema: " + json.dumps(str(destination)) +
                      ", datasource: { url: process.env.DATABASE_URL } };\n")
    prisma = REPO / "node_modules/prisma/build/index.js"
    if not prisma.is_file():
        raise ValueError("Install the workspace JavaScript dependencies before schema setup")
    # No --accept-data-loss: schema drift that needs deletion must remain visible.
    run(["bun", str(prisma), "db", "push", "--config", str(config)],
        cwd=state, env=dependency_environment(state))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", type=Path, default=DEFAULT_STATE)
    commands = parser.add_subparsers(dest="command", required=True)
    setup = commands.add_parser("prepare", help="Generate local credentials and native process settings")
    for name, default in (("postgres", 55440), ("redis", 56440), ("s3", 9044),
                          ("api", 8440), ("runtime", 8441)):
        setup.add_argument(f"--{name}-port", type=int, default=default)
    setup.add_argument("--slots", type=int, default=4)
    commands.add_parser("up", help="Start infrastructure, initialize scoped storage and apply SQL schema")
    commands.add_parser("migrate", help="Apply a fresh schema copy to this fixture database")
    commands.add_parser("down", help="Stop fixture containers; retain fixture volumes")
    commands.add_parser("status", help="Show this fixture's containers")
    sql = commands.add_parser("sql", help="Run fixture SQL from stdin against isolated Postgres")
    sql.add_argument("file", nargs="?", type=Path)
    native = commands.add_parser("run", help="Run a prebuilt API/runtime binary with fixture settings")
    native.add_argument("service", choices=("api", "runtime"))
    native.add_argument("--binary", type=Path)
    args = parser.parse_args()
    state = args.state.resolve()
    if args.command == "prepare":
        if args.slots < 1 or any(not 1 <= getattr(args, f"{name}_port") <= 65535
                                 for name in ("postgres", "redis", "s3", "api", "runtime")):
            raise ValueError("Ports must be between 1 and 65535; slots must be positive")
        prepare(state, args)
    elif args.command == "up":
        compose(state, "up", "-d", "--wait", "postgres", "redis", "object-store")
        compose(state, "run", "--rm", "object-store-init")
        migrate(state)
    elif args.command == "migrate":
        migrate(state)
    elif args.command == "down":
        compose(state, "down")
    elif args.command == "status":
        compose(state, "ps")
    elif args.command == "sql":
        payload = args.file.read_bytes() if args.file else sys.stdin.buffer.read()
        compose(state, "exec", "-T", "postgres", "psql", "-X", "-v", "ON_ERROR_STOP=1",
                "-U", "widget_bench", "-d", "widget_bench", input=payload)
    elif args.command == "run":
        binary = args.binary or REPO / "target/debug/examples/widget_action_stack"
        if not binary.is_file():
            raise ValueError(f"Build the native binary first: {binary}")
        if not (state / "platform.json").is_file():
            raise ValueError("Create the local platform.json fixture before starting services")
        os.chdir(state)
        os.execve(str(binary.resolve()), [str(binary.resolve()), args.service], environment(state))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
