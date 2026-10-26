#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["protobuf>=6.33", "lz4>=4.4", "pylance==8.0.0", "pyarrow>=18"]
# ///
"""Prepare a private app export with synthetic records for the local stack."""

import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


APP_ID = "j2qlhgh3fvdngiya3b5hc1y4"
BOARD_ID = "lzt37frs7aixhkvjnkaeas3o"
TABLE = "immoatlas_shared_demo_core_v2"
SENSITIVE = re.compile(r"(?:password|secret|credential|access.?token|api.?key|private.?key|authorization)", re.I)
EMAIL = re.compile(r"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
URL = re.compile(r"(?:https?|s3)://[^\s\"'<>]+")
TOKEN = re.compile(r"(?:AKIA|ASIA)[A-Z0-9]{16}|eyJ[A-Za-z0-9_-]{12,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+")


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    path.chmod(0o600)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Codec:
    def __init__(self, output):
        import lz4.block
        self.block = lz4.block
        proto_root = Path(__file__).resolve().parents[2] / "packages/types/proto/protobufs"
        generated = output / "proto-python"
        generated.mkdir(parents=True, exist_ok=True)
        subprocess.run(["protoc", "-I", str(proto_root), f"--python_out={generated}",
                        *(str(p) for p in sorted(proto_root.glob("*.proto")))], check=True)
        sys.path.insert(0, str(generated))
        self.classes = {
            ".app": importlib.import_module("app_pb2").App,
            ".board": importlib.import_module("board_pb2").Board,
            ".event": importlib.import_module("event_pb2").Event,
            ".page": importlib.import_module("a2ui_pb2").Page,
        }

    def decode(self, path):
        packed = path.read_bytes()
        if len(packed) < 4 or int.from_bytes(packed[:4], "little") > 128 * 1024 * 1024:
            raise ValueError(f"Invalid or oversized metadata object: {path.name}")
        raw = self.block.decompress(packed)
        if path.suffix == ".widget":
            return json.loads(raw)
        message = self.classes[path.suffix]()
        message.ParseFromString(raw)
        return message

    def encode(self, value):
        raw = (json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
               if isinstance(value, dict) else value.SerializeToString(deterministic=True))
        return self.block.compress(raw, store_size=True)


class Sanitizer:
    def __init__(self, user_id):
        self.user_id = user_id
        self.counts = Counter()

    def string(self, value):
        # Schema identifiers and static project links carry no caller credentials.
        def url(match):
            candidate = match.group()
            if candidate.startswith(("https://json-schema.org/", "http://json-schema.org/")):
                return candidate
            self.counts["external_urls"] += 1
            return "https://example.invalid/fixture"
        value = URL.sub(url, value)
        value, count = EMAIL.subn("fixture@example.invalid", value)
        self.counts["email_values"] += count
        value, count = TOKEN.subn("LOCAL_FIXTURE_REDACTED", value)
        self.counts["credential_patterns"] += count
        return value

    def json(self, value):
        if isinstance(value, dict):
            return {key: ("" if SENSITIVE.search(key) and isinstance(item, str)
                          else self.json(item)) for key, item in value.items()}
        if isinstance(value, list):
            return [self.json(item) for item in value]
        if isinstance(value, str):
            return self.string(value)
        return value

    def message(self, message):
        from google.protobuf.descriptor import FieldDescriptor
        kind = message.DESCRIPTOR.name
        if kind == "App":
            del message.authors[:]
            message.authors.append(self.user_id)
        if kind in ("Board", "Layer"):
            message.ClearField("comments")
        if kind in ("Board", "Layer", "Node", "Variable"):
            message.ClearField("hash")
        if kind == "Board":
            message.ClearField("internal_refs")
        if kind == "Variable" and (message.secret or SENSITIVE.search(message.name)):
            if message.default_value:
                self.counts["secret_defaults"] += 1
            message.ClearField("default_value")
        if kind == "Pin" and (message.options.sensitive or SENSITIVE.search(message.name)):
            if message.default_value:
                self.counts["secret_defaults"] += 1
            message.ClearField("default_value")
        for field, value in list(message.ListFields()):
            if field.type == FieldDescriptor.TYPE_MESSAGE:
                if field.is_repeated and field.message_type.GetOptions().map_entry:
                    value_field = field.message_type.fields_by_name["value"]
                    if value_field.type == FieldDescriptor.TYPE_MESSAGE:
                        for item in value.values():
                            self.message(item)
                    elif value_field.type == FieldDescriptor.TYPE_STRING:
                        for key in list(value):
                            value[key] = self.string(value[key])
                elif field.is_repeated:
                    for item in value:
                        self.message(item)
                else:
                    self.message(value)
            elif field.type == FieldDescriptor.TYPE_BYTES and value:
                try:
                    sanitized = self.json(json.loads(value))
                except (ValueError, UnicodeDecodeError) as error:
                    raise ValueError(f"Unrecognized binary metadata field {kind}.{field.name}") from error
                setattr(message, field.name, json.dumps(sanitized, ensure_ascii=False,
                                                       separators=(",", ":")).encode())
            elif field.type == FieldDescriptor.TYPE_STRING:
                if field.is_repeated:
                    for index in range(len(value)):
                        value[index] = self.string(value[index])
                else:
                    setattr(message, field.name, self.string(value))
        return message


def version(message):
    return [message.major, message.minor, message.patch]


def catalog_sources():
    """Map literal node constructors for a static coverage check, without building Rust."""
    root = Path(__file__).resolve().parents[2] / "packages/catalog"
    constructors = re.compile(r'(?:Node::new|array_node|bytes_node|format_node)\(\s*"([^"]+)"')
    sources = {}
    for path in sorted(root.glob("*/src/**/*.rs")):
        content = path.read_text().split("#[cfg(test)]", 1)[0]
        if "register_node]" not in content:
            continue
        package = path.relative_to(root).parts[0]
        for name in constructors.findall(content):
            sources.setdefault(name, set()).add(package)
    return sources


def catalog_coverage(node_types, sources):
    packages = {name: sorted(sources.get(name, set())) for name in sorted(node_types)}
    covered = {name: values for name, values in packages.items()
               if any(package == "data" or package.startswith("std-") for package in values)}
    return {
        "method": "Static literal constructor inventory; runtime compilation remains the authoritative check",
        "node_type_count": len(node_types), "covered_node_type_count": len(covered),
        "required_packages": sorted({package for values in packages.values() for package in values}),
        "outside_benchmark_catalog": sorted(set(node_types) - set(covered)),
        "source_packages_by_node_type": packages,
    }


def event_index(event):
    if event.variables or event.inputs or event.variants or event.HasField("canary"):
        raise ValueError("This fixture expects page events without overrides or input defaults")
    return {
        "id": event.id, "name": event.name, "description": event.description,
        "board_id": event.board_id, "node_id": event.node_id,
        "board_version": version(event.board_version) if event.HasField("board_version") else None,
        "event_version": version(event.event_version), "variables": {}, "config": list(event.config),
        "active": event.active, "priority": event.priority, "event_type": event.event_type,
        "default_page_id": event.default_page_id or None, "route": event.route or None,
        "is_default": event.is_default, "execution_mode": event.execution_mode or "Local",
        "exposure": event.exposure or "PUBLIC", "inputs": [], "variants": [], "canary": None,
        "notes": None,
        "created_at": {"secs_since_epoch": event.created_at.seconds, "nanos_since_epoch": event.created_at.nanos},
        "updated_at": {"secs_since_epoch": event.updated_at.seconds, "nanos_since_epoch": event.updated_at.nanos},
    }


def prepare_metadata(source, output, user_id):
    codec = Codec(output)
    sanitizer = Sanitizer(user_id)
    manifest = codec.decode(source / "manifest.app")
    if manifest.id != APP_ID:
        raise ValueError("The fixture builder expects the exported widget investigation app")
    index = {"app_id": manifest.id, "metadata_directory": "metadata", "content_directory": "content",
             "events": [], "pages": [], "widgets": [], "boards": [], "objects": []}
    sources = catalog_sources()
    # Import only current source objects. Local compilation must create its own artifacts.
    paths = [source / "manifest.app", *sorted(source.glob("*.board")),
             *sorted(source.glob("*.widget")), *sorted((source / "events").glob("*.event"))]
    paths.extend(sorted(path for path in source.glob("_*/*.page")))
    for path in paths:
        value = codec.decode(path)
        if isinstance(value, dict):
            value = sanitizer.json(value)
            value["dataModel"] = []
        else:
            sanitizer.message(value)
        relative = path.relative_to(source)
        target = output / "metadata" / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        packed = codec.encode(value)
        target.write_bytes(packed)
        target.chmod(0o600)
        index["objects"].append({"path": str(relative), "source_sha256": digest(path.read_bytes()),
                                 "fixture_sha256": digest(packed)})
        if path.suffix == ".event":
            index["events"].append(event_index(value))
        elif path.suffix == ".widget":
            index["widgets"].append({key: value.get(key) for key in ("id", "name", "description", "version")})
        elif path.suffix == ".page":
            index["pages"].append({"id": value.id, "name": value.name, "title": value.title,
                                   "board_id": value.board_id, "version": version(value.version),
                                   "on_load_event_id": value.on_load_event_id, "route": value.route})
        elif path.suffix == ".board":
            nodes = list(value.nodes.values()) + [node for layer in value.layers.values()
                                                  for node in layer.nodes.values()]
            unique_nodes = {node.id: node for node in nodes}
            node_types = dict(sorted(Counter(node.name for node in unique_nodes.values()).items()))
            index["boards"].append({"id": value.id,
                                    "version": [value.version_major, value.version_minor, value.version_patch],
                                    "node_count": len(unique_nodes), "node_types": node_types,
                                    "static_catalog_coverage": catalog_coverage(node_types, sources),
                                    "source_sha256": digest(path.read_bytes()), "fixture_sha256": digest(packed)})
    index["sanitization"] = {"replacements": dict(sanitizer.counts), "user_id": user_id,
                              "source_objects_only": True, "customer_database_copied": False,
                              "compiled_artifacts_imported": False}
    return index


def synthetic_records(app_id, people, history_rows, extra_documents=0):
    records = []
    timestamp = "2026-01-01T00:00:00Z"

    def add(kind, identifier, **payload):
        body = {"id": identifier, "entity_type": kind, "schema_version": 2, "revision": 1,
                "deleted": False, "created_at": timestamp, "updated_at": timestamp,
                "app_scope_id": app_id, "lifecycle": "active", **payload}
        records.append({"id": identifier, "app_scope_id": app_id, "entity_type": kind,
                        "schema_version": 2, "revision": 1, "deleted": False,
                        "created_at": timestamp, "updated_at": timestamp,
                        "payload_json": json.dumps(body, ensure_ascii=False, separators=(",", ":"))})

    for person in range(1, people + 1):
        contact = f"bench-contact-{person:04d}"
        add("contact", contact, contact_kind="person", name=f"Benchmark Person {person:04d}",
            display_name=f"Benchmark Person {person:04d}",
            email=f"person-{person:04d}@example.invalid", phone="", company="", notes="",
            roles=["tenant"], is_tenant=True, is_manager=False, is_contractor=False)
        for row in range(1, history_rows + 1):
            suffix = f"{person:04d}-{row:02d}"
            prop, unit, tenancy = (f"bench-{kind}-{suffix}" for kind in ("property", "unit", "tenancy"))
            active = row == history_rows
            start, end = f"{2010 + row:04d}-01-01", "" if active else f"{2011 + row:04d}-01-01"
            add("property", prop, name=f"Benchmark Property {suffix}", property_kind="building",
                address_line=f"Example Street {row}", postal_code="00000", city="Fixture City")
            add("unit", unit, property_id=prop, name=f"Benchmark Unit {suffix}",
                label=f"Benchmark Unit {suffix}", unit_kind="apartment", occupancy_mode="whole_unit")
            add("tenancy", tenancy, property_id=prop, unit_id=unit, label=f"Benchmark Lease {suffix}",
                name=f"Benchmark Lease {suffix}", start_date=start, end_date=end,
                lifecycle="active" if active else "ended", cold_rent_eur="700", warm_rent_eur="900",
                ancillary_prepayment_eur="200", deposit_eur="2100", payment_day="3",
                rent_check_mode="manual", rent_check_status="pending", rent_confirmed_month="")
            add("tenancy_member", f"bench-member-{suffix}", contact_id=contact, tenancy_id=tenancy,
                property_id=prop, unit_id=unit, role="tenant", start_date=start, end_date=end,
                lifecycle="active" if active else "ended", is_primary_contact=True)
    for document in range(extra_documents):
        add("document", f"bench-document-{document:04d}", name=f"Fixture document {document:04d}",
            file_name=f"fixture-{document:04d}.txt", mime_type="text/plain", size_bytes=0,
            storage_ref="", category="other", version_number=1)
    return records


def seed_content(output, index, people, history_rows, profile, layout):
    import lance
    import pyarrow as pa
    if profile == "production":
        people, history_rows = 2, 4
    records = synthetic_records(index["app_id"], people, history_rows,
                                extra_documents=13 if profile == "production" else 0)
    # These Arrow types and nullability come from the hosted version 62 manifest.
    schema = pa.schema([
        pa.field("app_scope_id", pa.large_string(), nullable=False),
        pa.field("created_at", pa.timestamp("ms", tz="UTC"), nullable=False),
        pa.field("deleted", pa.bool_(), nullable=False),
        pa.field("entity_type", pa.large_string(), nullable=False),
        pa.field("id", pa.large_string(), nullable=False,
                 metadata={b"lance-schema:unenforced-primary-key:position": b"1"}),
        pa.field("payload_json", pa.large_string(), nullable=False),
        pa.field("revision", pa.uint64(), nullable=False),
        pa.field("schema_version", pa.uint64(), nullable=False),
        pa.field("updated_at", pa.timestamp("ms", tz="UTC"), nullable=False),
    ])
    relative = Path("apps") / index["app_id"] / "storage/db" / f"{TABLE}.lance"
    location = output / "content" / relative
    location.parent.mkdir(parents=True, exist_ok=True)
    # Recreate only this generated table so repeated preparations do not retain old data files.
    if location.exists():
        import shutil
        shutil.rmtree(location)

    def batch(rows):
        timestamp = datetime(2026, 1, 1, tzinfo=timezone.utc)
        return pa.Table.from_pylist([{**row, "created_at": timestamp, "updated_at": timestamp}
                                    for row in rows], schema=schema)

    if profile == "production" and layout == "fragmented":
        # The source has 36 fragments, including three deletion bitmaps: 58 physical/47 live rows.
        shapes = [(1, 0)] * 28 + [(4, 4), (9, 2), (1, 5)] + [(1, 0)] * 5
    elif layout == "fragmented":
        count = min(36, len(records))
        base, remainder = divmod(len(records), count)
        shapes = [(base + (fragment < remainder), 0) for fragment in range(count)]
    else:
        shapes = [(len(records), 0)]
    cursor = 0
    for fragment, (live, removed) in enumerate(shapes):
        rows = records[cursor:cursor + live]
        cursor += live
        rows += [{**records[0], "id": f"bench-tombstone-{fragment}-{row}"} for row in range(removed)]
        lance.write_dataset(batch(rows), str(location), mode="overwrite" if fragment == 0 else "append",
                            data_storage_version="2.2", enable_v2_manifest_paths=True)
    dataset = lance.dataset(str(location))
    if any(removed for _, removed in shapes):
        dataset.delete("id LIKE 'bench-tombstone-%'")
        dataset = lance.dataset(str(location))
    actual = dataset.count_rows()
    if actual != len(records):
        raise ValueError("Synthetic dataset row count verification failed")
    fragments = dataset.get_fragments()
    index["database"] = {"table": TABLE, "path": str(relative), "rows": actual, "profile": profile,
                          "layout": layout, "logical_records_unchanged_by_layout": True,
                          "intentional_layout_change": profile == "production" and layout == "compact",
                          "people": people, "history_rows_per_person": history_rows,
                          "entity_counts": dict(Counter(row["entity_type"] for row in records)),
                          "schema": str(schema), "data_storage_version": "2.2", "pylance": lance.__version__,
                          "manifest_paths": "v2",
                          "synthetic": True,
                          "fragments": len(fragments),
                          "physical_rows": sum(fragment.metadata.physical_rows for fragment in fragments),
                          "deletion_files": sum(fragment.metadata.deletion_file is not None for fragment in fragments),
                          "reference": {"version": 62, "observed_at": "2026-10-25", "rows": 47,
                                        "fragments": 36, "physical_rows": 58, "deletion_files": 3,
                                        "source": "Hosted Lance manifest only; customer row files were not read",
                                        "entity_mix": "Inferred: two contacts with four histories each, plus thirteen unlinked documents"},
                          "records_sha256": digest(json.dumps(records, sort_keys=True).encode())}
    write_json(output / "synthetic-records.json", records)
    index["browser"] = {
        "route": "/use/personen", "query": {"id": index["app_id"]},
        "board_id": BOARD_ID, "on_load_node_id": "wic5f5042hxcb0v6go1qtpfd",
        "widget_action_node_id": "x12cajnxcj8bygecwmjxe9dj", "action_id": "open-person",
        "person_widget_id": "vls76coy54xdj0y71ujdlu37",
        "history_widget_id": "uxhufk5wzx254sawv2322vro",
        "history_element_id": "ps-history-list", "row_component_id": "tenancy-title",
        "persons": [{"id": f"bench-contact-{person:04d}", "name": f"Benchmark Person {person:04d}",
                     "expected_history_text": f"Benchmark Property {person:04d}-",
                     "expected_rows": history_rows} for person in (1, 2)],
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--metadata-export", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--user-id", default="widget-bench-user")
    parser.add_argument("--profile", choices=("production", "quick"), default="production",
                        help="production matches observed row/fragment counts with synthetic entity relationships")
    parser.add_argument("--layout", choices=("fragmented", "compact"),
                        help="Default: fragmented for production, compact for quick; changes only synthetic storage layout")
    parser.add_argument("--people", type=int, default=12, help="Number of contacts with --profile quick")
    parser.add_argument("--history-rows", type=int, default=4,
                        help="History entries per contact with --profile quick")
    args = parser.parse_args(argv)
    if args.people < 2 or args.history_rows < 1 or args.history_rows > 100:
        parser.error("Use at least two people and between 1 and 100 history rows per person")
    output = args.output.resolve()
    if output == args.metadata_export.resolve() or output in args.metadata_export.resolve().parents:
        parser.error("Output must not replace the source export")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=True)
    index = prepare_metadata(args.metadata_export.resolve(), output, args.user_id)
    layout = args.layout or ("fragmented" if args.profile == "production" else "compact")
    seed_content(output, index, args.people, args.history_rows, args.profile, layout)
    index["fixture_sha256"] = digest(json.dumps({"objects": index["objects"], "database": index["database"]},
                                               sort_keys=True).encode())
    write_json(output / "fixture-index.json", index)
    print(json.dumps({"fixture_index": str(output / "fixture-index.json"),
                      "source_objects": len(index["objects"]), "synthetic_rows": index["database"]["rows"]}))


if __name__ == "__main__":
    main()
