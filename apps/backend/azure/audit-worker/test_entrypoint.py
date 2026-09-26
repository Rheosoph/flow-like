import contextlib
import importlib.util
from email.message import Message
import io
from pathlib import Path
import unittest
import urllib.error
import urllib.parse
import urllib.request
import urllib.response

spec = importlib.util.spec_from_file_location("entrypoint", Path(__file__).with_name("entrypoint.py"))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


class LauncherTest(unittest.TestCase):
    env = {"IDENTITY_ENDPOINT": "http://localhost:42356/msi/token", "IDENTITY_HEADER": "private-header", "AZURE_CLIENT_ID": "worker-client"}

    def test_token_protocol(self):
        request = launcher.token_request(self.env)
        self.assertEqual(request.get_header("X-identity-header"), "private-header")
        self.assertEqual(urllib.parse.parse_qs(urllib.parse.urlsplit(request.full_url).query), {
            "api-version": ["2019-08-01"], "resource": [launcher.RESOURCE], "client_id": ["worker-client"],
        })

    def test_redirect_does_not_forward_identity_header(self):
        calls = []
        class RedirectingHTTP(urllib.request.HTTPHandler):
            def http_open(self, request):
                calls.append(request.full_url)
                headers = Message()
                headers["Location"] = "http://external.example/collect"
                response = urllib.response.addinfourl(io.BytesIO(b""), headers, request.full_url, 302)
                response.msg = "Found"
                return response
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), RedirectingHTTP(), launcher.NoRedirect()).open
        with self.assertRaises(RuntimeError):
            launcher.fetch_token(self.env, opener, now=1000, sleeper=lambda _: None)
        self.assertEqual(len(calls), 1)
        self.assertTrue(calls[0].startswith("http://localhost:"))

    def test_url_escape_and_preserve_tls(self):
        url = launcher.database_url("postgresql://worker%40tenant@host.example:5432/db?sslmode=verify-full", "a+/b:c=@&%", "worker@tenant")
        parsed = urllib.parse.urlsplit(url)
        self.assertEqual(urllib.parse.unquote(parsed.password), "a+/b:c=@&%")
        self.assertEqual(urllib.parse.unquote(parsed.username), "worker@tenant")
        self.assertEqual(parsed.hostname, "host.example")
        self.assertEqual(parsed.query, "sslmode=verify-full")

    def test_rejects_wrong_role_password_and_unsafe_tls(self):
        for url in [
            "postgresql://api@host/db?sslmode=verify-full",
            "postgresql://worker:password@host/db?sslmode=verify-full",
            "postgresql://worker@host/db?sslmode=require",
            "postgresql://worker@host/db?sslmode=verify-full&password=hidden",
            "postgresql://worker@host/db?sslmode=verify-full&user=api",
        ]:
            with self.subTest(url=url), self.assertRaises(ValueError):
                launcher.database_url(url, "secret", "worker")

    def test_refuses_remote_identity_endpoint(self):
        with self.assertRaises(ValueError):
            launcher.token_request(dict(self.env, IDENTITY_ENDPOINT="https://example.org/token"))

    def test_valid_token_and_timeout(self):
        def opener(request, timeout):
            self.assertEqual(timeout, 10)
            return io.StringIO('{"access_token":"token", "expires_on":"2000"}')
        self.assertEqual(launcher.fetch_token(self.env, opener, now=1000, sleeper=lambda _: None), ("token", 1000))

    def test_identity_startup_retries_are_bounded(self):
        delays = []
        attempts = []
        def opener(request, timeout):
            attempts.append(request.full_url)
            if len(attempts) < 3:
                raise urllib.error.URLError("connection refused")
            return io.StringIO('{"access_token":"token", "expires_on":"2000"}')
        self.assertEqual(launcher.fetch_token(self.env, opener, now=1000, sleeper=delays.append), ("token", 1000))
        self.assertEqual(delays, [1, 2])
        self.assertEqual(len(attempts), 3)

    def test_errors_never_expose_response_or_token(self):
        def failing_opener(request, timeout):
            raise urllib.error.HTTPError(request.full_url, 500, "secret-token", {}, io.BytesIO(b"secret-body"))
        for opener in [failing_opener, lambda *_args, **_kwargs: io.StringIO('{"access_token":"secret-token", "expires_on":"1050"}')]:
            capture = io.StringIO()
            with contextlib.redirect_stdout(capture), contextlib.redirect_stderr(capture):
                with self.assertRaisesRegex(RuntimeError, "Unable to obtain") as error:
                    launcher.fetch_token(self.env, opener, now=1000, sleeper=lambda _: None)
            self.assertNotIn("secret", str(error.exception))
            self.assertEqual(capture.getvalue(), "")
            self.assertTrue(error.exception.__suppress_context__)


if __name__ == "__main__":
    unittest.main()
