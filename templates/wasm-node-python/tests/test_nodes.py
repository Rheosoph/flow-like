"""Tests for the six main template nodes (Rust-equivalent)."""

import json

from conftest import make_context
from node import (
    CharCount,
    FileReader,
    FileWriter,
    Greeting,
    GreetingConfig,
    RepeatText,
    WeatherAgent,
    _get_weather_tool,
    _wmo_condition,
)
from sdk import (
    Bit,
    FlowPath,
    MockHostBridge,
    get_all_definitions,
    run_node,
)


# ── Registration ────────────────────────────────────────────────────────

NODE_NAMES = {
    "repeat_text_py",
    "char_count_py",
    "greeting_py",
    "file_writer_py",
    "file_reader_py",
    "weather_agent_py",
}


class TestRegistration:
    def test_all_registered(self):
        names = {d.name for d in get_all_definitions()}
        assert NODE_NAMES.issubset(names)

    def test_node_count(self):
        defs = [d for d in get_all_definitions() if d.name in NODE_NAMES]
        assert len(defs) == 6

    def test_categories(self):
        defs = {d.name: d for d in get_all_definitions() if d.name in NODE_NAMES}
        assert defs["repeat_text_py"].category == "Custom/WASM"
        assert defs["file_writer_py"].category == "Custom/WASM/Storage"
        assert defs["weather_agent_py"].category == "Custom/WASM/AI"

    def test_weather_agent_long_running(self):
        defs = {d.name: d for d in get_all_definitions() if d.name in NODE_NAMES}
        assert defs["weather_agent_py"].long_running is True

    def test_weather_agent_permissions(self):
        defs = {d.name: d for d in get_all_definitions() if d.name in NODE_NAMES}
        perms = defs["weather_agent_py"].permissions
        assert "models" in perms
        assert "network:http" in perms

    def test_storage_permissions(self):
        defs = {d.name: d for d in get_all_definitions() if d.name in NODE_NAMES}
        assert "storage:read" in defs["file_writer_py"].permissions
        assert "storage:write" in defs["file_writer_py"].permissions
        assert "storage:read" in defs["file_reader_py"].permissions


# ── RepeatText ──────────────────────────────────────────────────────────

_repeat = RepeatText()


class TestRepeatText:
    def test_basic(self):
        result = _repeat.run(make_context({"input_text": "ab", "multiplier": 3}))
        assert result.outputs["output_text"] == "ababab"

    def test_zero_multiplier(self):
        result = _repeat.run(make_context({"input_text": "hello", "multiplier": 0}))
        assert result.outputs["output_text"] == ""

    def test_negative_multiplier(self):
        result = _repeat.run(make_context({"input_text": "x", "multiplier": -5}))
        assert result.outputs["output_text"] == ""

    def test_default_multiplier(self):
        result = _repeat.run(make_context({"input_text": "ok"}))
        assert result.outputs["output_text"] == "ok"

    def test_empty_text(self):
        result = _repeat.run(make_context({"input_text": "", "multiplier": 10}))
        assert result.outputs["output_text"] == ""

    def test_exec_activated(self):
        result = _repeat.run(make_context({"input_text": "a", "multiplier": 1}))
        assert "exec_out" in result.activate_exec


# ── CharCount ───────────────────────────────────────────────────────────

_char = CharCount()


class TestCharCount:
    def test_basic(self):
        result = _char.run(make_context({"input_text": "hello"}))
        assert result.outputs["char_count"] == 5

    def test_empty(self):
        result = _char.run(make_context({"input_text": ""}))
        assert result.outputs["char_count"] == 0

    def test_unicode(self):
        result = _char.run(make_context({"input_text": "héllo 🌍"}))
        assert result.outputs["char_count"] == len("héllo 🌍")

    def test_whitespace(self):
        result = _char.run(make_context({"input_text": "  a  b  "}))
        assert result.outputs["char_count"] == 8


# ── Greeting ────────────────────────────────────────────────────────────

_greet = Greeting()


class TestGreeting:
    def test_default_config(self):
        cfg = GreetingConfig()
        result = _greet.run(make_context({"config": cfg.to_dict()}))
        out = result.outputs["result"]
        assert out["message"] == "Hello"
        assert out["length"] == 5

    def test_uppercase(self):
        cfg = GreetingConfig(greeting="hi", uppercase=True, repeat=1)
        result = _greet.run(make_context({"config": cfg.to_dict()}))
        assert result.outputs["result"]["message"] == "HI"

    def test_repeat(self):
        cfg = GreetingConfig(greeting="yo", repeat=3)
        result = _greet.run(make_context({"config": cfg.to_dict()}))
        assert result.outputs["result"]["message"] == "yoyoyo"
        assert result.outputs["result"]["length"] == 6

    def test_uppercase_and_repeat(self):
        cfg = GreetingConfig(greeting="ab", uppercase=True, repeat=2)
        result = _greet.run(make_context({"config": cfg.to_dict()}))
        assert result.outputs["result"]["message"] == "ABAB"

    def test_zero_repeat_clamps_to_one(self):
        cfg = GreetingConfig(greeting="x", repeat=0)
        result = _greet.run(make_context({"config": cfg.to_dict()}))
        assert result.outputs["result"]["message"] == "x"

    def test_result_schema(self):
        defs = {d.name: d for d in get_all_definitions()}
        greeting_def = defs["greeting_py"]
        result_pin = next(p for p in greeting_def.pins if p.name == "result")
        schema = json.loads(result_pin.schema)
        assert "message" in schema.get("properties", {})
        assert "length" in schema.get("properties", {})


# ── FileWriter ──────────────────────────────────────────────────────────

_writer = FileWriter()


class TestFileWriter:
    def _dir_path(self) -> dict:
        return FlowPath(path="/test/dir", store_ref="mock").to_dict()

    def test_write_file(self):
        host = MockHostBridge()
        ctx = make_context(
            {"directory": self._dir_path(), "filename": "hello.txt", "content": "hi"},
            host=host,
        )
        result = _writer.run(ctx)
        assert result.error is None
        assert "exec_out" in result.activate_exec
        fp = result.outputs["file_path"]
        assert fp["path"] == "/test/dir/hello.txt"
        assert host.storage.get("/test/dir/hello.txt") == b"hi"

    def test_default_filename(self):
        host = MockHostBridge()
        ctx = make_context(
            {"directory": self._dir_path(), "content": "data"},
            host=host,
        )
        result = _writer.run(ctx)
        assert result.error is None
        assert host.storage.get("/test/dir/output.txt") == b"data"

    def test_file_count(self):
        host = MockHostBridge()
        host.storage["/test/dir/existing.txt"] = b"x"
        ctx = make_context(
            {"directory": self._dir_path(), "filename": "new.txt", "content": "y"},
            host=host,
        )
        result = _writer.run(ctx)
        assert result.outputs["file_count"] == 2

    def test_empty_content(self):
        host = MockHostBridge()
        ctx = make_context(
            {"directory": self._dir_path(), "filename": "empty.txt"},
            host=host,
        )
        _writer.run(ctx)
        assert host.storage.get("/test/dir/empty.txt") == b""


# ── FileReader ──────────────────────────────────────────────────────────

_reader = FileReader()


class TestFileReader:
    def test_read_existing(self):
        host = MockHostBridge()
        host.storage["/test/file.txt"] = b"content here"
        fp = FlowPath(path="/test/file.txt", store_ref="mock").to_dict()
        result = _reader.run(make_context({"file": fp}, host=host))
        assert result.outputs["content"] == "content here"
        assert result.outputs["exists"] is True

    def test_read_missing(self):
        host = MockHostBridge()
        fp = FlowPath(path="/test/nope.txt", store_ref="mock").to_dict()
        result = _reader.run(make_context({"file": fp}, host=host))
        assert result.outputs["content"] == ""
        assert result.outputs["exists"] is False

    def test_exec_activated(self):
        host = MockHostBridge()
        host.storage["/f.txt"] = b"hi"
        fp = FlowPath(path="/f.txt", store_ref="mock").to_dict()
        result = _reader.run(make_context({"file": fp}, host=host))
        assert "exec_out" in result.activate_exec


# ── WeatherAgent ────────────────────────────────────────────────────────


class WeatherMockHost(MockHostBridge):
    """Mock host that simulates LLM tool-use and HTTP weather APIs."""

    def __init__(self, *, llm_responses: list[str] | None = None, http_responses: dict[str, str] | None = None):
        super().__init__()
        self._llm_responses = list(llm_responses or [])
        self._llm_call_count = 0
        self._http_responses = http_responses or {}

    def llm_prompt(self, bit_json: str, messages_json: str, do_stream: bool) -> str | None:
        if self._llm_call_count < len(self._llm_responses):
            resp = self._llm_responses[self._llm_call_count]
            self._llm_call_count += 1
            return resp
        return '{"role": "assistant", "content": "No more responses"}'

    def http_request(self, method: int, url: str, headers: str, body: bytes | None) -> str | None:
        for pattern, response in self._http_responses.items():
            if pattern in url:
                return json.dumps({"status": 200, "headers": {}, "body": response})
        return json.dumps({"status": 200, "headers": {}, "body": "{}"})


_agent = WeatherAgent()


class TestWeatherAgent:
    def _bit(self) -> dict:
        return Bit(id="test-model").to_dict()

    def test_direct_response(self):
        """LLM returns plain text without tool calls."""
        host = WeatherMockHost(llm_responses=[
            '{"role": "assistant", "content": "It is sunny today!"}'
        ])
        ctx = make_context(
            {"model": self._bit(), "message": "What is the weather?"},
            host=host,
        )
        result = _agent.run(ctx)
        assert result.error is None
        assert result.outputs["response"] == "It is sunny today!"

    def test_tool_call_then_response(self):
        """LLM calls get_weather tool, then responds with final answer."""
        geo_body = json.dumps({
            "results": [{"name": "Tokyo", "latitude": 35.68, "longitude": 139.69}]
        })
        wx_body = json.dumps({
            "current": {
                "temperature_2m": 22.5,
                "apparent_temperature": 21.0,
                "relative_humidity_2m": 65,
                "wind_speed_10m": 12.3,
                "wind_direction_10m": 180,
                "weather_code": 2,
            }
        })
        host = WeatherMockHost(
            llm_responses=[
                json.dumps({
                    "content": "",
                    "tool_calls": [{
                        "id": "tc_1",
                        "function": {
                            "name": "get_weather",
                            "arguments": json.dumps({"location": "Tokyo"}),
                        },
                    }],
                }),
                '{"role": "assistant", "content": "The weather in Tokyo is 22.5°C and partly cloudy."}',
            ],
            http_responses={
                "geocoding-api.open-meteo.com": geo_body,
                "api.open-meteo.com": wx_body,
            },
        )
        ctx = make_context(
            {"model": self._bit(), "message": "Weather in Tokyo?"},
            host=host,
        )
        result = _agent.run(ctx)
        assert result.error is None
        assert "22.5" in result.outputs["response"]

    def test_no_llm_response(self):
        """LLM returns None — should fail gracefully."""
        host = WeatherMockHost(llm_responses=[])
        host.llm_prompt = lambda *a: None  # type: ignore[assignment]
        ctx = make_context(
            {"model": self._bit(), "message": "weather?"},
            host=host,
        )
        result = _agent.run(ctx)
        assert result.error is not None
        assert "no response" in result.error.lower()

    def test_plain_text_llm_response(self):
        """LLM returns non-JSON plain text."""
        host = WeatherMockHost(llm_responses=["Just a plain answer!"])
        ctx = make_context(
            {"model": self._bit(), "message": "hi"},
            host=host,
        )
        result = _agent.run(ctx)
        assert result.outputs["response"] == "Just a plain answer!"

    def test_max_turns_exhausted(self):
        """LLM keeps returning tool calls — agent caps at max_turns."""
        tool_resp = json.dumps({
            "content": "",
            "tool_calls": [{
                "id": "tc_loop",
                "function": {
                    "name": "get_weather",
                    "arguments": json.dumps({"location": "Loop"}),
                },
            }],
        })
        host = WeatherMockHost(llm_responses=[tool_resp] * 10)
        ctx = make_context(
            {"model": self._bit(), "message": "loop"},
            host=host,
        )
        result = _agent.run(ctx)
        assert "maximum turns" in result.outputs.get("response", "").lower()

    def test_unknown_tool(self):
        """LLM calls an unknown tool — agent returns error string and continues."""
        host = WeatherMockHost(llm_responses=[
            json.dumps({
                "content": "",
                "tool_calls": [{
                    "id": "tc_bad",
                    "function": {
                        "name": "nonexistent_tool",
                        "arguments": "{}",
                    },
                }],
            }),
            '{"content": "Fallback answer"}',
        ])
        ctx = make_context(
            {"model": self._bit(), "message": "test"},
            host=host,
        )
        result = _agent.run(ctx)
        assert result.outputs["response"] == "Fallback answer"


# ── WMO code helper ────────────────────────────────────────────────────

class TestWMOCodes:
    def test_clear(self):
        assert _wmo_condition(0) == "Clear sky"

    def test_rain(self):
        assert "rain" in _wmo_condition(63).lower()

    def test_thunderstorm(self):
        assert "thunderstorm" in _wmo_condition(95).lower()

    def test_unknown(self):
        assert _wmo_condition(999) == "Unknown"


# ── Weather tool function ──────────────────────────────────────────────

class TestGetWeatherTool:
    def test_geocode_failure(self):
        """http_get returns None → graceful error."""

        class NoHttpCtx:
            def http_get(self, url: str, headers=None):
                return None

        result = _get_weather_tool(NoHttpCtx(), "Nowhere")
        assert "could not geocode" in result.lower()

    def test_location_not_found(self):
        """Geocoding returns empty results list."""

        class EmptyGeoCtx:
            def http_get(self, url: str, headers=None):
                if "geocoding" in url:
                    return {"status": 200, "body": json.dumps({"results": []})}
                return None

        result = _get_weather_tool(EmptyGeoCtx(), "FakeTown")
        assert "not found" in result.lower()

    def test_full_flow(self):
        """Complete geocode + weather flow returns formatted string."""
        geo_body = json.dumps({
            "results": [{"name": "Berlin", "latitude": 52.52, "longitude": 13.41}]
        })
        wx_body = json.dumps({
            "current": {
                "temperature_2m": 18.0,
                "apparent_temperature": 16.5,
                "relative_humidity_2m": 72,
                "wind_speed_10m": 8.0,
                "wind_direction_10m": 270,
                "weather_code": 3,
            }
        })

        class FullCtx:
            def http_get(self, url: str, headers=None):
                if "geocoding" in url:
                    return {"status": 200, "body": geo_body}
                if "api.open-meteo.com" in url:
                    return {"status": 200, "body": wx_body}
                return None

        result = _get_weather_tool(FullCtx(), "Berlin")
        assert "Berlin" in result
        assert "18.0" in result
        assert "Overcast" in result


# ── Dispatch ────────────────────────────────────────────────────────────

class TestDispatch:
    def test_repeat_text_dispatch(self):
        ctx = make_context({"input_text": "z", "multiplier": 4}, node_name="repeat_text_py")
        result = run_node("repeat_text_py", ctx)
        assert result.outputs["output_text"] == "zzzz"

    def test_char_count_dispatch(self):
        ctx = make_context({"input_text": "test"}, node_name="char_count_py")
        result = run_node("char_count_py", ctx)
        assert result.outputs["char_count"] == 4

    def test_unknown_node_dispatch(self):
        ctx = make_context({}, node_name="no_such_node")
        result = run_node("no_such_node", ctx)
        assert result.error is not None
