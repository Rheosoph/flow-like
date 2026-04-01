"""
Flow-Like WASM Node Template — Example Nodes

  1. WriteText     — writes text content to a FlowPath location
  2. WeatherAgent  — AI agent that looks up weather via LangChain tool-use
"""

import json
from typing import Any

from flow_like_wasm_sdk import (
    Bit,
    ExecutionResult,
    FlowPath,
    Input,
    NodeScores,
    Output,
    WasmNode,
)


# ── 1) Write Text ────────────────────────────────────────────────────────

class WriteText(WasmNode, name="write_text_py", title="Write Text", category="Custom/WASM/Storage"):
    """Writes text content to a file at the given FlowPath."""
    permissions = ["storage:read", "storage:write"]
    scores = NodeScores(privacy=8, security=7, performance=8, governance=7, reliability=8, cost=10)

    input_text: str = Input(default="", description="Text content to write")
    input_path: FlowPath = Input(description="Destination file path")
    output_path: FlowPath = Output(description="Path of the written file")

    def run(self, ctx) -> ExecutionResult:
        path: FlowPath = ctx.input_path
        if path is None:
            return ctx.fail("input_path is not connected")
        text: str = ctx.input_text or ""
        ctx.info(f"WriteText: writing {len(text)} bytes to {path.path} (store={path.store_ref})")
        if not path.put_string(ctx, text):
            return ctx.fail(
                f"storage_write returned false for path={path.path} store_ref={path.store_ref}"
            )
        ctx.output_path = path
        return ctx.success()


# ── 2) Weather Agent (LangChain + HTTP tool-use) ────────────────────────

_WMO_CODES: dict[int, str] = {
    0: "Clear sky",
    1: "Mainly clear", 2: "Partly cloudy", 3: "Overcast",
    45: "Foggy", 48: "Depositing rime fog",
    51: "Light drizzle", 53: "Moderate drizzle", 55: "Dense drizzle",
    56: "Light freezing drizzle", 57: "Dense freezing drizzle",
    61: "Slight rain", 63: "Moderate rain", 65: "Heavy rain",
    66: "Light freezing rain", 67: "Heavy freezing rain",
    71: "Slight snow", 73: "Moderate snow", 75: "Heavy snow",
    77: "Snow grains",
    80: "Slight rain showers", 81: "Moderate rain showers", 82: "Violent rain showers",
    85: "Slight snow showers", 86: "Heavy snow showers",
    95: "Thunderstorm", 96: "Thunderstorm with slight hail", 99: "Thunderstorm with heavy hail",
}


def _quote_plus(s: str) -> str:
    """Minimal URL percent-encoding (replaces urllib.parse.quote_plus)."""
    safe = set(b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.-~")
    out: list[str] = []
    for byte in s.encode("utf-8"):
        if byte == 0x20:
            out.append("+")
        elif byte in safe:
            out.append(chr(byte))
        else:
            out.append(f"%{byte:02X}")
    return "".join(out)


def _get_weather_tool(ctx: Any, location: str) -> str:
    """Geocode *location* via Open-Meteo then fetch current conditions."""
    geo_url = (
        f"https://geocoding-api.open-meteo.com/v1/search"
        f"?name={_quote_plus(location)}&count=1&language=en&format=json"
    )
    geo = ctx.http_get(geo_url)
    if geo is None:
        return f"Could not geocode location: {location}"

    try:
        body = json.loads(geo.get("body", "")) if isinstance(geo.get("body"), str) else geo.get("body", {})
    except (json.JSONDecodeError, TypeError):
        return f"Could not geocode location: {location}"

    results = body.get("results") if isinstance(body, dict) else None
    if not results:
        return f"Location not found: {location}"

    hit = results[0]
    lat, lon = hit.get("latitude", 0.0), hit.get("longitude", 0.0)
    resolved = hit.get("name", location)

    wx_url = (
        f"https://api.open-meteo.com/v1/forecast"
        f"?latitude={lat}&longitude={lon}"
        f"&current=temperature_2m,relative_humidity_2m,apparent_temperature,"
        f"wind_speed_10m,wind_direction_10m,weather_code"
    )
    wx = ctx.http_get(wx_url)
    if wx is None:
        return f"Weather API request failed for {resolved}"

    try:
        wx_body = json.loads(wx.get("body", "")) if isinstance(wx.get("body"), str) else wx.get("body", {})
    except (json.JSONDecodeError, TypeError):
        return f"Weather API parse failed for {resolved}"

    current = wx_body.get("current", {}) if isinstance(wx_body, dict) else {}
    code = int(current.get("weather_code", 0))
    condition = _WMO_CODES.get(code, "Unknown")

    return (
        f"Current weather in {resolved}: {current.get('temperature_2m', 0)}°C "
        f"(feels like {current.get('apparent_temperature', 0)}°C), "
        f"{condition}, humidity {current.get('relative_humidity_2m', 0)}%, "
        f"wind {current.get('wind_speed_10m', 0)} km/h from {current.get('wind_direction_10m', 0)}°"
    )


_WEATHER_TOOL_DEF = {
    "type": "function",
    "function": {
        "name": "get_weather",
        "description": "Get the current weather for a given location.",
        "parameters": {
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "City name, e.g. 'San Francisco' or 'Tokyo'",
                }
            },
            "required": ["location"],
        },
    },
}


class WeatherAgent(WasmNode, name="weather_agent_py", title="Weather Agent", category="Custom/WASM/AI"):
    """AI agent that looks up real weather conditions using LangChain tool-use."""
    permissions = ["models", "network:http"]
    long_running = True
    scores = NodeScores(privacy=5, security=4, performance=5, governance=5, reliability=6, cost=3)

    model: Bit = Input(description="LLM model descriptor")
    message: str = Input(
        default="What's the weather like in San Francisco?",
        description="User message",
    )
    response: str = Output(description="Agent response")

    def run(self, ctx) -> ExecutionResult:
        ctx.info("WeatherAgent: starting")
        bit: Bit = ctx.model
        if bit is None:
            return ctx.fail("model input is not connected")

        from flow_like_wasm_sdk.langchain import (
            FlowLikeChatModel,
            HumanMessage,
            SystemMessage,
            ToolMessage,
        )

        llm = FlowLikeChatModel(bit=bit, ctx=ctx)
        llm_with_tools = llm.bind_tools([_WEATHER_TOOL_DEF])

        messages = [
            SystemMessage(content=(
                "You are a helpful weather assistant. "
                "Use the get_weather tool to look up current weather conditions."
            )),
            HumanMessage(content=ctx.message),
        ]

        max_turns = 5
        for _ in range(max_turns):
            result = llm_with_tools.invoke(messages)
            messages.append(result)

            if not result.tool_calls:
                ctx.response = str(result.content)
                return ctx.success()

            for tc in result.tool_calls:
                if tc["name"] == "get_weather":
                    location = tc.get("args", {}).get("location", "unknown")
                    ctx.info(f"WeatherAgent: looking up weather for {location}")
                    tool_output = _get_weather_tool(ctx, location)
                else:
                    tool_output = f"Unknown tool: {tc['name']}"

                messages.append(ToolMessage(
                    content=tool_output,
                    tool_call_id=tc.get("id", ""),
                ))

        ctx.response = "Agent reached maximum turns without final answer"
        return ctx.success()
