"""Execute selected Python documentation examples against a local mock transport."""

from contextlib import redirect_stdout
from io import StringIO
import json
import os
from pathlib import Path
import sys

import httpx

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "libs/platform/python"))
from flow_like import FlowLikeClient  # noqa: E402

DOC = (ROOT / "apps/docs/src/content/docs/dev/sdks/python.mdx").read_text()


def snippet(title):
    fence = f'```python title="{title}"\n'
    assert fence in DOC, f"Missing documented example: {title}"
    return DOC.split(fence, 1)[1].split("\n```", 1)[0]


def run(title, client):
    output = StringIO()
    with redirect_stdout(output):
        exec(compile(snippet(title), title, "exec"), {"client": client})
    return output.getvalue()


status = 404
legacy_model = False
calls = []


def handler(request):
    assert str(request.url).startswith("https://api.flow-like.com/api/v1/")
    assert request.headers["Authorization"] == "pat_mock.example"
    path = request.url.path
    body = json.loads(request.content) if request.content else {}
    calls.append((path, body))
    if path == "/api/v1/apps/nonexistent-id":
        return httpx.Response(status, json={"message": "Mock error"})
    if path == "/api/v1/bit":
        return httpx.Response(200, json=[
            {
                "id": surface,
                "type": "Llm",
                "meta": {"en": {"name": surface}},
                "parameters": {"provider": {
                    "provider_name": "Hosted OpenAI",
                    **({} if legacy_model and surface == "ChatCompletions"
                       else {"api_surface": surface}),
                }},
            }
            for surface in ["Responses", "ChatCompletions"]
        ])
    if path == "/api/v1/chat/completions":
        assert body["model"] == "ChatCompletions"
        assert isinstance(body["messages"], list)
        assert body["max_tokens"] == 200
        return httpx.Response(200, json={
            "choices": [{"message": {"role": "assistant", "content": "Mock answer"}}],
        })
    if path == "/api/v1/responses":
        assert body["model"] == "Responses"
        assert isinstance(body["input"], str)
        assert body["max_output_tokens"] == 200
        return httpx.Response(200, json={"output": []})
    raise AssertionError(f"Unexpected request in documentation check: {path}")


saved_env = {name: os.environ.pop(name, None) for name in [
    "FLOW_LIKE_BASE_URL", "FLOW_LIKE_PAT", "FLOW_LIKE_API_KEY"
]}
try:
    with FlowLikeClient(base_url="https://api.flow-like.com", pat="pat_mock.example") as client:
        client._client = httpx.Client(
            base_url=client._api_base,
            headers=client._auth_headers,
            transport=httpx.MockTransport(handler),
        )
        for code, expected in [(404, "App not found"), (429, "Request rate limited"), (500, "HTTP 500")]:
            status = code
            assert expected in run("api-errors.py", client)
        for legacy in [False, True]:
            legacy_model = legacy
            assert "Mock answer" in run("chat-completions.py", client)
            assert "[]" in run("responses.py", client)
    print("Python SDK docs: 404/429/500 handling and both model APIs passed with mocked HTTP.")
finally:
    for name, value in saved_env.items():
        if value is not None:
            os.environ[name] = value
