"""Tests for the LangChain wrapper module."""

import pytest
from flow_like_wasm_sdk import (
    Bit,
    CachedEmbeddingModel,
    Context,
    ExecutionInput,
    MockHostBridge,
)
from flow_like_wasm_sdk.langchain import (
    FlowLikeChatModel,
    FlowLikeEmbeddings,
    _lc_to_chat_message,
    _response_to_ai_message,
)
from langchain_core.messages import (
    AIMessage,
    HumanMessage,
    SystemMessage,
    ToolMessage,
)


@pytest.fixture
def mock_ctx():
    host = MockHostBridge()
    return Context(ExecutionInput(inputs={}), host)


@pytest.fixture
def bit():
    return Bit(id="gpt-4o", bit_type="llm", hub="openai")


@pytest.fixture
def embedding_model():
    return CachedEmbeddingModel(cache_key="test-embedder")


class TestMessageConversion:
    def test_human_message(self):
        msg = _lc_to_chat_message(HumanMessage(content="Hello"))
        assert msg.role == "user"
        assert msg.content == "Hello"

    def test_system_message(self):
        msg = _lc_to_chat_message(SystemMessage(content="You are helpful"))
        assert msg.role == "system"
        assert msg.content == "You are helpful"

    def test_ai_message(self):
        msg = _lc_to_chat_message(AIMessage(content="I can help"))
        assert msg.role == "assistant"
        assert msg.content == "I can help"

    def test_tool_message(self):
        msg = _lc_to_chat_message(ToolMessage(content="result data", tool_call_id="call_123"))
        assert msg.role == "tool"
        assert msg.content == "result data"
        assert msg.tool_call_id == "call_123"

    def test_multimodal_message(self):
        msg = _lc_to_chat_message(HumanMessage(content=[
            {"type": "text", "text": "What's in this image?"},
            {"type": "image_url", "image_url": {"url": "https://example.com/img.png"}},
        ]))
        assert msg.role == "user"
        assert isinstance(msg.content, list)
        assert len(msg.content) == 2
        assert msg.content[0]["type"] == "text"
        assert msg.content[1]["type"] == "image"

    def test_multimodal_with_plain_string_blocks(self):
        msg = _lc_to_chat_message(HumanMessage(content=["Hello", "World"]))
        assert isinstance(msg.content, list)
        assert len(msg.content) == 2
        assert msg.content[0]["type"] == "text"
        assert msg.content[0]["text"] == "Hello"


class TestResponseConversion:
    def test_json_response(self):
        ai = _response_to_ai_message('{"role": "assistant", "content": "Hello back"}')
        assert ai.content == "Hello back"

    def test_plain_text_response(self):
        ai = _response_to_ai_message("Just plain text")
        assert ai.content == "Just plain text"

    def test_invalid_json(self):
        ai = _response_to_ai_message("{not valid json")
        assert ai.content == "{not valid json"

    def test_json_without_content(self):
        ai = _response_to_ai_message('{"status": "ok"}')
        assert ai.content == '{"status": "ok"}'


class TestFlowLikeChatModel:
    def test_invoke_string(self, mock_ctx, bit):
        llm = FlowLikeChatModel(bit=bit, ctx=mock_ctx)
        result = llm.invoke("Hello!")
        assert isinstance(result, AIMessage)
        assert result.content == "Mock LLM response"

    def test_invoke_messages(self, mock_ctx, bit):
        llm = FlowLikeChatModel(bit=bit, ctx=mock_ctx)
        result = llm.invoke([
            SystemMessage(content="You are a helper"),
            HumanMessage(content="What is 2+2?"),
        ])
        assert isinstance(result, AIMessage)
        assert "Mock LLM" in result.content

    def test_generate(self, mock_ctx, bit):
        llm = FlowLikeChatModel(bit=bit, ctx=mock_ctx)
        chat_result = llm._generate([HumanMessage(content="Hi")])
        assert len(chat_result.generations) == 1
        assert isinstance(chat_result.generations[0].message, AIMessage)

    def test_llm_type(self, mock_ctx, bit):
        llm = FlowLikeChatModel(bit=bit, ctx=mock_ctx)
        assert llm._llm_type == "flow-like-wasm"

    def test_none_response(self, mock_ctx, bit):
        mock_ctx._host.llm_prompt = lambda *args, **kwargs: None
        llm = FlowLikeChatModel(bit=bit, ctx=mock_ctx)
        result = llm.invoke("Hello")
        assert isinstance(result, AIMessage)
        assert result.content == ""


class TestFlowLikeEmbeddings:
    def test_embed_documents(self, mock_ctx, embedding_model):
        emb = FlowLikeEmbeddings(model=embedding_model, ctx=mock_ctx)
        vecs = emb.embed_documents(["doc1", "doc2"])
        assert len(vecs) == 2
        assert len(vecs[0]) == 3  # MockHostBridge returns [0.1, 0.2, 0.3]

    def test_embed_query(self, mock_ctx, embedding_model):
        emb = FlowLikeEmbeddings(model=embedding_model, ctx=mock_ctx)
        vec = emb.embed_query("search text")
        assert len(vec) == 3
        assert vec == [0.1, 0.2, 0.3]

    def test_embed_documents_empty(self, mock_ctx, embedding_model):
        emb = FlowLikeEmbeddings(model=embedding_model, ctx=mock_ctx)
        vecs = emb.embed_documents([])
        assert vecs == []

    def test_embed_returns_empty_on_none(self, mock_ctx, embedding_model):
        mock_ctx._host.embed_text_document = lambda *args, **kwargs: None
        emb = FlowLikeEmbeddings(model=embedding_model, ctx=mock_ctx)
        vecs = emb.embed_documents(["test"])
        assert vecs == []

    def test_embed_query_returns_empty_on_none(self, mock_ctx, embedding_model):
        mock_ctx._host.embed_text_query = lambda *args, **kwargs: None
        emb = FlowLikeEmbeddings(model=embedding_model, ctx=mock_ctx)
        vec = emb.embed_query("test")
        assert vec == []
