"""
WASM Component Model bridge for Flow-Like Python nodes.

This module implements the WIT world exports (get-node, run, get-abi-version)
and connects the WIT host imports to the SDK's HostBridge.

componentize-py compiles this into a WASM component that the Flow-Like runtime
can load via the component-model feature.

This file is shipped as part of the SDK package so that templates do not need
to maintain their own copy.
"""

from __future__ import annotations

import json
from typing import Any

from wit_world.imports import logging as wit_logging
from wit_world.imports import variables as wit_variables
from wit_world.imports import cache as wit_cache
from wit_world.imports import streaming as wit_streaming
from wit_world.imports import metadata as wit_metadata
from wit_world.imports import storage as wit_storage
from wit_world.imports import models as wit_models
from wit_world.imports import auth as wit_auth
from wit_world.imports import http as wit_http
from wit_world.imports import image as wit_image
from wit_world.imports import db as wit_db
from wit_world.imports import schema as wit_schema


def _make_bridge():
    """Create a HostBridge backed by real WIT host imports.

    Imported lazily so the SDK can be used outside WASM without errors.
    """
    from flow_like_wasm_sdk.host import HostBridge

    class WitHostBridge(HostBridge):

        def log(self, level: int, message: str) -> None:
            wit_logging.log(level, message)

        def stream(self, event_type: str, data: str) -> None:
            wit_streaming.emit(event_type, data)

        def stream_text(self, content: str) -> None:
            wit_streaming.text(content)

        def get_variable(self, name: str) -> Any:
            result = wit_variables.get_var(name)
            if result is None:
                return None
            return json.loads(result)

        def set_variable(self, name: str, value: Any) -> bool:
            wit_variables.set_var(name, json.dumps(value))
            return True

        def delete_variable(self, name: str) -> None:
            wit_variables.delete_var(name)

        def has_variable(self, name: str) -> bool:
            return wit_variables.has_var(name)

        def cache_get(self, key: str) -> Any:
            result = wit_cache.cache_get(key)
            if result is None:
                return None
            return json.loads(result)

        def cache_set(self, key: str, value: Any) -> None:
            wit_cache.cache_set(key, json.dumps(value))

        def cache_delete(self, key: str) -> None:
            wit_cache.cache_delete(key)

        def cache_has(self, key: str) -> bool:
            return wit_cache.cache_has(key)

        def time_now(self) -> int:
            return wit_metadata.time_now()

        def random(self) -> int:
            return wit_metadata.random()

        def storage_dir(self, node_scoped: bool) -> dict | None:
            result = wit_storage.storage_dir(node_scoped)
            return json.loads(result) if result is not None else None

        def upload_dir(self) -> dict | None:
            result = wit_storage.upload_dir()
            return json.loads(result) if result is not None else None

        def cache_dir(self, node_scoped: bool, user_scoped: bool) -> dict | None:
            result = wit_storage.cache_dir(node_scoped, user_scoped)
            return json.loads(result) if result is not None else None

        def user_dir(self, node_scoped: bool) -> dict | None:
            result = wit_storage.user_dir(node_scoped)
            return json.loads(result) if result is not None else None

        def storage_read(self, flow_path: dict) -> bytes | None:
            result = wit_storage.read_file(json.dumps(flow_path))
            return bytes(result) if result is not None else None

        def storage_write(self, flow_path: dict, data: bytes) -> bool:
            return wit_storage.write_file(json.dumps(flow_path), data)

        def storage_write_start(self, flow_path: dict, total_size: int) -> str | None:
            return wit_storage.write_file_start(json.dumps(flow_path), total_size)

        def storage_write_chunk(self, write_id: str, data: bytes) -> bool:
            return wit_storage.write_file_chunk(write_id, data)

        def storage_write_finish(self, write_id: str) -> bool:
            return wit_storage.write_file_finish(write_id)

        def storage_list(self, flow_path: dict) -> list[dict] | None:
            result = wit_storage.list_files(json.dumps(flow_path))
            return json.loads(result) if result is not None else None

        def embed_text(self, bit: dict, texts: list[str]) -> list[list[float]] | None:
            result = wit_models.embed_text(json.dumps(bit), json.dumps(texts))
            return json.loads(result) if result is not None else None

        def get_oauth_token(self, provider: str) -> dict | None:
            result = wit_auth.get_oauth_token(provider)
            return json.loads(result) if result is not None else None

        def has_oauth_token(self, provider: str) -> bool:
            return wit_auth.has_oauth_token(provider)

        def http_request(self, method: int, url: str, headers: str, body: bytes | None = None) -> str | None:
            return wit_http.request(method, url, headers, body)

        def llm_prompt(self, bit_json: str, messages_json: str, do_stream: bool) -> str | None:
            return wit_models.llm_prompt(bit_json, messages_json, do_stream)

        def llm_prompt_stream(self, bit_json: str, request_json: str) -> str | None:
            return wit_models.llm_prompt_stream(bit_json, request_json)

        def embed_text_query(self, model_json: str, texts_json: str) -> str | None:
            return wit_models.embed_text_query(model_json, texts_json)

        def embed_text_document(self, model_json: str, texts_json: str) -> str | None:
            return wit_models.embed_text_document(model_json, texts_json)

        def embed_image(self, model_json: str, image_data: bytes) -> str | None:
            return wit_models.embed_image(model_json, image_data)

        def image_from_bytes(self, data: bytes, fmt: str) -> str | None:
            return wit_image.from_bytes(data, fmt)

        def image_to_bytes(self, image_ref: str, fmt: str) -> bytes | None:
            result = wit_image.to_bytes(image_ref, fmt)
            return bytes(result) if result is not None else None

        def db_query(self, op: int, connection_json: str, payload_json: str) -> str | None:
            return wit_db.query(op, connection_json, payload_json)

        def get_type_schema(self, type_name: str) -> str | None:
            return wit_schema.get_type_schema(type_name)

        def list_types(self) -> list[str] | None:
            result = wit_schema.list_types()
            if result is not None:
                return json.loads(result)
            return None

    return WitHostBridge()
