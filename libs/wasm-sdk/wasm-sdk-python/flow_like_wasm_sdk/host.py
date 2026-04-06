from __future__ import annotations

from typing import Any


class HostBridge:
    """Interface for host function calls. Replaced at runtime by actual WASM host imports."""

    def log(self, level: int, message: str) -> None:
        pass

    def stream(self, event_type: str, data: str) -> None:
        pass

    def stream_text(self, content: str) -> None:
        pass

    def get_variable(self, name: str) -> Any:
        return None

    def set_variable(self, name: str, value: Any) -> bool:
        return False

    def delete_variable(self, name: str) -> None:
        pass

    def has_variable(self, name: str) -> bool:
        return False

    def cache_get(self, key: str) -> Any:
        return None

    def cache_set(self, key: str, value: Any) -> None:
        pass

    def cache_delete(self, key: str) -> None:
        pass

    def cache_has(self, key: str) -> bool:
        return False

    def time_now(self) -> int:
        return 0

    def random(self) -> int:
        return 0

    def storage_dir(self, node_scoped: bool) -> dict | None:
        return None

    def upload_dir(self) -> dict | None:
        return None

    def cache_dir(self, node_scoped: bool, user_scoped: bool) -> dict | None:
        return None

    def user_dir(self, node_scoped: bool) -> dict | None:
        return None

    def storage_read(self, flow_path: dict) -> bytes | None:
        return None

    def storage_write(self, flow_path: dict, data: bytes) -> bool:
        return False

    def storage_list(self, flow_path: dict) -> list[dict] | None:
        return None

    def embed_text(self, bit: dict, texts: list[str]) -> list[list[float]] | None:
        return None

    def get_oauth_token(self, provider: str) -> dict | None:
        return None

    def has_oauth_token(self, provider: str) -> bool:
        return False

    def http_request(self, method: int, url: str, headers: str, body: bytes | None) -> str | None:
        return None

    def llm_prompt(self, bit_json: str, messages_json: str, do_stream: bool) -> str | None:
        return None

    def llm_prompt_stream(self, bit_json: str, request_json: str) -> str | None:
        return None

    def embed_text_query(self, model_json: str, texts_json: str) -> str | None:
        return None

    def embed_text_document(self, model_json: str, texts_json: str) -> str | None:
        return None

    def embed_image(self, model_json: str, image_data: bytes) -> str | None:
        return None

    def image_from_bytes(self, data: bytes, fmt: str) -> str | None:
        return None

    def image_to_bytes(self, image_ref: str, fmt: str) -> bytes | None:
        return None

    def db_query(self, op: int, connection_json: str, payload_json: str) -> str | None:
        return None

    def get_type_schema(self, type_name: str) -> str | None:
        return None

    def list_types(self) -> list[str] | None:
        return None


class MockHostBridge(HostBridge):
    """Host bridge for local testing with captured logs and streams."""

    def __init__(self) -> None:
        self.logs: list[tuple[int, str]] = []
        self.streams: list[tuple[str, str]] = []
        self.variables: dict[str, Any] = {}
        self.cache_data: dict[str, Any] = {}
        self._time: int = 0
        self._random_value: int = 42
        self.storage: dict[str, bytes] = {}
        self._embeddings: list[list[float]] = [[0.1, 0.2, 0.3]]
        self.oauth_tokens: dict[str, dict] = {}

    def log(self, level: int, message: str) -> None:
        self.logs.append((level, message))

    def stream(self, event_type: str, data: str) -> None:
        self.streams.append((event_type, data))

    def stream_text(self, content: str) -> None:
        self.streams.append(("text", content))

    def get_variable(self, name: str) -> Any:
        return self.variables.get(name)

    def set_variable(self, name: str, value: Any) -> bool:
        self.variables[name] = value
        return True

    def delete_variable(self, name: str) -> None:
        self.variables.pop(name, None)

    def has_variable(self, name: str) -> bool:
        return name in self.variables

    def cache_get(self, key: str) -> Any:
        return self.cache_data.get(key)

    def cache_set(self, key: str, value: Any) -> None:
        self.cache_data[key] = value

    def cache_delete(self, key: str) -> None:
        self.cache_data.pop(key, None)

    def cache_has(self, key: str) -> bool:
        return key in self.cache_data

    def time_now(self) -> int:
        return self._time

    def random(self) -> int:
        return self._random_value

    def storage_dir(self, node_scoped: bool) -> dict | None:
        return {"path": "storage/node" if node_scoped else "storage", "store_ref": "mock_store", "cache_store_ref": None}

    def upload_dir(self) -> dict | None:
        return {"path": "upload", "store_ref": "mock_store", "cache_store_ref": None}

    def cache_dir(self, node_scoped: bool, user_scoped: bool) -> dict | None:
        return {"path": "tmp/cache", "store_ref": "mock_store", "cache_store_ref": None}

    def user_dir(self, node_scoped: bool) -> dict | None:
        return {"path": "users/mock", "store_ref": "mock_store", "cache_store_ref": None}

    def storage_read(self, flow_path: dict) -> bytes | None:
        return self.storage.get(flow_path.get("path", ""))

    def storage_write(self, flow_path: dict, data: bytes) -> bool:
        self.storage[flow_path.get("path", "")] = data
        return True

    def storage_list(self, flow_path: dict) -> list[dict] | None:
        prefix = flow_path.get("path", "")
        return [{"path": k, "store_ref": flow_path.get("store_ref", ""), "cache_store_ref": None} for k in self.storage if k.startswith(prefix)]

    def embed_text(self, bit: dict, texts: list[str]) -> list[list[float]] | None:
        return [self._embeddings[0][:] for _ in texts]

    def get_oauth_token(self, provider: str) -> dict | None:
        return self.oauth_tokens.get(provider)

    def has_oauth_token(self, provider: str) -> bool:
        return provider in self.oauth_tokens

    def http_request(self, method: int, url: str, headers: str, body: bytes | None) -> str | None:
        import json as _json
        return _json.dumps({"status": 200, "headers": {}, "body_base64": "e30="})

    def llm_prompt(self, bit_json: str, messages_json: str, do_stream: bool) -> str | None:
        return '{"role": "assistant", "content": "Mock LLM response"}'

    def llm_prompt_stream(self, bit_json: str, request_json: str) -> str | None:
        return '{"role": "assistant", "content": "Mock LLM streaming response", "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}}'

    def embed_text_query(self, model_json: str, texts_json: str) -> str | None:
        import json as _json
        texts = _json.loads(texts_json)
        return _json.dumps([self._embeddings[0][:] for _ in texts])

    def embed_text_document(self, model_json: str, texts_json: str) -> str | None:
        import json as _json
        texts = _json.loads(texts_json)
        return _json.dumps([self._embeddings[0][:] for _ in texts])

    def embed_image(self, model_json: str, image_data: bytes) -> str | None:
        import json as _json
        return _json.dumps(self._embeddings[0][:])

    def image_from_bytes(self, data: bytes, fmt: str) -> str | None:
        import json as _json
        self._image_counter = getattr(self, "_image_counter", 0) + 1
        ref = f"mock_img_{self._image_counter}"
        self.storage[f"__images__/{ref}"] = data
        return _json.dumps({"image_ref": ref})

    def image_to_bytes(self, image_ref: str, fmt: str) -> bytes | None:
        return self.storage.get(f"__images__/{image_ref}")

    def db_query(self, op: int, connection_json: str, payload_json: str) -> str | None:
        import json as _json
        return _json.dumps([])


_host: HostBridge = HostBridge()


def set_host(host: HostBridge) -> None:
    global _host
    _host = host


def get_host() -> HostBridge:
    return _host
