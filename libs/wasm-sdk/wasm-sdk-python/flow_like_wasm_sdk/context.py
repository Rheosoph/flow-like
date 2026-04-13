from __future__ import annotations

import json
from typing import TYPE_CHECKING, Any

from flow_like_wasm_sdk.host import HostBridge, _host
from flow_like_wasm_sdk.types import ExecutionInput, ExecutionResult, LogLevel

if TYPE_CHECKING:
    from flow_like_wasm_sdk.interop import (
        Bit, CachedEmbeddingModel, ChatMessage, FlowImage,
    )

_CHUNK_SIZE = 8 * 1024 * 1024  # 8 MiB, safely under host 10 MiB limit

_B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
_B64_INV = {c: i for i, c in enumerate(_B64)}


def _b64decode(s: str) -> str:
    s = s.rstrip("=")
    out = bytearray()
    buf = 0
    bits = 0
    for ch in s:
        val = _B64_INV.get(ch)
        if val is None:
            continue
        buf = (buf << 6) | val
        bits += 6
        if bits >= 8:
            bits -= 8
            out.append((buf >> bits) & 0xFF)
    return out.decode("utf-8")


class Context:
    """Execution context providing typed input access, output setting, logging, and streaming."""

    def __init__(self, execution_input: ExecutionInput, host: HostBridge | None = None) -> None:
        self._input = execution_input
        self._result = ExecutionResult.ok()
        self._host = host or _host

    @classmethod
    def from_dict(cls, data: dict[str, Any], host: HostBridge | None = None) -> Context:
        return cls(ExecutionInput.from_dict(data), host)

    @classmethod
    def from_json(cls, json_str: str, host: HostBridge | None = None) -> Context:
        return cls(ExecutionInput.from_json(json_str), host)

    @property
    def node_id(self) -> str:
        return self._input.node_id

    @property
    def node_name(self) -> str:
        return self._input.node_name

    @property
    def run_id(self) -> str:
        return self._input.run_id

    @property
    def app_id(self) -> str:
        return self._input.app_id

    @property
    def board_id(self) -> str:
        return self._input.board_id

    @property
    def user_id(self) -> str:
        return self._input.user_id

    @property
    def stream_enabled(self) -> bool:
        return self._input.stream_state

    @property
    def log_level(self) -> int:
        return self._input.log_level

    def get_input(self, name: str) -> Any:
        return self._input.inputs.get(name)

    def get_string(self, name: str, default: str | None = None) -> str | None:
        val = self.get_input(name)
        if val is None:
            return default
        return str(val)

    def get_i64(self, name: str, default: int | None = None) -> int | None:
        val = self.get_input(name)
        if val is None:
            return default
        return int(val)

    def get_f64(self, name: str, default: float | None = None) -> float | None:
        val = self.get_input(name)
        if val is None:
            return default
        return float(val)

    def get_bool(self, name: str, default: bool | None = None) -> bool | None:
        val = self.get_input(name)
        if val is None:
            return default
        return bool(val)

    def require_input(self, name: str) -> Any:
        val = self.get_input(name)
        if val is None:
            raise ValueError(f"Required input '{name}' not provided")
        return val

    def set_output(self, name: str, value: Any) -> None:
        self._result.set_output(name, value)

    def activate_exec(self, pin_name: str) -> None:
        self._result.exec(pin_name)

    def set_pending(self, pending: bool) -> None:
        self._result.set_pending(pending)

    def debug(self, message: str) -> None:
        if self._input.log_level <= LogLevel.DEBUG:
            self._host.log(LogLevel.DEBUG, message)

    def info(self, message: str) -> None:
        if self._input.log_level <= LogLevel.INFO:
            self._host.log(LogLevel.INFO, message)

    def warn(self, message: str) -> None:
        if self._input.log_level <= LogLevel.WARN:
            self._host.log(LogLevel.WARN, message)

    def error(self, message: str) -> None:
        if self._input.log_level <= LogLevel.ERROR:
            self._host.log(LogLevel.ERROR, message)

    def stream_text(self, text: str) -> None:
        if self._input.stream_state:
            self._host.stream("text", text)

    def stream_json(self, data: Any) -> None:
        if self._input.stream_state:
            self._host.stream("json", json.dumps(data))

    def stream_progress(self, progress: float, message: str) -> None:
        if self._input.stream_state:
            payload = json.dumps({"progress": progress, "message": message})
            self._host.stream("progress", payload)

    def get_variable(self, name: str) -> Any:
        return self._host.get_variable(name)

    def set_variable(self, name: str, value: Any) -> bool:
        return self._host.set_variable(name, value)

    def delete_variable(self, name: str) -> None:
        self._host.delete_variable(name)

    def has_variable(self, name: str) -> bool:
        return self._host.has_variable(name)

    def cache_get(self, key: str) -> Any:
        return self._host.cache_get(key)

    def cache_set(self, key: str, value: Any) -> None:
        self._host.cache_set(key, value)

    def cache_delete(self, key: str) -> None:
        self._host.cache_delete(key)

    def cache_has(self, key: str) -> bool:
        return self._host.cache_has(key)

    def storage_dir(self, node_scoped: bool = False) -> dict | None:
        return self._host.storage_dir(node_scoped)

    def upload_dir(self) -> dict | None:
        return self._host.upload_dir()

    def cache_dir(self, node_scoped: bool = False, user_scoped: bool = False) -> dict | None:
        return self._host.cache_dir(node_scoped, user_scoped)

    def user_dir(self, node_scoped: bool = False) -> dict | None:
        return self._host.user_dir(node_scoped)

    def storage_read(self, flow_path: dict) -> bytes | None:
        return self._host.storage_read(flow_path)

    def storage_write(self, flow_path: dict, data: bytes) -> bool:
        if len(data) <= _CHUNK_SIZE:
            return self._host.storage_write(flow_path, data)
        return self._storage_write_chunked(flow_path, data)

    def _storage_write_chunked(self, flow_path: dict, data: bytes) -> bool:
        write_id = self._host.storage_write_start(flow_path, len(data))
        if write_id is None:
            return False
        for i in range(0, len(data), _CHUNK_SIZE):
            if not self._host.storage_write_chunk(write_id, data[i:i + _CHUNK_SIZE]):
                return False
        return self._host.storage_write_finish(write_id)

    def storage_list(self, flow_path: dict) -> list[dict] | None:
        return self._host.storage_list(flow_path)

    def embed_text(self, bit: dict, texts: list[str]) -> list[list[float]] | None:
        return self._host.embed_text(bit, texts)

    def http_request(self, method: int, url: str, headers: dict[str, str] | None = None, body: bytes | None = None) -> dict | None:
        result = self._host.http_request(method, url, json.dumps(headers or {}), body)
        if result is None:
            return None
        parsed = json.loads(result)
        if isinstance(parsed, dict) and "body_base64" in parsed:
            try:
                parsed["body"] = _b64decode(parsed["body_base64"])
            except Exception:
                parsed["body"] = ""
        return parsed

    def http_get(self, url: str, headers: dict[str, str] | None = None) -> dict | None:
        return self.http_request(0, url, headers)

    def http_post(self, url: str, body: bytes | None = None, headers: dict[str, str] | None = None) -> dict | None:
        return self.http_request(1, url, headers, body)

    def get_oauth_token(self, provider: str) -> dict | None:
        return self._host.get_oauth_token(provider)

    def has_oauth_token(self, provider: str) -> bool:
        return self._host.has_oauth_token(provider)

    # ── LLM / VLM ───────────────────────────────────────────────────

    def llm_prompt(
        self,
        bit: Bit,
        messages: list[ChatMessage],
        stream: bool = False,
        tools: list[dict[str, Any]] | None = None,
        temperature: float | None = None,
        max_tokens: int | None = None,
        tool_choice: Any | None = None,
        output_schema: dict | None = None,
    ) -> str | None:
        msg_list = [m.to_dict() for m in messages]
        payload: dict[str, Any] = {"messages": msg_list}
        if tools:
            payload["tools"] = tools
        if temperature is not None:
            payload["temperature"] = temperature
        if max_tokens is not None:
            payload["max_tokens"] = max_tokens
        if tool_choice is not None:
            payload["tool_choice"] = tool_choice
        if output_schema is not None:
            payload["output_schema"] = output_schema
        return self._host.llm_prompt(bit.to_json(), json.dumps(payload), stream)

    def llm_prompt_stream(
        self,
        bit: Bit,
        messages: list[ChatMessage],
        tools: list[dict[str, Any]] | None = None,
        temperature: float | None = None,
        max_tokens: int | None = None,
        tool_choice: Any | None = None,
        output_schema: dict | None = None,
    ) -> str | None:
        msg_list = [m.to_dict() for m in messages]
        payload: dict[str, Any] = {"messages": msg_list}
        if tools:
            payload["tools"] = tools
        if temperature is not None:
            payload["temperature"] = temperature
        if max_tokens is not None:
            payload["max_tokens"] = max_tokens
        if tool_choice is not None:
            payload["tool_choice"] = tool_choice
        if output_schema is not None:
            payload["output_schema"] = output_schema
        return self._host.llm_prompt_stream(bit.to_json(), json.dumps(payload))

    # ── Embedding ────────────────────────────────────────────────────

    def embed_text_query(self, model: CachedEmbeddingModel, texts: list[str]) -> list[list[float]] | None:
        result = self._host.embed_text_query(model.to_json(), json.dumps(texts))
        return json.loads(result) if result is not None else None

    def embed_text_document(self, model: CachedEmbeddingModel, texts: list[str]) -> list[list[float]] | None:
        result = self._host.embed_text_document(model.to_json(), json.dumps(texts))
        return json.loads(result) if result is not None else None

    def embed_image(self, model: CachedEmbeddingModel, image: FlowImage) -> list[float] | None:
        img_bytes = image.to_bytes(self)
        if img_bytes is None:
            return None
        result = self._host.embed_image(model.to_json(), img_bytes)
        return json.loads(result) if result is not None else None

    # ── Image ────────────────────────────────────────────────────────

    def image_from_bytes(self, data: bytes, fmt: str = "png") -> FlowImage | None:
        from flow_like_wasm_sdk.interop import FlowImage as _FI
        result = self._host.image_from_bytes(data, fmt)
        if result is None:
            return None
        return _FI.from_json(result)

    def image_to_bytes(self, image: FlowImage, fmt: str = "png") -> bytes | None:
        return self._host.image_to_bytes(image.image_ref, fmt)

    # ── Database ─────────────────────────────────────────────────────

    def db_query(self, op: int, connection: Any, payload: dict) -> Any:
        conn_json = json.dumps(connection.to_dict() if hasattr(connection, "to_dict") else connection)
        result = self._host.db_query(op, conn_json, json.dumps(payload))
        return json.loads(result) if result is not None else None

    # ── Finalization ─────────────────────────────────────────────────

    def success(self) -> ExecutionResult:
        self._result.exec("exec_out")
        return self._result

    def fail(self, error: str) -> ExecutionResult:
        self._result.error = error
        return self._result

    def finish(self) -> ExecutionResult:
        return self._result
