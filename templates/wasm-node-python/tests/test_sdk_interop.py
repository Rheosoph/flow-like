"""Smoke tests for interop types in the standalone sdk.py."""
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from sdk import (
    FlowPath, FlowImage, Bit, ChatMessage, ContentPart,
    CachedEmbeddingModel, NodeDBConnection, VectorSearchQuery,
    ImageData,
    ToolCallData, ToolResultData,
    MockHostBridge, Context, ExecutionInput, PinType, WasmNode,
    ExecutionResult, Input, Output, _is_interop_type, _scalar_data_type,
)


class TestFlowPathSdk:
    def test_serde(self):
        fp = FlowPath("uploads/doc.pdf", "local")
        assert fp.to_dict() == {"path": "uploads/doc.pdf", "store_ref": "local"}
        assert FlowPath.from_dict(fp.to_dict()) == fp

    def test_path_ops(self):
        fp = FlowPath("uploads/doc.pdf", "s3")
        assert fp.child("sub").path == "uploads/doc.pdf/sub"
        assert fp.parent().path == "uploads"
        assert fp.file_name() == "doc.pdf"
        assert fp.extension() == "pdf"
        assert fp.with_extension("txt").path == "uploads/doc.txt"

    def test_io_with_mock(self):
        host = MockHostBridge()
        host.storage["test/file.txt"] = b"hello world"
        ctx = Context(ExecutionInput(inputs={}), host)
        fp = FlowPath("test/file.txt", "mock")
        assert fp.get(ctx) == b"hello world"
        assert fp.get_string(ctx) == "hello world"
        fp2 = FlowPath("test/new.txt", "mock")
        assert fp2.put_string(ctx, "written")
        assert fp2.get_string(ctx) == "written"


class TestBitSdk:
    def test_serde(self):
        bit = Bit(id="gpt-4o", bit_type="llm", hub="openai")
        d = bit.to_dict()
        assert d["id"] == "gpt-4o"
        assert Bit.from_dict(d).id == "gpt-4o"

    def test_prompt(self):
        host = MockHostBridge()
        ctx = Context(ExecutionInput(inputs={}), host)
        bit = Bit(id="test", bit_type="llm")
        result = bit.prompt(ctx, [ChatMessage.user("Hi")])
        assert "Mock LLM" in result


class TestChatMessageSdk:
    def test_factories(self):
        assert ChatMessage.system("sys").role == "system"
        assert ChatMessage.user("hi").role == "user"
        assert ChatMessage.assistant("ok").role == "assistant"

    def test_multimodal(self):
        msg = ChatMessage.user_multimodal([
            ContentPart.text("Describe"),
            ContentPart.image_url("http://example.com/img.png"),
        ])
        assert isinstance(msg.content, list)
        assert msg.content[0]["type"] == "text"
        assert msg.content[1]["type"] == "image"

    def test_text_content(self):
        msg = ChatMessage.user("Hello")
        assert msg.text_content() == "Hello"

    def test_serde(self):
        msg = ChatMessage.user("test")
        d = msg.to_dict()
        restored = ChatMessage.from_dict(d)
        assert restored.role == "user"
        assert restored.content == "test"


class TestFlowImageSdk:
    def test_roundtrip(self):
        host = MockHostBridge()
        ctx = Context(ExecutionInput(inputs={}), host)
        img = FlowImage.from_bytes(ctx, b"fake_png", "png")
        assert img is not None
        assert img.image_ref.startswith("mock_img_")
        data = img.to_bytes(ctx)
        assert data == b"fake_png"


class TestEmbeddingSdk:
    def test_embed_query(self):
        host = MockHostBridge()
        ctx = Context(ExecutionInput(inputs={}), host)
        model = CachedEmbeddingModel(cache_key="test")
        result = model.embed_query(ctx, ["hello", "world"])
        assert len(result) == 2
        assert len(result[0]) == 3


class TestDbSdk:
    def test_vector_search(self):
        host = MockHostBridge()
        ctx = Context(ExecutionInput(inputs={}), host)
        db = NodeDBConnection(cache_key="test_db")
        results = db.vector_search(ctx, VectorSearchQuery(vector=[0.1, 0.2], limit=5))
        assert results == []


class TestInteropTypeSystem:
    def test_is_interop_type(self):
        assert _is_interop_type(FlowPath) is True
        assert _is_interop_type(Bit) is True
        assert _is_interop_type(CachedEmbeddingModel) is True
        assert _is_interop_type(NodeDBConnection) is True
        assert _is_interop_type(FlowImage) is True
        assert _is_interop_type(str) is False

    def test_scalar_data_type(self):
        dt, model = _scalar_data_type(FlowPath)
        assert dt == PinType.STRUCT
        assert model is FlowPath

        dt2, model2 = _scalar_data_type(Bit)
        assert dt2 == PinType.STRUCT
        assert model2 is Bit

    def test_node_with_interop_pin(self):
        class LlmNode(WasmNode, name="test_llm", category="AI"):
            """Test LLM node"""
            model: Bit = Input()
            messages: list[ChatMessage] = Input(default_factory=list)
            response: str = Output()

            def run(self, ctx) -> ExecutionResult:
                return ctx.success()

        defn = LlmNode().get_node()
        assert defn.name == "test_llm"
        model_pin = next(p for p in defn.pins if p.name == "model")
        assert model_pin.data_type == PinType.STRUCT
        assert model_pin.schema is not None

    def test_typed_context_interop_deser(self):
        host = MockHostBridge()
        ctx = Context(ExecutionInput(inputs={
            "path": {"path": "data/file.csv", "store_ref": "s3"},
        }), host)

        class FileNode(WasmNode, name="test_file_deser", category="Test"):
            """Test"""
            path: FlowPath = Input()
            result: str = Output()

            def run(self, ctx) -> ExecutionResult:
                ctx.result = ctx.path.path
                return ctx.success()

        node = FileNode()
        result = node.run(ctx)
        assert result.outputs["result"] == "data/file.csv"


class TestContentPartsSdk:
    def test_text(self):
        p = ContentPart.text("Hello")
        assert p.to_dict() == {"type": "text", "text": "Hello"}

    def test_image(self):
        p = ContentPart.image("http://x.com/i.png", "image/png")
        d = p.to_dict()
        assert d["type"] == "image"
        assert d["image"]["media_type"] == "image/png"

    def test_tool_call(self):
        p = ContentPart.tool_call("call_1", "search", {"q": "hello"})
        d = p.to_dict()
        assert d["type"] == "tool_call"
        assert d["tool_call"]["name"] == "search"

    def test_reasoning(self):
        p = ContentPart.reasoning(["step1", "step2"])
        d = p.to_dict()
        assert d["reasoning"]["text"] == ["step1", "step2"]


class TestDataTypesSdk:
    def test_image_data(self):
        d = ImageData(url="http://x.com/i.png", media_type="image/png", detail="high")
        assert d.to_dict() == {"url": "http://x.com/i.png", "media_type": "image/png", "detail": "high"}

    def test_tool_call_data(self):
        d = ToolCallData(id="1", name="search", arguments={"q": "test"})
        assert d.to_dict()["name"] == "search"

    def test_tool_result_data(self):
        d = ToolResultData(id="1", content="result text")
        assert d.to_dict() == {"id": "1", "content": "result text"}
