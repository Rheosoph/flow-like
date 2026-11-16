import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


GENERATOR = Path(__file__).resolve().parents[1] / "runtime-config.py"
SPEC = importlib.util.spec_from_file_location("web_runtime_config", GENERATOR)
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


class RuntimeConfigTests(unittest.TestCase):
    def test_requires_api_without_legacy_or_hosted_fallback(self):
        for environment in ({}, {"NEXT_PUBLIC_API_URL": "https://legacy.example"}, {"FLOW_LIKE_WEB_API_URL": ""}):
            with self.subTest(environment=environment), self.assertRaises(ValueError):
                runtime.build_config(environment)

    def test_allowlisted_public_values_only_and_origin_relative_defaults(self):
        config = runtime.build_config({
            "FLOW_LIKE_WEB_API_URL": "https://api.example.test/prefix///",
            "FLOW_LIKE_WEB_REDIRECT_URL": "",
            "DATABASE_URL": "private-database-marker",
            "AWS_SECRET_ACCESS_KEY": "private-aws-marker",
            "CLIENT_SECRET": "private-client-marker",
            "FLOW_LIKE_WEB_UNKNOWN": "private-unknown-marker",
        })
        self.assertEqual(config, {"version": 1, "apiUrl": "https://api.example.test/prefix"})
        self.assertNotIn("private-", runtime.render_script(config))

    def test_explicit_redirects_and_local_or_ipv6_addresses(self):
        self.assertEqual(runtime.public_url("http://[::1]:8080/", "API", api=True), "http://[::1]:8080")
        config = runtime.build_config({
            "FLOW_LIKE_WEB_API_URL": "http://localhost:8080",
            "FLOW_LIKE_WEB_REDIRECT_URL": "https://web.example.test/callback",
            "FLOW_LIKE_WEB_LOGOUT_URL": "https://web.example.test/",
        })
        self.assertEqual(config["redirectUrl"], "https://web.example.test/callback")
        self.assertEqual(config["logoutUrl"], "https://web.example.test/")

    def test_rejects_unsafe_or_ambiguous_urls_without_echoing_values(self):
        invalid = [
            "api.example.test", "//api.example.test", "/api", "javascript:alert(1)",
            "https:///api.example.test", "https://", " https://api.example.test",
            "https://api.example.test/\n", "https://api.example.test/\x00",
            "https://user:do-not-log@example.test", "https://user@example.test",
            "https://api.example.test/?credential=do-not-log", "https://api.example.test/#do-not-log",
            "https://api.example.test/?", "https://api.example.test/#",
            "https://api.example.test\\@other.example.test", "https://api.example.test:invalid",
            "https://api.example.test:65536", "https://api.example.test:0", "https://api.example.test:",
            "https://bad_host.example.test", "https://bad..example.test", "https://-bad.example.test",
            "HTTPS://api.example.test", "https://1.2.3.999", "https://example.123", "https://[fe80::1%eth0]",
        ]
        for value in invalid:
            for name in ("FLOW_LIKE_WEB_API_URL", "FLOW_LIKE_WEB_REDIRECT_URL", "FLOW_LIKE_WEB_LOGOUT_URL"):
                with self.subTest(value=value, name=name), self.assertRaises(ValueError) as raised:
                    runtime.build_config({"FLOW_LIKE_WEB_API_URL": "https://api.example.test", name: value})
                self.assertNotIn(value, str(raised.exception))
                self.assertNotIn("do-not-log", str(raised.exception))

    def test_javascript_escaping_round_trips_without_html_or_line_separators(self):
        config = {"version": 1, "apiUrl": 'https://api.example.test/</script><script>"&\\\u2028\u2029'}
        script = runtime.render_script(config)
        self.assertNotIn("<", script)
        self.assertNotIn(">", script)
        self.assertNotIn("&", script)
        self.assertNotIn("\u2028", script)
        self.assertNotIn("\u2029", script)
        payload = script.removeprefix("window.__FLOW_LIKE_PUBLIC_CONFIG__=Object.freeze(").removesuffix(");\n")
        self.assertEqual(json.loads(payload), config)

    def test_cli_writes_only_requested_public_file_and_failures_do_not_replace_it(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "generated" / "runtime-config.js"
            command = [sys.executable, "-B", str(GENERATOR), "--output", str(output)]
            environment = {**os.environ, "FLOW_LIKE_WEB_API_URL": "https://api.example.test"}
            environment.pop("FLOW_LIKE_WEB_REDIRECT_URL", None)
            environment.pop("FLOW_LIKE_WEB_LOGOUT_URL", None)
            success = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertEqual(success.returncode, 0, success.stderr)
            self.assertEqual(success.stdout, "")
            self.assertEqual(success.stderr, "")
            expected = output.read_text()
            self.assertEqual(output.stat().st_mode & 0o777, 0o644)
            environment["FLOW_LIKE_WEB_API_URL"] = "https://do-not-log@example.test"
            failure = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertNotEqual(failure.returncode, 0)
            self.assertNotIn("do-not-log", failure.stdout + failure.stderr)
            self.assertEqual(output.read_text(), expected)
            self.assertEqual(list(output.parent.iterdir()), [output])

    def test_hosted_proxy_preserves_api_prefix_and_quotes_configuration_syntax(self):
        with tempfile.TemporaryDirectory() as directory:
            resolvers = Path(directory) / "resolv.conf"
            resolvers.write_text("search internal.example\nnameserver 127.0.0.11\nnameserver 2001:db8::1\n")
            config = runtime.build_config({
                "FLOW_LIKE_WEB_API_URL": 'https://api.example.test/prefix/$http_cookie";{}/café',
            })
            proxy = runtime.render_hosted_proxy(config, resolvers)
            self.assertIn("resolver 127.0.0.11 [2001:db8::1] valid=30s;", proxy)
            self.assertIn('proxy_pass "https://api.example.test/prefix/%24http_cookie%22%3B%7B%7D/caf%C3%A9/frontend/document$flowlike_hosted_document";', proxy)
            self.assertNotIn("$http_cookie", proxy)
            self.assertIn('proxy_ssl_name "api.example.test";', proxy)
            self.assertIn("proxy_ssl_verify on;", proxy)

    def test_hosted_proxy_uses_literal_ip_without_dns_and_supports_ipv6(self):
        for api, expected in [
            ("http://127.0.0.1:8080/hub", "http://127.0.0.1:8080/hub/frontend/document"),
            ("http://[::1]:8080", "http://[::1]:8080/frontend/document"),
        ]:
            with self.subTest(api=api):
                config = runtime.build_config({"FLOW_LIKE_WEB_API_URL": api})
                proxy = runtime.render_hosted_proxy(config, Path("/missing/resolv.conf"))
                self.assertIn(f'proxy_pass "{expected}$flowlike_hosted_document";', proxy)
                self.assertNotIn("resolver ", proxy)
                self.assertNotIn("proxy_ssl", proxy)

    def test_hosted_proxy_rejects_missing_dns_before_replacing_public_config(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "runtime-config.js"
            proxy = Path(directory) / "hosted-proxy.conf"
            resolvers = Path(directory) / "resolv.conf"
            resolvers.write_text("nameserver invalid;\nnameserver 127.0.0.11;\n")
            output.write_text("existing config")
            with self.assertRaises(ValueError):
                runtime.write_config(output, {"FLOW_LIKE_WEB_API_URL": "https://api.example.test"}, proxy, resolvers)
            self.assertEqual(output.read_text(), "existing config")
            self.assertFalse(proxy.exists())

    def test_cli_can_write_proxy_include_beside_public_runtime_config(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "runtime-config.js"
            proxy = Path(directory) / "hosted-proxy.conf"
            result = subprocess.run([
                sys.executable, "-B", str(GENERATOR), "--output", str(output),
                "--nginx-output", str(proxy),
            ], env={**os.environ, "FLOW_LIKE_WEB_API_URL": "http://127.0.0.1:8080/hub"}, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('proxy_pass "http://127.0.0.1:8080/hub/frontend/document$flowlike_hosted_document";', proxy.read_text())
            self.assertIn('"apiUrl":"http://127.0.0.1:8080/hub"', output.read_text())
            self.assertEqual(proxy.stat().st_mode & 0o777, 0o644)

    def test_internal_hosted_api_stays_out_of_browser_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "runtime-config.js"
            proxy = Path(directory) / "hosted-proxy.conf"
            runtime.write_config(output, {
                "FLOW_LIKE_WEB_API_URL": "https://api.example.test/public",
                "FLOW_LIKE_WEB_HOSTED_API_URL": "http://127.0.0.1:8080/internal",
            }, proxy, Path("/missing/resolv.conf"))
            self.assertIn('"apiUrl":"https://api.example.test/public"', output.read_text())
            self.assertNotIn("127.0.0.1", output.read_text())
            self.assertIn('proxy_pass "http://127.0.0.1:8080/internal/frontend/document$flowlike_hosted_document";', proxy.read_text())
            with self.assertRaises(ValueError):
                runtime.write_config(output, {
                    "FLOW_LIKE_WEB_API_URL": "https://api.example.test",
                    "FLOW_LIKE_WEB_HOSTED_API_URL": "https://secret@example.test",
                }, proxy)

    def test_nginx_routes_query_links_without_forwarding_other_query_parameters(self):
        compose = GENERATOR.parent / "nginx.conf"
        kubernetes = GENERATOR.parents[2] / "kubernetes/web/nginx.conf"
        source = compose.read_text()
        self.assertEqual(source, kubernetes.read_text())
        self.assertIn('default "?app=$arg_app&route=$arg_route";', source)
        self.assertIn('~^/a/([^?]+) /$1;', source)
        self.assertIn('map $arg_app $flowlike_hosted_shell {\n    "" /a.html;\n    default "";\n}', source)
        for route in ("/a", "/a/"):
            self.assertIn(f"location = {route} {{ try_files $flowlike_hosted_shell @hosted_document; }}", source)
        proxy = source.split("location @hosted_document {", 1)[1].split("\n    }", 1)[0]
        self.assertIn("proxy_pass_request_headers off;", proxy)
        self.assertNotIn("$args", proxy)
        self.assertNotIn("$request_uri", proxy)


if __name__ == "__main__":
    unittest.main()
