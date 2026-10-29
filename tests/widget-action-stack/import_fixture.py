#!/usr/bin/env python3
"""Import sanitized app metadata into the isolated widget benchmark stack."""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import sys
from urllib.parse import urlsplit

import stack


def sql_value(value):
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, (dict, list)):
        return sql_value(json.dumps(value, separators=(",", ":"))) + "::jsonb"
    if isinstance(value, (int, float)):
        return str(value)
    if "\0" in value:
        raise ValueError("Fixture SQL values must not contain NUL")
    return "'" + value.replace("'", "''") + "'"


def insert(table, values):
    # Table/column names come only from the fixed maps below, never the imported files.
    names = list(values)
    columns = ",".join(f'"{name}"' for name in [*names, "updatedAt"])
    fields = ",".join([*(sql_value(values[name]) for name in names), "NOW()"])
    updates = ",".join(f'"{name}"=EXCLUDED."{name}"' for name in names if name != "id")
    return (f'INSERT INTO "{table}" ({columns}) VALUES ({fields}) '
            f'ON CONFLICT ("id") DO UPDATE SET {updates},"updatedAt"=NOW();')


def version(value):
    if isinstance(value, (list, tuple)):
        return ".".join(str(part) for part in value)
    return value


def generated_table(fixture, index):
    content = (fixture / index.get("content_directory", "content")).resolve()
    database = index["database"]
    table_path = database["path"]
    table = (content / table_path).resolve()
    expected_prefix = f'apps/{index["app_id"]}/storage/db/'
    if (not content.is_relative_to(fixture) or not table.is_relative_to(content / expected_prefix)
            or ".." in Path(table_path).parts or not table_path.startswith(expected_prefix)
            or not table_path.endswith(".lance") or not table.is_dir()
            or database.get("synthetic") is not True):
        raise ValueError("The fixture must identify one generated Lance table inside its app prefix")
    return table, table_path, database


def graph_identity(index):
    def digest(rows):
        return hashlib.sha256(json.dumps(sorted(rows), separators=(",", ":")).encode()).hexdigest()
    return {
        "graph_source_sha256": digest([(board["id"], board["source_sha256"]) for board in index["boards"]]),
        "graph_fixture_sha256": digest([(item["path"], item["fixture_sha256"]) for item in index["objects"]]),
    }


def fixture_sql(index, user_id):
    app_id = index["app_id"]
    owner = app_id + "-bench-owner"
    viewer = app_id + "-bench-viewer"
    statements = ["BEGIN;", insert("App", {
        "id": app_id, "status": "ACTIVE", "visibility": "PRIVATE", "executionMode": "ANY",
    })]
    for role_id, name, permissions in ((owner, "Owner", 1), (viewer, "User", 534528)):
        statements.append(insert("Role", {
            "id": role_id, "appId": app_id, "name": name, "permissions": permissions,
        }))
    statements.append(
        f'UPDATE "App" SET "ownerRoleId"={sql_value(owner)},"defaultRoleId"={sql_value(viewer)} '
        f'WHERE "id"={sql_value(app_id)};')
    statements.append(insert("Membership", {
        "id": app_id + "-bench-member", "appId": app_id, "roleId": owner, "userId": user_id,
    }))
    statements.append(insert("Meta", {
        "id": app_id + "-bench-meta", "appId": app_id, "lang": "en",
        "name": "Widget action benchmark", "description": "Local fixture with synthetic rental records",
    }))
    for widget in index["widgets"]:
        statements.append(insert("Widget", {
            "id": widget["id"], "appId": app_id, "version": version(widget.get("version")),
        }))
        statements.append(insert("Meta", {
            "id": widget["id"] + "-bench-meta", "widgetId": widget["id"], "lang": "en",
            "name": widget["name"], "description": widget.get("description"),
        }))
    page_ids = set()
    for page in index["pages"]:
        if page["id"] in page_ids:
            raise ValueError("fixture-index.json must list each current page exactly once")
        page_ids.add(page["id"])
        statements.append(insert("Page", {
            "id": page["id"], "appId": app_id, "name": page["name"],
            "description": page.get("title"), "boardId": page["board_id"],
            "version": version(page.get("version")),
        }))
    for event in index["events"]:
        config = event.get("config")
        if isinstance(config, list):
            config = {"base64": base64.b64encode(bytes(config)).decode()} if config else None
        statements.append(insert("Event", {
            "id": event["id"], "appId": app_id, "name": event["name"],
            "description": event.get("description"), "eventType": event["event_type"],
            "active": event.get("active", True), "priority": event.get("priority", 0),
            "boardId": event["board_id"], "boardVersion": version(event.get("board_version")),
            "nodeId": event.get("node_id", ""), "pageId": event.get("default_page_id"),
            "route": event.get("route"), "isDefault": event.get("is_default", False),
            "eventVersion": version(event.get("event_version", [0, 0, 0])),
            "executionMode": "Remote", "exposure": event.get("exposure", "PUBLIC").upper(),
            "variables": event.get("variables") or None, "config": config,
            "inputs": event.get("inputs") or None,
        }))
    statements.append("COMMIT;")
    return "\n".join(statements) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path, help="Sanitized output directory containing fixture-index.json")
    parser.add_argument("--state", type=Path, default=stack.DEFAULT_STATE)
    parser.add_argument("--user-id", default="widget-bench-user")
    args = parser.parse_args()
    state, fixture = args.state.resolve(), args.fixture.resolve()
    index = json.loads((fixture / "fixture-index.json").read_text())
    metadata = (fixture / index.get("metadata_directory", "metadata")).resolve()
    if not metadata.is_relative_to(fixture) or not (metadata / "manifest.app").is_file():
        raise ValueError("Sanitized metadata must be inside the fixture directory and contain manifest.app")
    env = stack.environment(state)
    endpoint = env["S3_PUBLIC_ENDPOINT"]
    if urlsplit(endpoint).hostname not in ("127.0.0.1", "localhost", "::1"):
        raise ValueError("This importer only writes to a loopback S3 endpoint")
    table, table_path, database = generated_table(fixture, index)
    sql = fixture_sql(index, args.user_id)
    seed_user = state / "user.sql"
    if seed_user.is_file():
        stack.compose(state, "exec", "-T", "postgres", "psql", "-X", "-v", "ON_ERROR_STOP=1",
                      "-U", "widget_bench", "-d", "widget_bench", input=seed_user.read_bytes())
    stack.compose(state, "exec", "-T", "postgres", "psql", "-X", "-v", "ON_ERROR_STOP=1",
                  "-U", "widget_bench", "-d", "widget_bench", input=sql.encode())
    # Recompile from source with the local catalog. Hosted compiled artifacts cannot be replayed.
    stack.run(["aws", "--endpoint-url", endpoint, "s3", "sync", str(metadata),
               f's3://{env["META_BUCKET"]}/apps/{index["app_id"]}/',
               "--exclude", "compiled/*", "--only-show-errors"], env=env)
    # A shorter regenerated dataset must remove prior manifests, or Lance would open
    # the higher version left by an earlier profile. Deletion is scoped to this table.
    stack.run(["aws", "--endpoint-url", endpoint, "s3", "sync", str(table),
               f's3://{env["CONTENT_BUCKET"]}/{table_path}/', "--delete", "--only-show-errors"], env=env)
    result = {"app_id": index["app_id"], "widgets": len(index["widgets"]),
              "pages": len(index["pages"]), "events": len(index["events"]),
              "fixture_sha256": index["fixture_sha256"],
              "database": {key: database[key] for key in (
                  "profile", "rows", "fragments", "physical_rows", "deletion_files", "records_sha256",
                  "data_storage_version", "pylance")}}
    result["database"]["layout"] = database.get("layout", "fragmented")
    result["database"]["schema_sha256"] = hashlib.sha256(database["schema"].encode()).hexdigest()
    result.update(graph_identity(index))
    (state / "import-result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
