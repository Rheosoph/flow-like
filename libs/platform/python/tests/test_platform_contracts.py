"""Wire contracts from packages/api/src/routes and the device hub routers."""
import inspect
import io
import json
import unittest

import httpx

from flow_like import FlowLikeClient
from flow_like._errors import APIError, ConfigurationError, FlowLikeError, NotFoundError
from flow_like._http import segment


class EventStream(httpx.SyncByteStream, httpx.AsyncByteStream):
    def __iter__(self):
        yield b': keepalive\r\nid: 7\r\nevent: chunk\r\nda'
        yield b'ta: first\r\ndata:  second\r\n\r\ndata: last'

    async def __aiter__(self):
        for chunk in self:
            yield chunk


class PlatformContracts(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.requests = []
        self.reply = lambda request: httpx.Response(200, json={"ok": True})
        self.client = FlowLikeClient(base_url="https://hub.example/gateway", api_key="flk_test")
        transport = httpx.MockTransport(self.handle)
        self.client._client = httpx.Client(base_url=self.client._api_base, transport=transport,
                                          headers=self.client._auth_headers)
        self.client._async_client = httpx.AsyncClient(base_url=self.client._api_base, transport=transport,
                                                      headers=self.client._auth_headers)

    async def asyncTearDown(self):
        await self.client.aclose()

    def handle(self, request):
        self.requests.append(request)
        return self.reply(request)

    def assert_wire(self, method, path, body=None, params=None):
        request = self.requests[-1]
        self.assertEqual(request.method, method)
        self.assertEqual(request.url.raw_path.split(b"?")[0].decode(), "/gateway/api/v1" + path)
        self.assertEqual(dict(request.url.params), params or {})
        self.assertEqual(json.loads(request.content) if request.content else None, body)
        self.assertEqual(request.headers.get("x-api-key"), "flk_test")
        self.assertEqual(request.headers.get("x-flow-like-board-format"), "2")

    async def call_both(self, name, *args, **kwargs):
        for prefix in ("", "a"):
            result = getattr(self.client, prefix + name)(*args, **kwargs)
            if inspect.isawaitable(result):
                result = await result
            yield result

    async def test_event_invocation_envelope_and_async_route(self):
        self.reply = lambda _: httpx.Response(200, json={"run_id": "run", "poll_token": "poll", "status": "pending"})
        async for result in self.call_both("trigger_event_async", "app/id", "event?x", {"input": 3}, version="1_2_3", variant="blue", profile_id="profile"):
            self.assertEqual(result.run_id, "run")
            self.assert_wire("POST", "/apps/app%2Fid/events/event%3Fx/invoke/async",
                             {"payload": {"input": 3}, "version": "1_2_3", "profile_id": "profile"}, {"__variant": "blue"})

    async def test_event_sse_handles_chunks_crlf_multiline_and_eof(self):
        self.reply = lambda _: httpx.Response(200, headers={"Content-Type": "text/event-stream"}, stream=EventStream())
        events = list(self.client.trigger_event("app", "event", {"text": "hello"}, variant="green"))
        async_events = [event async for event in await self.client.atrigger_event("app", "event", {"text": "hello"}, variant="green")]
        for received in (events, async_events):
            self.assertEqual([event.data for event in received], ["first\n second", "last"])
            self.assertEqual([event.id for event in received], ["7", "7"])
            self.assertEqual(received[0].event, "chunk")
        self.assert_wire("POST", "/apps/app/events/event/invoke", {"payload": {"text": "hello"}}, {"__variant": "green"})

    async def test_poll_uses_only_bearer_default_cursor_and_server_status(self):
        self.reply = lambda _: httpx.Response(200, json={"run_id": "r", "status": "Completed", "progress": 100,
            "events": [{"sequence": 0, "event_type": "output", "payload": {"value": 1}}]})
        async for result in self.call_both("poll_execution", "poll-secret", headers={"authorization": "old", "X-API-Key": "old-key"}):
            request = self.requests[-1]
            self.assertEqual(request.headers["authorization"], "Bearer poll-secret")
            self.assertNotIn("x-api-key", request.headers)
            self.assertEqual(dict(request.url.params), {"after_sequence": "-1", "timeout": "30"})
            self.assertNotIn("poll-secret", str(request.url))
            self.assertTrue(result.done)
            self.assertEqual(result.status, "Completed")
            self.assertEqual(result.next_sequence, 0)
        self.reply = lambda _: httpx.Response(200, json={"status": "Running", "events": []})
        result = self.client.poll_execution("poll-secret", after_sequence=12)
        self.assertFalse(result.done)
        self.assertEqual(result.next_sequence, 12)
        for status in ("Failed", "Cancelled", "Timeout"):
            self.reply = lambda _, status=status: httpx.Response(200, json={"status": status, "events": []})
            self.assertTrue(self.client.poll_execution("poll-secret").done)

    async def test_create_app_uses_upsert_and_complete_metadata(self):
        self.reply = lambda _: httpx.Response(200, json={"id": "app", "bits": []})
        async for app in self.call_both("create_app", "My app", "Description", bits=["bit"]):
            self.assertEqual(app.id, "app")
            request = self.requests[-1]
            self.assertEqual(request.method, "PUT")
            self.assertEqual(request.url.path, "/gateway/api/v1/apps/new")
            body = json.loads(request.content)
            self.assertEqual(body["bits"], ["bit"])
            self.assertEqual(body["meta"]["name"], "My app")
            self.assertEqual(body["meta"]["description"], "Description")
            self.assertEqual(body["meta"]["tags"], [])
            self.assertEqual(body["meta"]["preview_media"], [])
            self.assertGreater(body["meta"]["created_at"]["secs_since_epoch"], 0)

    async def test_database_corrected_routes_bodies_queries_and_scalar_count(self):
        self.reply = lambda _: httpx.Response(200, json=None)
        async for result in self.call_both("add_to_table", "app", "table", [{"id": 1}], params={"scope": "user", "branch": "draft"}):
            self.assertIsNone(result)
            self.assert_wire("PUT", "/apps/app/db/table", {"items": [{"id": 1}]}, {"scope": "user", "branch": "draft"})
        async for _ in self.call_both("delete_from_table", "app", "table", "id = 1", params={"branch": "draft"}):
            self.assert_wire("DELETE", "/apps/app/db/table", {"query": "id = 1"}, {"branch": "draft"})
        self.reply = lambda _: httpx.Response(200, json=123)
        async for result in self.call_both("count_items", "app", "table", params={"version": 7}):
            self.assertEqual(result.count, 123)
            self.assert_wire("GET", "/apps/app/db/table/count", params={"version": "7"})
        self.reply = lambda _: httpx.Response(200, json=[{"id": 2}])
        async for result in self.call_both("query_table", "app", "table", {"sql": "SELECT * FROM table WHERE id=$id", "sql_params": {"id": 2}}, params={"limit": 10, "offset": 5, "scope": "project", "tag": "release"}):
            self.assertEqual(result.rows, [{"id": 2}])
            self.assertEqual(dict(self.requests[-1].url.params), {"limit": "10", "offset": "5", "scope": "project", "tag": "release"})
        self.reply = lambda _: httpx.Response(200, json=["table"])
        async for result in self.call_both("list_tables", "app"):
            self.assertEqual(result, ["table"])
            self.assert_wire("GET", "/apps/app/db")
        schema = {"fields": [{"name": "id", "data_type": "Int64", "nullable": False}], "metadata": {"key": "id"}}
        self.reply = lambda _: httpx.Response(200, json=schema)
        async for result in self.call_both("get_table_schema", "app", "table"):
            self.assertEqual(result.columns, schema["fields"])
            self.assertEqual(result.raw, schema)

    async def test_database_cloud_credential_options(self):
        cases = [
            ({"Gcp": {"content_bucket": "bucket", "access_token": "gcp-token"}},
             "gs://bucket/users/person/apps/app/db", {"google_storage_token": "gcp-token"}),
            ({"Azure": {"content_container": "container", "account_name": "account", "content_sas_token": "project-sas", "user_content_sas_token": "user-sas"}},
             "az://container/users/person/apps/app/db", {"azure_storage_account_name": "account", "azure_storage_sas_token": "user-sas"}),
            ({"Mixed": {"content": {"Aws": {"content_bucket": "bucket", "access_key_id": "access", "secret_access_key": "secret", "session_token": "session", "region": "eu-west-1"}}}},
             "s3://bucket/users/person/apps/app/db", {"aws_access_key_id": "access", "aws_secret_access_key": "secret", "aws_session_token": "session", "aws_region": "eu-west-1"}),
        ]
        for credentials, uri, options in cases:
            self.reply = lambda _, credentials=credentials: httpx.Response(200, json={"shared_credentials": credentials, "db_path": "users/person/apps/app/db"})
            async for result in self.call_both("get_db_credentials", "app"):
                self.assertEqual(result.uri, uri)
                self.assertEqual(result.storage_options, options)
        self.reply = lambda _: httpx.Response(200, json={"db_path": "apps/app/storage/db"})
        async for result in self.call_both("get_db_credentials_raw", "app", scope="project"):
            self.assertEqual(result.db_path, "apps/app/storage/db")
            self.assert_wire("POST", "/apps/app/db/presign/project", {"table_name": "_default", "access_mode": "read"})

    async def test_public_http_methods_have_matching_async_signatures(self):
        excluded = {"as_langchain_chat", "as_langchain_embeddings"}
        checked = 0
        for name, method in inspect.getmembers(FlowLikeClient, inspect.isfunction):
            if name.startswith("_") or name in excluded or inspect.iscoroutinefunction(method):
                continue
            counterpart = getattr(FlowLikeClient, "a" + name, None)
            self.assertIsNotNone(counterpart, name)
            self.assertTrue(inspect.iscoroutinefunction(counterpart), name)
            self.assertEqual(inspect.signature(method).parameters,
                             inspect.signature(counterpart).parameters, name)
            checked += 1
        self.assertGreater(checked, 175)

    async def test_file_listing_delete_and_scoped_credentials(self):
        self.reply = lambda _: httpx.Response(200, json=[{"location": "folder/file", "size": 4, "is_dir": False}])
        async for result in self.call_both("list_files", "app", prefix="folder", user=True, refresh=True):
            self.assertEqual(result[0].key, "folder/file")
            self.assert_wire("POST", "/apps/app/data/user/list", {"prefix": "folder"}, {"refresh": "true"})
        self.reply = lambda _: httpx.Response(204)
        async for _ in self.call_both("delete_file", "app", "folder/file", user=True):
            self.assert_wire("DELETE", "/apps/app/data/user", {"prefixes": ["folder/file"]})
        credentials = {"shared_credentials": {"Aws": {"content_bucket": "bucket"}}, "path": "apps/app/upload", "access_mode": "read", "expiration": "later"}
        self.reply = lambda _: httpx.Response(200, json=credentials)
        async for result in self.call_both("presign_data", "app", prefix="folder"):
            self.assertEqual(result.shared_credentials, credentials["shared_credentials"])
            self.assertEqual(result.path, "apps/app/upload")
            self.assertEqual(result.expiration, "later")
            self.assert_wire("POST", "/apps/app/data/presign", {"prefix": "folder", "access_mode": "read"})

    async def test_signed_put_and_post_uploads_do_not_forward_credentials(self):
        signed = "https://storage.example/object?sig=a%2Fb&expires=42"
        for method in ("PUT", "POST"):
            grant = {"prefix": "folder/file", "url": signed, "method": method}
            if method == "POST":
                grant["fields"] = {"key": "signed/path", "policy": "signed-policy"}
            def reply(request):
                if request.url.host == "hub.example":
                    self.assertEqual(request.method, "PUT")
                    self.assertEqual(json.loads(request.content), {"prefixes": ["folder/file"], "sizes": [5]})
                    return httpx.Response(200, json=[grant])
                self.assertEqual(str(request.url), signed)
                self.assertEqual(request.method, method)
                self.assertNotIn("authorization", request.headers)
                self.assertNotIn("x-api-key", request.headers)
                self.assertNotIn("x-flow-like-board-format", request.headers)
                if method == "PUT":
                    self.assertEqual(request.content, b"hello")
                    self.assertEqual(request.headers["content-length"], "5")
                else:
                    self.assertIn(b'signed-policy', request.content)
                    self.assertIn(b'hello', request.content)
                    self.assertIn("multipart/form-data", request.headers["content-type"])
                return httpx.Response(204)
            self.reply = reply
            self.assertEqual(self.client.upload_file("app", io.BytesIO(b"hello"), key="folder/file"), grant)
            self.assertEqual(await self.client.aupload_file("app", io.BytesIO(b"hello"), key="folder/file"), grant)

    async def test_multipart_upload_starts_at_the_selected_offset(self):
        def reply(request):
            if request.url.host == "hub.example":
                self.assertEqual(json.loads(request.content)["sizes"], [5])
                return httpx.Response(200, json=[{"prefix": "file", "url": "https://storage.example/upload", "method": "POST", "fields": {"policy": "bounded"}}])
            self.assertIn(b"hello", request.content)
            self.assertNotIn(b"skip:", request.content)
            self.assertEqual(int(request.headers["content-length"]), len(request.content))
            return httpx.Response(204)
        self.reply = reply
        for asynchronous in (False, True):
            file = io.BytesIO(b"skip:hello")
            file.seek(5)
            if asynchronous:
                await self.client.aupload_file("app", file, key="file")
            else:
                self.client.upload_file("app", file, key="file")

    async def test_signed_download_and_partial_grant_failure(self):
        signed = "https://storage.example/file?signature=a%2Fb"
        def reply(request):
            if request.url.host == "hub.example":
                self.assertEqual(request.url.path, "/gateway/api/v1/apps/app/data/user/download")
                self.assertEqual(json.loads(request.content), {"prefixes": ["folder/file"]})
                return httpx.Response(200, json=[{"prefix": "folder/file", "url": signed}])
            self.assertEqual(str(request.url), signed)
            self.assertEqual(request.method, "GET")
            self.assertNotIn("authorization", request.headers)
            self.assertNotIn("x-api-key", request.headers)
            return httpx.Response(200, content=b"file bytes")
        self.reply = reply
        async for content in self.call_both("download_file", "app", "folder/file", user=True):
            self.assertEqual(content, b"file bytes")
        self.reply = lambda _: httpx.Response(200, json=[{"prefix": "folder/file", "error": "cannot sign"}])
        with self.assertRaisesRegex(FlowLikeError, "cannot sign"):
            self.client.download_file("app", "folder/file")
        with self.assertRaisesRegex(FlowLikeError, "cannot sign"):
            await self.client.adownload_file("app", "folder/file")

    async def test_execution_helpers_preserve_request_and_response(self):
        self.reply = lambda _: httpx.Response(200, json=[])
        async for _ in self.call_both("list_runs", "app", "board", limit=20, offset=4):
            self.assert_wire("GET", "/apps/app/board/board/runs", params={"limit": "20", "offset": "4"})
        async for _ in self.call_both("get_run_logs", "app", "board", "run", query={"levels": [2, 3, 4]}, limit=50):
            self.assert_wire("POST", "/apps/app/board/board/logs/query", {"run_id": "run", "query": {"levels": [2, 3, 4]}, "offset": 0, "limit": 50})
        self.reply = lambda _: httpx.Response(200, json=None)
        async for result in self.call_both("get_run_summary", "app", "board", "run"):
            self.assertIsNone(result)
            self.assert_wire("GET", "/apps/app/board/board/logs/summary", params={"run_id": "run"})
        self.reply = lambda _: httpx.Response(200, json={"run_id": "run", "status": "cancellation_requested", "cancelled": False})
        async for result in self.call_both("cancel_run", "run"):
            self.assertFalse(result["cancelled"])
            self.assert_wire("DELETE", "/execution/run/run")

    async def test_empty_results_and_error_responses(self):
        self.reply = lambda _: httpx.Response(204)
        async for result in self.call_both("revoke_device", "device"):
            self.assertIsNone(result)
        self.reply = lambda _: httpx.Response(404, json={"error": "not found"})
        with self.assertRaises(NotFoundError):
            self.client.get_event("app", "missing")
        with self.assertRaises(NotFoundError):
            await self.client.aget_event("app", "missing")
        self.reply = lambda _: httpx.Response(403, text="denied")
        with self.assertRaisesRegex(APIError, "denied"):
            list(self.client.trigger_event("app", "event"))
        with self.assertRaisesRegex(APIError, "denied"):
            async for _ in await self.client.atrigger_event("app", "event"):
                pass

    async def test_sse_rejects_non_stream_success(self):
        self.reply = lambda _: httpx.Response(200, json={"run_id": "run", "status": "pending"})
        with self.assertRaisesRegex(APIError, "text/event-stream"):
            list(self.client.trigger_event("app", "event"))
        with self.assertRaisesRegex(APIError, "text/event-stream"):
            async for _ in await self.client.atrigger_event("app", "event"):
                pass

    async def test_current_board_format_preserves_geometry(self):
        board = {"id": "board", "variables": {"shape": {
            "data_type": "Geometry", "default_value": {"type": "Point", "coordinates": [13.4, 52.5]}
        }}}
        self.reply = lambda _: httpx.Response(200, json=board)
        async for result in self.call_both("get_board", "app", "board"):
            self.assertEqual(result.raw, board)
            self.assert_wire("GET", "/apps/app/board/board")
        self.reply = lambda _: httpx.Response(200, json={"id": "board", "board": board})
        async for result in self.call_both("upsert_board", "app", "board", template=board):
            self.assertEqual(result.raw["board"], board)
            self.assert_wire("PUT", "/apps/app/board/board", {"template": board})

    async def test_sink_path_keeps_hierarchy_and_accepts_empty_success(self):
        self.reply = lambda _: httpx.Response(204)
        async for result in self.call_both("trigger_http_sink", "app", "hooks/special name?", body={"id": 1}):
            self.assertIsNone(result)
            self.assert_wire("POST", "/sink/trigger/http/app/hooks/special%20name%3F", {"id": 1})

    async def test_prefix_identifier_and_base_url_validation(self):
        for value in ("", ".", ".."):
            with self.assertRaises(ValueError):
                segment(value)
        for base in ("file:///tmp/a", "https://user:pass@hub.example", "https://hub.example?token=secret", "https://hub.example/#part"):
            with self.assertRaises(ConfigurationError):
                FlowLikeClient(base_url=base, api_key="flk_test")
        with self.assertRaises(ValueError):
            self.client._request("GET", "https://elsewhere.example/private")
        with self.assertRaises(ValueError):
            await self.client._arequest("GET", "//elsewhere.example/private")
        client = FlowLikeClient(base_url="https://hub.example/prefix/api/v1/", pat="pat_test")
        self.assertEqual(client._api_base, "https://hub.example/prefix/api/v1")
        client.close()


# Independent examples of the documented backend route families. All mutation bodies
# are forwarded intact, including camelCase fields where the backend uses serde aliases.
ROUTES = [
    ("update_app", ["app", {"app": {"id": "app"}}], "PUT", "/apps/app", {"app": {"id": "app"}}),
    ("get_app_meta", ["app"], "GET", "/apps/app/meta", None),
    ("set_app_visibility", ["app", {"visibility": "PRIVATE"}], "PATCH", "/apps/app/visibility", {"visibility": "PRIVATE"}),
    ("fork_app", ["app", {"name": "copy"}], "POST", "/apps/app/fork", {"name": "copy"}),
    ("publish_app", ["app", {"target_visibility": "PUBLIC"}], "POST", "/apps/app/publication/request", {"target_visibility": "PUBLIC"}),
    ("upsert_event", ["app", "evt", {"event": {"id": "evt"}, "register_source": False}], "PUT", "/apps/app/events/evt", {"event": {"id": "evt"}, "register_source": False}),
    ("validate_event", ["app", "evt"], "POST", "/apps/app/events/evt/validate", None),
    ("setup_event", ["app", "evt", {}], "POST", "/apps/app/events/evt/setup", {}),
    ("restore_event", ["app", "evt", {"version": [1, 2, 3], "dry_run": True}], "POST", "/apps/app/events/evt/restore", {"version": [1, 2, 3], "dry_run": True}),
    ("promote_event_canary", ["app", "evt", {"dry_run": True}], "POST", "/apps/app/events/evt/canary/promote", {"dry_run": True}),
    ("list_schedules", [], "GET", "/user/schedules", None),
    ("version_board", ["app", "board"], "PATCH", "/apps/app/board/board", None),
    ("publish_board_if_changed", ["app", "board"], "POST", "/apps/app/board/board/version/current", None),
    ("sync_board", ["app", "board", {"parts": {}}], "POST", "/apps/app/board/board/sync", {"parts": {}}),
    ("get_flowscript", ["app", "board"], "GET", "/apps/app/board/board/flowscript", None),
    ("apply_flowscript", ["app", "board", {"flowscript": "event Start {}"}], "POST", "/apps/app/board/board/flowscript/apply", {"flowscript": "event Start {}"}),
    ("undo_board", ["app", "board", {"commands": []}], "PATCH", "/apps/app/board/board/undo", {"commands": []}),
    ("list_app_nodes", ["app"], "GET", "/apps/app/nodes", None),
    ("create_table", ["app", "table", {"fields": [{"name": "id", "type": "int64"}]}], "POST", "/apps/app/db/table", {"fields": [{"name": "id", "type": "int64"}]}),
    ("drop_table", ["app", "table"], "DELETE", "/apps/app/db/table/table", None),
    ("add_table_column", ["app", "table", {"name": "name", "type": "string"}], "POST", "/apps/app/db/table/columns", {"name": "name", "type": "string"}),
    ("alter_table_column", ["app", "table", {"column": "name", "rename": "label"}], "PUT", "/apps/app/db/table/columns", {"column": "name", "rename": "label"}),
    ("drop_table_columns", ["app", "table", {"columns": ["label"]}], "DELETE", "/apps/app/db/table/columns", {"columns": ["label"]}),
    ("set_table_primary_key", ["app", "table", {"column": "id"}], "PUT", "/apps/app/db/table/primary-key", {"column": "id"}),
    ("build_table_index", ["app", "table", {"column": "id", "index_type": "BTree", "optimize": False}], "POST", "/apps/app/db/table/index", {"column": "id", "index_type": "BTree", "optimize": False}),
    ("drop_table_index", ["app", "table", "id_idx"], "DELETE", "/apps/app/db/table/index/id_idx", None),
    ("table_reference_action", ["app", "table", {"action": "create_tag", "name": "release"}], "POST", "/apps/app/db/table/references", {"action": "create_tag", "name": "release"}),
    ("optimize_table", ["app", "table", {}], "POST", "/apps/app/db/table/optimize", {}),
    ("create_saved_query", ["app", {"name": "q", "sql": "SELECT 1"}], "POST", "/apps/app/db/queries", {"name": "q", "sql": "SELECT 1"}),
    ("execute_query", ["app", {"sql": "SELECT 1", "surface": "project"}], "POST", "/apps/app/db/queries/execute", {"sql": "SELECT 1", "surface": "project"}),
    ("upsert_page", ["app", "page", {"page": {"id": "page"}}], "PUT", "/apps/app/pages/page", {"page": {"id": "page"}}),
    ("create_route", ["app", {"path": "/", "eventId": "evt", "isDefault": True}], "POST", "/apps/app/routes", {"path": "/", "eventId": "evt", "isDefault": True}),
    ("create_widget_version", ["app", "widget", {"version_type": "patch"}], "POST", "/apps/app/widgets/widget/versions", {"version_type": "patch"}),
    ("add_connection", ["app", {"source_app_id": "other", "role_id": "role"}], "POST", "/apps/app/connections", {"source_app_id": "other", "role_id": "role"}),
    ("accept_connection", ["app", "connection", {"role_id": "role"}], "POST", "/apps/app/connections/queue/connection", {"role_id": "role"}),
    ("add_package", ["app", {"packageId": "pkg", "autoUpdate": True}], "POST", "/apps/app/packages", {"packageId": "pkg", "autoUpdate": True}),
    ("assign_role", ["app", "role", "user/id"], "POST", "/apps/app/roles/role/assign/user%2Fid", None),
    ("accept_join_request", ["app", "request"], "POST", "/apps/app/team/queue/request", None),
    ("create_api_key", ["app", {"name": "key", "role_id": "role"}], "PUT", "/apps/app/api", {"name": "key", "role_id": "role"}),
    ("rename_device", ["dev", {"display_name": "Desk"}], "PATCH", "/devices/dev", {"display_name": "Desk"}),
    ("cancel_device_enrollment", ["enroll"], "DELETE", "/devices/enrollments/enroll", None),
    ("get_device_policy", ["dev"], "GET", "/devices/dev/management/policy", None),
    ("create_device_resource_grant", ["dev", {"project_id": "app"}], "POST", "/devices/dev/resource-grants", {"project_id": "app"}),
    ("approve_resource_grant_billing", ["dev", "grant", {"limit": 10}], "POST", "/devices/dev/resource-grants/grant/billing", {"limit": 10}),
    ("register_device_instance", ["dev", {"id": "service"}], "POST", "/devices/dev/instances", {"id": "service"}),
    ("release_device_schedule", ["app", "evt", {"device_id": "dev", "placement_id": "service"}], "PUT", "/apps/app/device-schedules/evt", {"device_id": "dev", "placement_id": "service"}),
    ("give_back_device_schedule", ["app", "evt"], "DELETE", "/apps/app/device-schedules/evt", None),
    ("delete_fleet_reader", ["dev", "key", {"declaration": "signed"}], "DELETE", "/devices/dev/fleet/readers/key", {"declaration": "signed"}),
    ("unmute_device_certificate_notices", ["dev"], "DELETE", "/devices/dev/certificate-notices/mute", None),
]


def contract_case(name, args, method, path, body):
    async def run(self):
        self.reply = lambda _: httpx.Response(204)
        async for result in self.call_both(name, *args):
            self.assertIsNone(result)
            self.assert_wire(method, path, body)
    return run


for case in ROUTES:
    setattr(PlatformContracts, "test_route_" + case[0], contract_case(*case))


if __name__ == "__main__":
    unittest.main()
