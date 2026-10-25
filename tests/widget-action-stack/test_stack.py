import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import import_fixture
import run
import stack


class StackBoundaries(unittest.TestCase):
    def test_auth_prepare_reuses_valid_certificates_and_renews_expired_ones(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            (state / "stack.json").write_text(json.dumps({
                "fixture": "flow-like-widget-action-stack-v1",
                "environment": {"API_URL": "http://127.0.0.1:8440"},
            }))
            auth = stack.HERE / "auth.mjs"
            command = ["node", str(auth), "prepare", str(state)]
            subprocess.run(command, check=True, capture_output=True)
            original = (state / "oidc-cert.pem").read_bytes()
            signing_key = (state / "oidc-signing.pem").read_bytes()
            platform = json.loads((state / "platform.json").read_text())
            self.assertEqual(platform["authentication"]["openid"]["user_info_url"],
                             "http://127.0.0.1:9441/userinfo")
            user_info_check = """
                import http from 'node:http';
                import https from 'node:https';
                import assert from 'node:assert/strict';
                import {readFileSync} from 'node:fs';
                import {sign} from 'node:crypto';
                const modulePath=process.argv[1], directory=process.argv[2];
                let handler;
                const fakeServer={listen(){return this;}};
                http.createServer=(callback)=>{handler=callback;return fakeServer;};
                https.createServer=()=>fakeServer;
                process.argv=['node',modulePath,'serve',directory];
                await import(modulePath);
                const token=readFileSync(directory+'/access-token','utf8').trim();
                function request(bearer){
                    const response={status:200,body:null,setHeader(){},
                        writeHead(status){this.status=status;return this;},
                        end(body){this.body=JSON.parse(body);return this;}};
                    handler({url:'/userinfo',method:'GET',headers:{authorization:bearer}},response);
                    return response;
                }
                const accepted=request('Bearer '+token);
                assert.equal(accepted.status,200);
                assert.equal(accepted.body.sub,'widget-bench-user');
                assert.equal(accepted.body.name,'Benchmark User');
                assert.equal(request(undefined).status,401);
                const [header,payload,signature]=token.split('.');
                const claims=JSON.parse(Buffer.from(payload,'base64url'));
                claims.exp=1;
                const expiredPayload=Buffer.from(JSON.stringify(claims)).toString('base64url');
                const unsigned=header+'.'+expiredPayload;
                const expired=unsigned+'.'+sign('RSA-SHA256',Buffer.from(unsigned),
                    readFileSync(directory+'/oidc-signing.pem')).toString('base64url');
                assert.equal(request('Bearer '+expired).status,401);
                assert.equal(request('Bearer '+unsigned+'.'+signature).status,401);
            """
            subprocess.run(["node", "--input-type=module", "-e", user_info_check, auth.as_uri(), str(state)],
                           check=True, capture_output=True)
            subprocess.run(command, check=True, capture_output=True)
            self.assertEqual((state / "oidc-cert.pem").read_bytes(), original)
            future_clock = """
                const modulePath=process.argv[1], directory=process.argv[2], now=Date.now();
                Date.now=()=>now+31*24*3600*1000;
                process.argv=['node',modulePath,'prepare',directory];
                await import(modulePath);
            """
            subprocess.run(["node", "--input-type=module", "-e", future_clock, auth.as_uri(), str(state)],
                           check=True, capture_output=True)
            self.assertNotEqual((state / "oidc-cert.pem").read_bytes(), original)
            self.assertEqual((state / "oidc-signing.pem").read_bytes(), signing_key)

    def test_dependency_downloads_keep_public_trust_after_local_auth_setup(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            (state / "stack.json").write_text(json.dumps({
                "fixture": "flow-like-widget-action-stack-v1", "environment": {"AWS_ACCESS_KEY_ID": "local"},
            }))
            (state / "oidc-ca.pem").touch()
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(stack.environment(state)["SSL_CERT_FILE"], str(state / "oidc-ca.pem"))
                self.assertNotIn("SSL_CERT_FILE", stack.dependency_environment(state))
            public_ca = state / "organization-public-ca.pem"
            public_ca.touch()
            with patch.dict(os.environ, {"SSL_CERT_FILE": str(public_ca)}, clear=True):
                self.assertEqual(stack.dependency_environment(state)["SSL_CERT_FILE"], str(public_ca))
                self.assertEqual(stack.environment(state)["SSL_CERT_FILE"], str(state / "oidc-ca.pem"))

    def test_rust_stack_setting_is_scoped_to_started_services(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            (state / "platform.json").write_text('{"features":{"telemetry":false}}')
            before = os.environ.get("RUST_MIN_STACK")
            args = SimpleNamespace(state=state, workers=None, executor_slots=None, thread_stack_mb=32, trace=False)
            with patch.object(stack, "environment", return_value={}):
                services = run.Services(args)
            self.assertEqual(services.env["RUST_MIN_STACK"], "33554432")
            self.assertEqual(os.environ.get("RUST_MIN_STACK"), before)

    def test_backend_build_target_is_remembered_without_the_original_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            target = (state / "isolated-cargo-target").resolve()
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}, clear=True):
                binary = run.target_binary("release-fast")
            self.assertEqual(binary, target / "release-fast/examples/widget_action_stack")
            (state / "build-backend.json").write_text(json.dumps({
                "repository": str(stack.REPO), "profile": "release-fast", "binary": str(binary),
            }))
            with patch.dict(os.environ, {}, clear=True):
                self.assertEqual(run.built_binary(state, "release-fast"), binary)
                self.assertEqual(run.built_binary(state, "debug"), stack.REPO / "target/debug/examples/widget_action_stack")

    def test_existing_fixture_cannot_silently_ignore_requested_layout(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            fixture = state / "fixture"
            fixture.mkdir()
            (fixture / "fixture-index.json").write_text(json.dumps({
                "database": {"profile": "production", "layout": "fragmented"},
            }))
            args = SimpleNamespace(state=state, fixture=None, metadata_export=None,
                                   fixture_profile=None, fixture_layout="compact")
            with self.assertRaisesRegex(ValueError, "provide --metadata-export"):
                run.fixture_directory(args)
            args.fixture_layout = None
            self.assertEqual(run.fixture_directory(args), fixture.resolve())
            args.fixture_profile = "quick"
            with self.assertRaisesRegex(ValueError, "provide --metadata-export"):
                run.fixture_directory(args)

    def test_external_fixture_browser_config_remains_the_benchmark_default(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state, fixture = root / "state", root / "external-fixture"
            state.mkdir()
            fixture.mkdir()
            self.assertEqual(run.default_browser_config(state), state / "fixture/browser-config.json")
            (fixture / "fixture-index.json").write_text(json.dumps({"browser": {
                "person_widget_id": "card", "history_widget_id": "row", "history_element_id": "history",
                "route": "/use/personen", "query": {"id": "app"}, "persons": [{
                    "id": "person-1", "name": "Person One", "expected_history_text": "History One", "expected_rows": 4,
                }],
            }}))
            generated = run.browser_config(state, fixture)
            self.assertEqual(run.default_browser_config(state), generated.resolve())
            args = SimpleNamespace(state=state, config=None, output=state / "report")
            with patch.object(stack, "environment", return_value={}), patch.object(stack, "run") as execute:
                run.benchmark(args)
            command = execute.call_args.args[0]
            self.assertEqual(command[command.index("--config") + 1], str(generated.resolve()))

    def test_services_do_not_inherit_deployment_credentials(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            (state / "stack.json").write_text(json.dumps({
                "fixture": "flow-like-widget-action-stack-v1",
                "environment": {"AWS_ACCESS_KEY_ID": "local", "DATABASE_URL": "local-db"},
            }))
            (state / "oidc-ca.pem").touch()
            with patch.dict(os.environ, {
                "AWS_PROFILE": "production", "AWS_SESSION_TOKEN": "production-token",
                "AWS_ACCESS_KEY_ID": "production", "DATABASE_URL": "production-db",
                "FLOW_LIKE_CONFIG_JSON": "production-config", "DSQL_CLUSTER_ENDPOINT": "production",
            }):
                env = stack.environment(state)
            self.assertEqual(env["AWS_ACCESS_KEY_ID"], "local")
            self.assertEqual(env["DATABASE_URL"], "local-db")
            self.assertEqual(env["SSL_CERT_FILE"], str(state / "oidc-ca.pem"))
            for name in ("AWS_PROFILE", "AWS_SESSION_TOKEN", "FLOW_LIKE_CONFIG_JSON", "DSQL_CLUSTER_ENDPOINT"):
                self.assertNotIn(name, env)

    def test_wrong_fixture_marker_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary)
            (state / "stack.json").write_text('{"fixture":"different-project"}')
            with self.assertRaises(ValueError):
                stack.environment(state)

    def test_occupied_ports_are_not_adopted(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            with self.assertRaises(ValueError):
                run.unused_port(f"http://127.0.0.1:{listener.getsockname()[1]}/health")

    def test_stale_process_inventory_cannot_stop_another_process(self):
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                                   start_new_session=True)
        try:
            run.stop_record({"pid": process.pid, "started": "a different process lifetime"})
            self.assertIsNone(process.poll())
        finally:
            process.terminate()
            process.wait(timeout=5)

    def test_exact_event_target_and_floating_version_survive_import(self):
        index = {"app_id": "fixture-app", "widgets": [], "pages": [], "events": [{
            "id": "person-event", "name": "Owner's history", "event_type": "page",
            "board_id": "original-board", "board_version": None, "node_id": "original-node",
            "default_page_id": "original-page", "event_version": [0, 0, 3],
            "config": [123, 125],
        }]}
        sql = import_fixture.fixture_sql(index, "local-user")
        self.assertIn("'original-board',NULL,'original-node','original-page'", sql)
        self.assertIn("'0.0.3'", sql)
        self.assertIn("'Owner''s history'", sql)
        self.assertIn('{"base64":"e30="}', sql)
        self.assertNotIn("production", sql)

    def test_archived_pages_cannot_replace_current_page_rows(self):
        page = {"id": "page", "name": "Current", "board_id": "board"}
        with self.assertRaises(ValueError):
            import_fixture.fixture_sql({
                "app_id": "app", "widgets": [], "events": [], "pages": [page, page],
            }, "user")

    def test_remote_table_cleanup_cannot_escape_fixture_app(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = Path(temporary).resolve()
            table = fixture / "content/apps/app/storage/db/history.lance"
            table.mkdir(parents=True)
            index = {"app_id": "app", "database": {
                "path": "apps/app/storage/db/history.lance", "synthetic": True,
            }}
            self.assertEqual(import_fixture.generated_table(fixture, index)[0], table)
            index["database"]["path"] = "apps/app/storage/db/../../../../other.lance"
            with self.assertRaises(ValueError):
                import_fixture.generated_table(fixture, index)
            index["database"]["path"] = "apps/app/storage/db/history.lance"
            index["database"]["synthetic"] = False
            with self.assertRaises(ValueError):
                import_fixture.generated_table(fixture, index)


if __name__ == "__main__":
    unittest.main()
