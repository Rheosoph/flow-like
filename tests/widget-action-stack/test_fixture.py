import json
import unittest

from fixture import APP_ID, Sanitizer, catalog_coverage, synthetic_records
from export import CURRENT_SOURCE, relative_key


class FixtureTests(unittest.TestCase):
    def test_sanitizer_removes_credentials_and_external_destinations(self):
        value = {
            "secret_key": "private-value", "label": "Person <person@company.test>",
            "url": "https://example.test/path?token=private-value",
            "aws": "AKIAABCDEFGHIJKLMNOP",
            "jwt": "eyJhbGciOiJIUzI1NiJ9.cGF5bG9hZA.c2lnbmF0dXJl",
        }
        clean = Sanitizer("fixture-user").json(value)
        encoded = json.dumps(clean)
        for private in ("private-value", "company.test", "example.test", "AKIA", "eyJ"):
            self.assertNotIn(private, encoded)
        self.assertEqual(clean["secret_key"], "")

    def test_sanitizer_preserves_schema_identity_and_graph_references(self):
        value = {"$schema": "https://json-schema.org/draft/2020-12/schema",
                 "node_id": "x12cajnxcj8bygecwmjxe9dj", "path": "$item.name",
                 "filter": "app_scope_id = $scope AND id = $id"}
        self.assertEqual(Sanitizer("fixture-user").json(value), value)

    def test_synthetic_history_references_existing_records_in_same_scope(self):
        records = synthetic_records(APP_ID, 2, 4)
        self.assertEqual(len(records), 34)
        by_id = {row["id"]: json.loads(row["payload_json"]) for row in records}
        self.assertEqual(len(by_id), len(records))
        members = [row for row in by_id.values() if row["entity_type"] == "tenancy_member"]
        for member in members:
            for key, kind in (("contact_id", "contact"), ("tenancy_id", "tenancy"),
                              ("property_id", "property"), ("unit_id", "unit")):
                self.assertEqual(by_id[member[key]]["entity_type"], kind)
                self.assertEqual(by_id[member[key]]["app_scope_id"], APP_ID)
        for person in (1, 2):
            history = [row for row in members if row["contact_id"] == f"bench-contact-{person:04d}"]
            self.assertEqual(len(history), 4)
            self.assertEqual(sum(row["lifecycle"] == "active" for row in history), 1)

    def test_records_match_query_envelope(self):
        for row in synthetic_records(APP_ID, 2, 1):
            payload = json.loads(row["payload_json"])
            for key in ("id", "app_scope_id", "entity_type", "schema_version", "revision", "deleted"):
                self.assertEqual(row[key], payload[key])
            self.assertEqual(row["schema_version"], 2)
            self.assertFalse(row["deleted"])

    def test_person_and_history_fields_match_actual_graph_reads(self):
        records = synthetic_records(APP_ID, 2, 4)
        for row in records:
            payload = json.loads(row["payload_json"])
            kind = row["entity_type"]
            if kind == "contact":
                # psPersonCard and psShowPerson read display_name directly into string pins.
                self.assertEqual(payload["display_name"], payload["name"])
                for field in ("display_name", "email", "phone", "company", "notes"):
                    self.assertIsInstance(payload[field], str)
                self.assertEqual(payload["roles"], ["tenant"])
            elif kind == "property":
                self.assertIsInstance(payload["name"], str)
                self.assertTrue(payload["name"].startswith("Benchmark Property "))
            elif kind in ("unit", "tenancy"):
                self.assertIsInstance(payload["label"], str)
                self.assertIsInstance(payload["property_id"], str)
                if kind == "tenancy":
                    for field in ("unit_id", "start_date", "end_date", "lifecycle", "cold_rent_eur",
                                  "warm_rent_eur", "ancillary_prepayment_eur", "deposit_eur", "payment_day",
                                  "rent_check_mode", "rent_check_status", "rent_confirmed_month"):
                        self.assertIsInstance(payload[field], str)

    def test_export_rejects_paths_outside_app_or_output(self):
        prefix = f"apps/{APP_ID}/"
        for key in ("apps/other/manifest.app", prefix + "../escape.board",
                    prefix + "/absolute.board", prefix + "dir\\escape.board"):
            with self.assertRaises(ValueError):
                relative_key(key, prefix)
        self.assertEqual(relative_key(prefix + "events/event.event", prefix), "events/event.event")

    def test_export_selects_only_current_sources(self):
        for key in ("manifest.app", "board.board", "widget.widget", "events/event.event", "_board/page.page"):
            self.assertRegex(key, CURRENT_SOURCE)
        for key in ("compiled/board.flcb", "_board/versions/page.page", "storage/db/table.lance",
                    "events/event/1.event", "versions/board.board"):
            self.assertIsNone(CURRENT_SOURCE.fullmatch(key))

    def test_production_record_mix_has_expected_live_count(self):
        rows = synthetic_records(APP_ID, 2, 4, extra_documents=13)
        self.assertEqual(len(rows), 47)
        self.assertEqual(sum(row["entity_type"] == "tenancy_member" for row in rows), 8)

    def test_catalog_coverage_does_not_accept_missing_or_unlinked_packages(self):
        result = catalog_coverage({"known": 4, "media": 1, "unknown": 1},
                                  {"known": {"std-ui"}, "media": {"media-image"}})
        self.assertEqual(result["covered_node_type_count"], 1)
        self.assertEqual(result["outside_benchmark_catalog"], ["media", "unknown"])


if __name__ == "__main__":
    unittest.main()
