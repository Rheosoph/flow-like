"""Tests for interop domain types: FlowPath, FlowImage, Bit, ChatMessage, etc."""
import json
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from flow_like_wasm_sdk.interop import (
    FlowPath, FlowImage, Bit, ChatMessage, ContentPart,
    CachedEmbeddingModel, NodeDBConnection,
    VectorSearchQuery, FtsSearchQuery, HybridSearchQuery,
    ToolCallData,
)
from flow_like_wasm_sdk.host import MockHostBridge, set_host
from flow_like_wasm_sdk.context import Context
from flow_like_wasm_sdk.types import (
    ExecutionInput, PinType,
    Input, Output, WasmNode, run_node,
    _NODE_REGISTRY,
)

import pytest


def make_context(inputs=None, stream=False, host=None):
    host = host or MockHostBridge()
    set_host(host)
    ei = ExecutionInput(
        node_id="test_node", node_name="test", run_id="run_1",
        inputs=inputs or {}, stream_state=stream,
        app_id="app_1", board_id="board_1", user_id="user_1",
    )
    return Context(ei, host), host


# ═════════════════════════════════════════════════════════════════════════
# FlowPath
# ═════════════════════════════════════════════════════════════════════════

class TestFlowPath:
    def test_new(self):
        fp = FlowPath("a/b.txt", "store1")
        assert fp.path == "a/b.txt"
        assert fp.store_ref == "store1"
        assert fp.cache_store_ref is None

    def test_new_with_cache(self):
        fp = FlowPath("a/b.txt", "store1", "cache1")
        assert fp.cache_store_ref == "cache1"

    def test_serde_roundtrip(self):
        fp = FlowPath("dir/file.bin", "s3", "redis")
        d = fp.to_dict()
        fp2 = FlowPath.from_dict(d)
        assert fp.path == fp2.path
        assert fp.store_ref == fp2.store_ref
        assert fp.cache_store_ref == fp2.cache_store_ref

    def test_json_roundtrip(self):
        fp = FlowPath("x/y", "local")
        s = fp.to_json()
        fp2 = FlowPath.from_json(s)
        assert fp.path == fp2.path

    def test_omits_none_cache(self):
        fp = FlowPath("x", "y")
        d = fp.to_dict()
        assert "cache_store_ref" not in d

    def test_from_dict_without_cache(self):
        fp = FlowPath.from_dict({"path": "a", "store_ref": "b"})
        assert fp.cache_store_ref is None

    def test_child(self):
        fp = FlowPath("data", "s3")
        child = fp.child("file.txt")
        assert child.path == "data/file.txt"
        assert child.store_ref == "s3"

    def test_child_nested(self):
        fp = FlowPath("a", "s")
        nested = fp.child("b").child("c.txt")
        assert nested.path == "a/b/c.txt"

    def test_child_trailing_slash(self):
        fp = FlowPath("data/", "s3")
        child = fp.child("file.txt")
        assert child.path == "data/file.txt"

    def test_parent(self):
        fp = FlowPath("a/b/c.txt", "s")
        p = fp.parent()
        assert p is not None
        assert p.path == "a/b"

    def test_parent_root(self):
        fp = FlowPath("file.txt", "s")
        assert fp.parent() is None

    def test_file_name(self):
        fp = FlowPath("a/b/doc.pdf", "s")
        assert fp.file_name() == "doc.pdf"

    def test_file_name_no_dir(self):
        fp = FlowPath("readme.md", "s")
        assert fp.file_name() == "readme.md"

    def test_file_name_empty(self):
        fp = FlowPath("", "s")
        assert fp.file_name() is None

    def test_extension(self):
        fp = FlowPath("a/b.tar.gz", "s")
        assert fp.extension() == "gz"

    def test_extension_none(self):
        fp = FlowPath("a/b", "s")
        assert fp.extension() is None

    def test_with_extension(self):
        fp = FlowPath("a/b.txt", "s")
        fp2 = fp.with_extension("json")
        assert fp2.path == "a/b.json"

    def test_join(self):
        fp = FlowPath("root", "s")
        joined = fp.join("a", "b", "c.txt")
        assert joined.path == "root/a/b/c.txt"

    def test_json_schema(self):
        schema = FlowPath.json_schema()
        assert schema["type"] == "object"
        assert "path" in schema["properties"]
        assert "store_ref" in schema["properties"]

    def test_io_read_write(self):
        ctx, host = make_context()
        fp = FlowPath("test.txt", "mock_store")
        assert fp.put(ctx, b"hello world")
        data = fp.get(ctx)
        assert data == b"hello world"

    def test_io_string(self):
        ctx, host = make_context()
        fp = FlowPath("greeting.txt", "mock_store")
        assert fp.put_string(ctx, "hello")
        assert fp.get_string(ctx) == "hello"

    def test_io_json(self):
        ctx, host = make_context()
        fp = FlowPath("data.json", "mock_store")
        assert fp.put_json(ctx, {"key": "value"})
        assert fp.get_json(ctx) == {"key": "value"}

    def test_io_exists(self):
        ctx, host = make_context()
        fp = FlowPath("exists.txt", "mock_store")
        assert not fp.exists(ctx)
        fp.put(ctx, b"data")
        assert fp.exists(ctx)

    def test_io_list(self):
        ctx, host = make_context()
        host.storage["dir/a.txt"] = b"a"
        host.storage["dir/b.txt"] = b"b"
        fp = FlowPath("dir", "mock_store")
        items = fp.list(ctx)
        assert items is not None
        assert len(items) == 2

    def test_read_write_aliases(self):
        ctx, host = make_context()
        fp = FlowPath("alias.txt", "mock_store")
        fp.write(ctx, b"data")
        assert fp.read(ctx) == b"data"


# ═════════════════════════════════════════════════════════════════════════
# FlowImage
# ═════════════════════════════════════════════════════════════════════════

class TestFlowImage:
    def test_serde_roundtrip(self):
        img = FlowImage("img_abc123")
        d = img.to_dict()
        img2 = FlowImage.from_dict(d)
        assert img.image_ref == img2.image_ref

    def test_json_roundtrip(self):
        img = FlowImage("img_xyz")
        s = img.to_json()
        img2 = FlowImage.from_json(s)
        assert img.image_ref == img2.image_ref

    def test_json_schema(self):
        schema = FlowImage.json_schema()
        assert schema["type"] == "object"
        assert "image_ref" in schema["properties"]

    def test_from_bytes(self):
        ctx, host = make_context()
        img = FlowImage.from_bytes(ctx, b"\x89PNG...", "png")
        assert img is not None
        assert img.image_ref.startswith("mock_img_")

    def test_to_bytes(self):
        ctx, host = make_context()
        img = FlowImage.from_bytes(ctx, b"\x89PNG_DATA", "png")
        result = img.to_bytes(ctx, "png")
        assert result == b"\x89PNG_DATA"


# ═════════════════════════════════════════════════════════════════════════
# Bit (LLM model handle)
# ═════════════════════════════════════════════════════════════════════════

class TestBit:
    def test_serde_roundtrip(self):
        bit = Bit(id="gpt-4o", bit_type="llm", hub="openai")
        d = bit.to_dict()
        bit2 = Bit.from_dict(d)
        assert bit.id == bit2.id
        assert bit.bit_type == bit2.bit_type
        assert bit.hub == bit2.hub

    def test_json_roundtrip(self):
        bit = Bit(id="claude-3", bit_type="llm", hub="anthropic", version="3.5")
        s = bit.to_json()
        bit2 = Bit.from_json(s)
        assert bit.id == bit2.id
        assert bit.version == bit2.version

    def test_extra_fields(self):
        data = {"id": "test", "type": "llm", "hub": "x", "custom_field": "value"}
        bit = Bit.from_dict(data)
        assert bit.extra == {"custom_field": "value"}
        d = bit.to_dict()
        assert d["custom_field"] == "value"

    def test_json_schema(self):
        schema = Bit.json_schema()
        assert "id" in schema["properties"]

    def test_prompt(self):
        ctx, host = make_context()
        bit = Bit(id="gpt-4o", bit_type="llm", hub="openai")
        messages = [ChatMessage.user("Hello")]
        result = bit.prompt(ctx, messages)
        assert result is not None

    def test_prompt_stream(self):
        ctx, host = make_context()
        bit = Bit(id="gpt-4o", bit_type="llm", hub="openai")
        messages = [ChatMessage.user("Hello")]
        result = bit.prompt_stream(ctx, messages)
        assert result is not None


# ═════════════════════════════════════════════════════════════════════════
# ChatMessage
# ═════════════════════════════════════════════════════════════════════════

class TestChatMessage:
    def test_system(self):
        msg = ChatMessage.system("You are helpful")
        assert msg.role == "system"
        assert msg.content == "You are helpful"

    def test_user(self):
        msg = ChatMessage.user("What is 2+2?")
        assert msg.role == "user"
        assert msg.text_content() == "What is 2+2?"

    def test_assistant(self):
        msg = ChatMessage.assistant("The answer is 4")
        assert msg.role == "assistant"

    def test_tool_result(self):
        msg = ChatMessage.tool_result("call_123", '{"result": 42}')
        assert msg.role == "tool"
        assert msg.tool_call_id == "call_123"

    def test_assistant_with_tool_calls(self):
        calls = [ToolCallData(id="1", name="get_weather", arguments={"city": "NYC"})]
        msg = ChatMessage.assistant_with_tool_calls("Let me check", calls)
        assert msg.tool_calls is not None
        assert len(msg.tool_calls) == 1
        assert msg.tool_calls[0]["name"] == "get_weather"

    def test_user_multimodal(self):
        parts = [
            ContentPart.text("What is this?"),
            ContentPart.image_url("https://example.com/img.png"),
        ]
        msg = ChatMessage.user_multimodal(parts)
        assert msg.role == "user"
        assert isinstance(msg.content, list)
        assert len(msg.content) == 2
        assert msg.text_content() == "What is this?"

    def test_serde_roundtrip(self):
        msg = ChatMessage.user("Hello")
        d = msg.to_dict()
        msg2 = ChatMessage.from_dict(d)
        assert msg.role == msg2.role
        assert msg.content == msg2.content

    def test_json_roundtrip(self):
        msg = ChatMessage.system("Be concise")
        s = msg.to_json()
        msg2 = ChatMessage.from_json(s)
        assert msg.content == msg2.content


# ═════════════════════════════════════════════════════════════════════════
# ContentPart
# ═════════════════════════════════════════════════════════════════════════

class TestContentPart:
    def test_text(self):
        p = ContentPart.text("hello")
        d = p.to_dict()
        assert d["type"] == "text"
        assert d["text"] == "hello"

    def test_image_url(self):
        p = ContentPart.image_url("https://x.com/img.png", detail="high")
        d = p.to_dict()
        assert d["type"] == "image"
        assert d["image"]["url"] == "https://x.com/img.png"
        assert d["image"]["detail"] == "high"

    def test_image_with_media_type(self):
        p = ContentPart.image("data:image/png;base64,...", "image/png")
        d = p.to_dict()
        assert d["image"]["media_type"] == "image/png"

    def test_audio(self):
        p = ContentPart.audio("https://x.com/audio.mp3", "audio/mpeg")
        d = p.to_dict()
        assert d["type"] == "audio"

    def test_video(self):
        p = ContentPart.video_url("https://x.com/video.mp4")
        d = p.to_dict()
        assert d["type"] == "video"

    def test_document(self):
        p = ContentPart.document("https://x.com/doc.pdf", "application/pdf")
        d = p.to_dict()
        assert d["type"] == "document"

    def test_tool_call(self):
        p = ContentPart.tool_call("c1", "fn_name", {"x": 1})
        d = p.to_dict()
        assert d["type"] == "tool_call"
        assert d["tool_call"]["name"] == "fn_name"

    def test_tool_result(self):
        p = ContentPart.tool_result("c1", "the result")
        d = p.to_dict()
        assert d["type"] == "tool_result"
        assert d["tool_result"]["content"] == "the result"

    def test_reasoning(self):
        p = ContentPart.reasoning(["step 1", "step 2"])
        d = p.to_dict()
        assert d["type"] == "reasoning"
        assert len(d["reasoning"]["text"]) == 2


# ═════════════════════════════════════════════════════════════════════════
# CachedEmbeddingModel
# ═════════════════════════════════════════════════════════════════════════

class TestCachedEmbeddingModel:
    def test_serde(self):
        m = CachedEmbeddingModel("key_abc", "text-embedding-3-small")
        d = m.to_dict()
        m2 = CachedEmbeddingModel.from_dict(d)
        assert m.cache_key == m2.cache_key
        assert m.model_type == m2.model_type

    def test_embed_query(self):
        ctx, host = make_context()
        m = CachedEmbeddingModel("key1", "embed-v1")
        result = m.embed_query(ctx, ["hello", "world"])
        assert result is not None
        assert len(result) == 2

    def test_embed_document(self):
        ctx, host = make_context()
        m = CachedEmbeddingModel("key1", "embed-v1")
        result = m.embed_document(ctx, ["doc text"])
        assert result is not None
        assert len(result) == 1


# ═════════════════════════════════════════════════════════════════════════
# NodeDBConnection
# ═════════════════════════════════════════════════════════════════════════

class TestNodeDBConnection:
    def test_serde(self):
        db = NodeDBConnection("db_cache_key")
        d = db.to_dict()
        db2 = NodeDBConnection.from_dict(d)
        assert db.cache_key == db2.cache_key

    def test_json_schema(self):
        schema = NodeDBConnection.json_schema()
        assert "cache_key" in schema["properties"]


# ═════════════════════════════════════════════════════════════════════════
# Query types
# ═════════════════════════════════════════════════════════════════════════

class TestQueryTypes:
    def test_vector_search_query(self):
        q = VectorSearchQuery(vector=[0.1, 0.2], limit=5)
        d = q.to_dict()
        assert d["vector"] == [0.1, 0.2]
        assert d["limit"] == 5

    def test_fts_search_query(self):
        q = FtsSearchQuery(text="hello world", limit=10, fields=["title", "body"])
        d = q.to_dict()
        assert d["text"] == "hello world"
        assert d["fields"] == ["title", "body"]

    def test_hybrid_search_query(self):
        q = HybridSearchQuery(vector=[0.1], text="hello", rerank=True)
        d = q.to_dict()
        assert d["rerank"] is True


# ═════════════════════════════════════════════════════════════════════════
# Integration: interop types as WasmNode pin annotations
# ═════════════════════════════════════════════════════════════════════════

class TestInteropPinIntegration:
    """Test that interop types work as declarative pin type annotations."""

    @pytest.fixture(autouse=True)
    def _clear_registry(self):
        saved = _NODE_REGISTRY[:]
        _NODE_REGISTRY.clear()
        yield
        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.extend(saved)

    def test_flow_path_pin(self):
        class PathNode(WasmNode, name="path_node", category="Test"):
            """Reads a file"""
            source: FlowPath = Input()
            content: str = Output()

            def run(self, ctx):
                return ctx.success()

        nd = PathNode().get_node()
        source_pin = next(p for p in nd.pins if p.name == "source")
        assert source_pin.data_type == PinType.STRUCT
        assert source_pin.schema is not None
        schema = json.loads(source_pin.schema)
        assert "path" in schema["properties"]

    def test_flow_image_pin(self):
        class ImageNode(WasmNode, name="image_node", category="Test"):
            """Processes an image"""
            image_in: FlowImage = Input()
            image_out: FlowImage = Output()

            def run(self, ctx):
                return ctx.success()

        nd = ImageNode().get_node()
        in_pin = next(p for p in nd.pins if p.name == "image_in")
        out_pin = next(p for p in nd.pins if p.name == "image_out")
        assert in_pin.data_type == PinType.STRUCT
        assert out_pin.data_type == PinType.STRUCT

    def test_bit_pin(self):
        class LLMNode(WasmNode, name="llm_node", category="AI"):
            """Runs LLM inference"""
            model: Bit = Input()
            prompt_text: str = Input(default="")
            response: str = Output()

            def run(self, ctx):
                return ctx.success()

        nd = LLMNode().get_node()
        model_pin = next(p for p in nd.pins if p.name == "model")
        assert model_pin.data_type == PinType.STRUCT
        schema = json.loads(model_pin.schema)
        assert "id" in schema["properties"]

    def test_typed_context_deserializes_flow_path(self):
        class PathReader(WasmNode, name="path_reader", category="Test"):
            """Reads a file from FlowPath"""
            source: FlowPath = Input()
            content: str = Output()

            def run(self, ctx):
                assert isinstance(ctx.source, FlowPath)
                assert ctx.source.path == "test/data.txt"
                assert ctx.source.store_ref == "local"
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(PathReader)

        host = MockHostBridge()
        set_host(host)
        ctx, _ = make_context(
            inputs={"source": {"path": "test/data.txt", "store_ref": "local"}},
            host=host,
        )
        result = run_node("path_reader", ctx)
        assert result.error is None

    def test_typed_context_serializes_flow_path(self):
        class PathWriter(WasmNode, name="path_writer", category="Test"):
            """Outputs a FlowPath"""
            result_path: FlowPath = Output()

            def run(self, ctx):
                ctx.result_path = FlowPath("output/result.json", "s3")
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(PathWriter)

        host = MockHostBridge()
        set_host(host)
        ctx, _ = make_context(host=host)
        result = run_node("path_writer", ctx)
        assert result.error is None
        assert result.outputs["result_path"]["path"] == "output/result.json"

    def test_typed_context_deserializes_bit(self):
        class LLMRunner(WasmNode, name="llm_runner", category="AI"):
            """Runs LLM"""
            model: Bit = Input()
            output_text: str = Output()

            def run(self, ctx):
                assert isinstance(ctx.model, Bit)
                assert ctx.model.id == "gpt-4o"
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(LLMRunner)

        host = MockHostBridge()
        set_host(host)
        ctx, _ = make_context(
            inputs={"model": {"id": "gpt-4o", "type": "llm", "hub": "openai"}},
            host=host,
        )
        result = run_node("llm_runner", ctx)
        assert result.error is None

    def test_list_of_flow_paths(self):
        class MultiPathNode(WasmNode, name="multi_path", category="Test"):
            """Processes multiple paths"""
            paths: list[FlowPath] = Input(default_factory=list)
            count: int = Output()

            def run(self, ctx):
                assert isinstance(ctx.paths, list)
                assert all(isinstance(p, FlowPath) for p in ctx.paths)
                ctx.count = len(ctx.paths)
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(MultiPathNode)

        host = MockHostBridge()
        set_host(host)
        ctx, _ = make_context(
            inputs={"paths": [
                {"path": "a.txt", "store_ref": "s"},
                {"path": "b.txt", "store_ref": "s"},
            ]},
            host=host,
        )
        result = run_node("multi_path", ctx)
        assert result.error is None
        assert result.outputs["count"] == 2

    def test_full_llm_node_example(self):
        """Integration test: a full LLM node that reads a Bit, builds messages, and gets a response."""
        class ChatNode(WasmNode, name="chat_node", category="AI/Chat"):
            """Sends a chat message to an LLM."""
            model: Bit = Input()
            system_prompt: str = Input(default="You are helpful")
            user_message: str = Input(default="")
            response: str = Output()

            def run(self, ctx):
                messages = [
                    ChatMessage.system(ctx.system_prompt),
                    ChatMessage.user(ctx.user_message),
                ]
                result = ctx.model.prompt(ctx, messages)
                ctx.response = result or ""
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(ChatNode)

        host = MockHostBridge()
        set_host(host)
        ctx, _ = make_context(
            inputs={
                "model": {"id": "gpt-4o", "type": "llm", "hub": "openai"},
                "system_prompt": "Be brief",
                "user_message": "What is 2+2?",
            },
            host=host,
        )
        result = run_node("chat_node", ctx)
        assert result.error is None
        assert result.outputs["response"] != ""

    def test_file_processor_node_example(self):
        """Integration test: a node that reads a file via FlowPath and outputs its length."""
        class FileLen(WasmNode, name="file_len", category="IO"):
            """Gets the byte length of a file."""
            path: FlowPath = Input()
            length: int = Output()

            def run(self, ctx):
                data = ctx.path.get(ctx)
                ctx.length = len(data) if data is not None else 0
                return ctx.success()

        _NODE_REGISTRY.clear()
        _NODE_REGISTRY.append(FileLen)

        host = MockHostBridge()
        host.storage["my/file.txt"] = b"hello world"
        set_host(host)
        ctx, _ = make_context(
            inputs={"path": {"path": "my/file.txt", "store_ref": "mock_store"}},
            host=host,
        )
        result = run_node("file_len", ctx)
        assert result.error is None
        assert result.outputs["length"] == 11
