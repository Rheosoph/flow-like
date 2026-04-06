"""
Domain types for WASM node I/O: FlowPath, FlowImage, Bit (LLM), ChatMessage.

These mirror the Rust SDK's ``interop.rs`` and are serialised as JSON when
crossing the WASM boundary.  Each type is a plain dataclass so it can be
round-tripped through ``json.dumps`` / ``json.loads`` without extra ceremony.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from flow_like_wasm_sdk.context import Context


def _host_schema(type_name: str) -> dict[str, Any] | None:
    """Try to fetch the authoritative JSON schema from the WASM host.

    Returns ``None`` when running outside WASM (e.g. build-time extraction).
    """
    try:
        from wit_world.imports.schema import get_type_schema
        result = get_type_schema(type_name)
        if result is not None:
            return json.loads(result)
    except Exception:
        pass
    return None


# ── FlowPath — handle to a file in an object store ─────────────────────


@dataclass
class FlowPath:
    """Reference to a file/directory in a host-resolved object store.

    Serialisation format::

        {"path": "uploads/doc.pdf", "store_ref": "local", "cache_store_ref": null}
    """

    path: str
    store_ref: str
    cache_store_ref: str | None = None

    # ── I/O (require host Context) ──────────────────────────────────

    def get(self, ctx: Context) -> bytes | None:
        """Read this path's contents from the object store."""
        return ctx.storage_read(self.to_dict())

    def put(self, ctx: Context, data: bytes) -> bool:
        """Write *data* to this path in the object store."""
        return ctx.storage_write(self.to_dict(), data)

    read = get
    write = put

    def list(self, ctx: Context) -> list[FlowPath] | None:
        """List children under this directory path."""
        raw = ctx.storage_list(self.to_dict())
        if raw is None:
            return None
        return [FlowPath.from_dict(d) for d in raw]

    def exists(self, ctx: Context) -> bool:
        return self.get(ctx) is not None

    def get_string(self, ctx: Context, encoding: str = "utf-8") -> str | None:
        data = self.get(ctx)
        return data.decode(encoding) if data is not None else None

    def put_string(self, ctx: Context, text: str, encoding: str = "utf-8") -> bool:
        return self.put(ctx, text.encode(encoding))

    def get_json(self, ctx: Context) -> Any:
        data = self.get(ctx)
        return json.loads(data) if data is not None else None

    def put_json(self, ctx: Context, obj: Any) -> bool:
        return self.put(ctx, json.dumps(obj).encode())

    # ── Path manipulation (pure, no host calls) ─────────────────────

    def child(self, name: str) -> FlowPath:
        sep = "" if self.path.endswith("/") or not self.path else "/"
        return FlowPath(f"{self.path}{sep}{name}", self.store_ref, self.cache_store_ref)

    def parent(self) -> FlowPath | None:
        trimmed = self.path.rstrip("/")
        idx = trimmed.rfind("/")
        if idx < 0:
            return None
        return FlowPath(trimmed[:idx], self.store_ref, self.cache_store_ref)

    def file_name(self) -> str | None:
        trimmed = self.path.rstrip("/")
        idx = trimmed.rfind("/")
        if idx >= 0:
            return trimmed[idx + 1:]
        return trimmed or None

    def extension(self) -> str | None:
        name = self.file_name()
        if name is None:
            return None
        idx = name.rfind(".")
        return name[idx + 1:] if idx >= 0 else None

    def with_extension(self, ext: str) -> FlowPath:
        trimmed = self.path.rstrip("/")
        dot = trimmed.rfind(".")
        slash = trimmed.rfind("/")
        base = trimmed[:dot] if dot >= 0 and (slash < 0 or dot > slash) else trimmed
        return FlowPath(f"{base}.{ext}", self.store_ref, self.cache_store_ref)

    def join(self, *segments: str) -> FlowPath:
        current = self
        for seg in segments:
            current = current.child(seg)
        return current

    # ── Serialisation ────────────────────────────────────────────────

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"path": self.path, "store_ref": self.store_ref}
        if self.cache_store_ref is not None:
            d["cache_store_ref"] = self.cache_store_ref
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> FlowPath:
        return cls(
            path=data["path"],
            store_ref=data["store_ref"],
            cache_store_ref=data.get("cache_store_ref"),
        )

    @classmethod
    def from_json(cls, s: str) -> FlowPath:
        return cls.from_dict(json.loads(s))

    @classmethod
    def json_schema(cls) -> dict[str, Any]:
        return _host_schema("FlowPath") or {
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "store_ref": {"type": "string"},
                "cache_store_ref": {"type": ["string", "null"]},
            },
            "required": ["path", "store_ref"],
        }


# ── FlowImage — handle to an in-memory image ───────────────────────────


@dataclass
class FlowImage:
    """Reference to a host-side in-memory image.

    The actual pixel data lives in the host's cache; this is just a handle.

    Serialisation format::

        {"image_ref": "img_abc123"}
    """

    image_ref: str

    def to_bytes(self, ctx: Context, fmt: str = "png") -> bytes | None:
        """Serialise the image to bytes in the given format (png, jpeg, …)."""
        return ctx.image_to_bytes(self, fmt)

    @classmethod
    def from_bytes(cls, ctx: Context, data: bytes, fmt: str = "png") -> FlowImage | None:
        """Create a host-side image from raw bytes."""
        return ctx.image_from_bytes(data, fmt)

    def to_dict(self) -> dict[str, str]:
        return {"image_ref": self.image_ref}

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> FlowImage:
        return cls(image_ref=data["image_ref"])

    @classmethod
    def from_json(cls, s: str) -> FlowImage:
        return cls.from_dict(json.loads(s))

    @classmethod
    def json_schema(cls) -> dict[str, Any]:
        return {
            "type": "object",
            "properties": {"image_ref": {"type": "string"}},
            "required": ["image_ref"],
        }


# ── Bit — LLM/VLM model descriptor ─────────────────────────────────────


@dataclass
class Bit:
    """Handle to an LLM or VLM model registered on the host.

    Nodes receive this as a Struct pin; call :meth:`prompt` to run inference.

    Serialisation format::

        {"id": "gpt-4o", "type": "llm", "hub": "openai", ...}
    """

    id: str
    bit_type: str = ""
    hub: str = ""
    hash: str = ""
    parameters: Any = field(default_factory=dict)
    file_name: str | None = None
    version: str | None = None
    license: str | None = None
    extra: dict[str, Any] = field(default_factory=dict)

    def prompt(self, ctx: Context, messages: list[ChatMessage], **kwargs) -> str | None:
        """Send a completion request and return the model's response text.

        Optional kwargs: temperature, max_tokens, tool_choice, output_schema, tools
        """
        return ctx.llm_prompt(self, messages, stream=False, **kwargs)

    def prompt_stream(self, ctx: Context, messages: list[ChatMessage], **kwargs) -> str | None:
        """Stream a completion — chunks arrive via the streaming interface.

        Uses llm_prompt_stream (ABI v2) for true host-side streaming.
        Optional kwargs: temperature, max_tokens, tool_choice, output_schema, tools
        """
        return ctx.llm_prompt_stream(self, messages, **kwargs)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self.id, "type": self.bit_type, "hub": self.hub}
        if self.hash:
            d["hash"] = self.hash
        if self.parameters:
            d["parameters"] = self.parameters
        if self.file_name is not None:
            d["file_name"] = self.file_name
        if self.version is not None:
            d["version"] = self.version
        if self.license is not None:
            d["license"] = self.license
        if self.extra:
            d.update(self.extra)
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> Bit:
        known = {"id", "type", "hub", "hash", "parameters", "file_name", "version", "license"}
        extra = {k: v for k, v in data.items() if k not in known}
        return cls(
            id=data.get("id", ""),
            bit_type=data.get("type", ""),
            hub=data.get("hub", ""),
            hash=data.get("hash", ""),
            parameters=data.get("parameters", {}),
            file_name=data.get("file_name"),
            version=data.get("version"),
            license=data.get("license"),
            extra=extra,
        )

    @classmethod
    def from_json(cls, s: str) -> Bit:
        return cls.from_dict(json.loads(s))

    @classmethod
    def json_schema(cls) -> dict[str, Any]:
        return _host_schema("Bit") or {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "type": {"type": "string"},
                "hub": {"type": "string"},
                "hash": {"type": "string"},
                "parameters": {"type": "object"},
                "file_name": {"type": ["string", "null"]},
                "version": {"type": ["string", "null"]},
                "license": {"type": ["string", "null"]},
            },
            "required": ["id"],
        }


# ── CachedEmbeddingModel — handle to an embedding model ────────────────


@dataclass
class CachedEmbeddingModel:
    """Handle to a cached embedding model on the host."""

    cache_key: str
    model_type: str = ""

    def embed_query(self, ctx: Context, texts: list[str]) -> list[list[float]] | None:
        """Embed texts optimised for retrieval queries."""
        return ctx.embed_text_query(self, texts)

    def embed_document(self, ctx: Context, texts: list[str]) -> list[list[float]] | None:
        """Embed texts optimised for document indexing."""
        return ctx.embed_text_document(self, texts)

    def embed_image(self, ctx: Context, image: FlowImage) -> list[float] | None:
        """Embed an image."""
        return ctx.embed_image(self, image)

    def to_dict(self) -> dict[str, str]:
        return {"cache_key": self.cache_key, "model_type": self.model_type}

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> CachedEmbeddingModel:
        return cls(cache_key=data["cache_key"], model_type=data.get("model_type", ""))

    @classmethod
    def from_json(cls, s: str) -> CachedEmbeddingModel:
        return cls.from_dict(json.loads(s))

    @classmethod
    def json_schema(cls) -> dict[str, Any]:
        return _host_schema("CachedEmbeddingModel") or {
            "type": "object",
            "properties": {
                "cache_key": {"type": "string"},
                "model_type": {"type": "string"},
            },
            "required": ["cache_key"],
        }


# ── ChatMessage & content parts ─────────────────────────────────────────


@dataclass
class ImageData:
    url: str
    media_type: str | None = None
    detail: str | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"url": self.url}
        if self.media_type is not None:
            d["media_type"] = self.media_type
        if self.detail is not None:
            d["detail"] = self.detail
        return d


@dataclass
class AudioData:
    url: str
    media_type: str | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"url": self.url}
        if self.media_type is not None:
            d["media_type"] = self.media_type
        return d


@dataclass
class VideoData:
    url: str
    media_type: str | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"url": self.url}
        if self.media_type is not None:
            d["media_type"] = self.media_type
        return d


@dataclass
class DocumentData:
    url: str
    media_type: str | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"url": self.url}
        if self.media_type is not None:
            d["media_type"] = self.media_type
        return d


@dataclass
class ToolCallData:
    id: str
    name: str
    arguments: Any = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"id": self.id, "name": self.name, "arguments": self.arguments}


@dataclass
class ToolResultData:
    id: str
    content: str

    def to_dict(self) -> dict[str, Any]:
        return {"id": self.id, "content": self.content}


@dataclass
class ReasoningData:
    text: list[str]
    id: str | None = None
    signature: str | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"text": self.text}
        if self.id is not None:
            d["id"] = self.id
        if self.signature is not None:
            d["signature"] = self.signature
        return d


@dataclass
class ContentPart:
    """One part of a multimodal message.

    Use the static factory methods to build parts::

        ContentPart.text("Hello")
        ContentPart.image_url("https://…")
    """

    type: str
    data: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"type": self.type}
        d.update(self.data)
        return d

    # ── Factories ────────────────────────────────────────────────────

    @classmethod
    def text(cls, text: str) -> ContentPart:
        return cls(type="text", data={"text": text})

    @classmethod
    def image_url(cls, url: str, detail: str | None = None) -> ContentPart:
        d: dict[str, Any] = {"image": {"url": url}}
        if detail is not None:
            d["image"]["detail"] = detail
        return cls(type="image", data=d)

    @classmethod
    def image(cls, url: str, media_type: str, detail: str | None = None) -> ContentPart:
        d: dict[str, Any] = {"image": {"url": url, "media_type": media_type}}
        if detail is not None:
            d["image"]["detail"] = detail
        return cls(type="image", data=d)

    @classmethod
    def audio_url(cls, url: str) -> ContentPart:
        return cls(type="audio", data={"audio": {"url": url}})

    @classmethod
    def audio(cls, url: str, media_type: str) -> ContentPart:
        return cls(type="audio", data={"audio": {"url": url, "media_type": media_type}})

    @classmethod
    def video_url(cls, url: str) -> ContentPart:
        return cls(type="video", data={"video": {"url": url}})

    @classmethod
    def video(cls, url: str, media_type: str) -> ContentPart:
        return cls(type="video", data={"video": {"url": url, "media_type": media_type}})

    @classmethod
    def document_url(cls, url: str) -> ContentPart:
        return cls(type="document", data={"document": {"url": url}})

    @classmethod
    def document(cls, url: str, media_type: str) -> ContentPart:
        return cls(type="document", data={"document": {"url": url, "media_type": media_type}})

    @classmethod
    def tool_call(cls, call_id: str, name: str, arguments: Any = None) -> ContentPart:
        return cls(type="tool_call", data={"tool_call": {"id": call_id, "name": name, "arguments": arguments or {}}})

    @classmethod
    def tool_result(cls, call_id: str, content: str) -> ContentPart:
        return cls(type="tool_result", data={"tool_result": {"id": call_id, "content": content}})

    @classmethod
    def reasoning(cls, text: list[str]) -> ContentPart:
        return cls(type="reasoning", data={"reasoning": {"text": text}})


@dataclass
class ChatMessage:
    """A single message in a chat conversation.

    Supports both simple text and multimodal (parts-based) content.

    Usage::

        msg = ChatMessage.user("What is 2+2?")
        msg = ChatMessage.user_multimodal([
            ContentPart.text("Describe this image"),
            ContentPart.image_url("https://example.com/img.png"),
        ])
        msg = ChatMessage.system("You are a helpful assistant.")

        response = bit.prompt(ctx, [system_msg, user_msg])
    """

    role: str
    content: str | list[dict[str, Any]] = ""
    tool_calls: list[dict[str, Any]] | None = None
    tool_call_id: str | None = None

    # ── Factories ────────────────────────────────────────────────────

    @classmethod
    def system(cls, content: str) -> ChatMessage:
        return cls(role="system", content=content)

    @classmethod
    def user(cls, content: str) -> ChatMessage:
        return cls(role="user", content=content)

    @classmethod
    def user_multimodal(cls, parts: list[ContentPart]) -> ChatMessage:
        return cls(role="user", content=[p.to_dict() for p in parts])

    @classmethod
    def assistant(cls, content: str) -> ChatMessage:
        return cls(role="assistant", content=content)

    @classmethod
    def assistant_with_tool_calls(cls, content: str, tool_calls: list[ToolCallData]) -> ChatMessage:
        return cls(
            role="assistant",
            content=content,
            tool_calls=[tc.to_dict() for tc in tool_calls] if tool_calls else None,
        )

    @classmethod
    def tool_result(cls, tool_call_id: str, content: str) -> ChatMessage:
        return cls(role="tool", content=content, tool_call_id=tool_call_id)

    # ── Helpers ──────────────────────────────────────────────────────

    def text_content(self) -> str:
        """Extract plain text from the message content."""
        if isinstance(self.content, str):
            return self.content
        return "\n".join(
            p["text"] for p in self.content if isinstance(p, dict) and p.get("type") == "text"
        )

    # ── Serialisation ────────────────────────────────────────────────

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"role": self.role, "content": self.content}
        if self.tool_calls is not None:
            d["tool_calls"] = self.tool_calls
        if self.tool_call_id is not None:
            d["tool_call_id"] = self.tool_call_id
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> ChatMessage:
        return cls(
            role=data["role"],
            content=data.get("content", ""),
            tool_calls=data.get("tool_calls"),
            tool_call_id=data.get("tool_call_id"),
        )

    @classmethod
    def from_json(cls, s: str) -> ChatMessage:
        return cls.from_dict(json.loads(s))


# ── NodeDBConnection — handle to a vector database ──────────────────────


@dataclass
class VectorSearchQuery:
    vector: list[float]
    limit: int = 10
    offset: int = 0
    filter: str | None = None
    select: list[str] | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"vector": self.vector, "limit": self.limit, "offset": self.offset}
        if self.filter is not None:
            d["filter"] = self.filter
        if self.select is not None:
            d["select"] = self.select
        return d


@dataclass
class FtsSearchQuery:
    text: str
    limit: int = 10
    offset: int = 0
    filter: str | None = None
    select: list[str] | None = None
    fields: list[str] | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"text": self.text, "limit": self.limit, "offset": self.offset}
        if self.filter is not None:
            d["filter"] = self.filter
        if self.select is not None:
            d["select"] = self.select
        if self.fields is not None:
            d["fields"] = self.fields
        return d


@dataclass
class HybridSearchQuery:
    vector: list[float]
    text: str
    limit: int = 10
    offset: int = 0
    rerank: bool = False
    filter: str | None = None
    select: list[str] | None = None
    fields: list[str] | None = None

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "vector": self.vector, "text": self.text,
            "limit": self.limit, "offset": self.offset, "rerank": self.rerank,
        }
        if self.filter is not None:
            d["filter"] = self.filter
        if self.select is not None:
            d["select"] = self.select
        if self.fields is not None:
            d["fields"] = self.fields
        return d


@dataclass
class NodeDBConnection:
    """Handle to a host-side vector database connection."""

    cache_key: str

    def vector_search(self, ctx: Context, query: VectorSearchQuery) -> list[Any] | None:
        return ctx.db_query(0, self, query.to_dict())

    def fts_search(self, ctx: Context, query: FtsSearchQuery) -> list[Any] | None:
        return ctx.db_query(1, self, query.to_dict())

    def hybrid_search(self, ctx: Context, query: HybridSearchQuery) -> list[Any] | None:
        return ctx.db_query(2, self, query.to_dict())

    def insert(self, ctx: Context, items: list[Any]) -> bool:
        result = ctx.db_query(3, self, {"items": items})
        return result is not None

    def upsert(self, ctx: Context, items: list[Any], id_field: str = "id") -> bool:
        result = ctx.db_query(4, self, {"items": items, "id_field": id_field})
        return result is not None

    def delete(self, ctx: Context, filter_expr: str) -> bool:
        result = ctx.db_query(5, self, {"filter": filter_expr})
        return result is not None

    def list_rows(self, ctx: Context, select: list[str] | None = None, limit: int = 100, offset: int = 0) -> list[Any] | None:
        return ctx.db_query(6, self, {"select": select, "limit": limit, "offset": offset})

    def count(self, ctx: Context, filter_expr: str | None = None) -> int | None:
        result = ctx.db_query(7, self, {"filter": filter_expr})
        return result if isinstance(result, int) else None

    def to_dict(self) -> dict[str, str]:
        return {"cache_key": self.cache_key}

    def to_json(self) -> str:
        return json.dumps(self.to_dict())

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> NodeDBConnection:
        return cls(cache_key=data["cache_key"])

    @classmethod
    def from_json(cls, s: str) -> NodeDBConnection:
        return cls.from_dict(json.loads(s))

    @classmethod
    def json_schema(cls) -> dict[str, Any]:
        return _host_schema("NodeDBConnection") or {
            "type": "object",
            "properties": {"cache_key": {"type": "string"}},
            "required": ["cache_key"],
        }
