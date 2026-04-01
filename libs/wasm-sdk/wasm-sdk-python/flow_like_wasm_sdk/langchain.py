"""LangChain wrappers for the Flow-Like WASM SDK.

Provides :class:`FlowLikeChatModel` (a LangChain ``BaseChatModel``) and
:class:`FlowLikeEmbeddings` (a LangChain ``Embeddings``) that delegate
to the host runtime through the SDK's :class:`Context`.

Supports tool calling / function calling for use with LangChain agents
and LangGraph workflows.

Works in two modes:

- **Native Python**: uses real ``langchain_core`` (requires pydantic).
- **WASM**: falls back to pure-Python stubs shipped in the SDK when
  ``langchain_core`` cannot be imported (pydantic_core is a native
  extension that cannot run inside WASM).

Usage::

    from flow_like_wasm_sdk.langchain import FlowLikeChatModel, FlowLikeEmbeddings
    from flow_like_wasm_sdk.langchain import HumanMessage, SystemMessage, ToolMessage

    llm = FlowLikeChatModel(bit=my_bit, ctx=ctx)
    result = llm.invoke("Hello!")

    # With tool calling:
    llm_with_tools = llm.bind_tools([my_tool])
    result = llm_with_tools.invoke([HumanMessage(content="Use the tool")])
"""

from __future__ import annotations

import json
from typing import Any

# Try real langchain_core; fall back to pure-Python stubs for WASM
try:
    from langchain_core.callbacks import CallbackManagerForLLMRun
    from langchain_core.embeddings import Embeddings
    from langchain_core.language_models.chat_models import BaseChatModel
    from langchain_core.messages import (
        AIMessage,
        BaseMessage,
        HumanMessage,
        SystemMessage,
        ToolMessage,
    )
    from langchain_core.outputs import ChatGeneration, ChatResult

    _USING_STUBS = False
except ImportError:
    from flow_like_wasm_sdk._langchain_stubs import (  # type: ignore[assignment]
        AIMessage,
        BaseMessage,
        BaseChatModel,
        CallbackManagerForLLMRun,
        ChatGeneration,
        ChatResult,
        Embeddings,
        HumanMessage,  # noqa: F401 — re-exported
        SystemMessage,  # noqa: F401 — re-exported
        ToolMessage,
    )

    _USING_STUBS = True

from flow_like_wasm_sdk.context import Context
from flow_like_wasm_sdk.interop import CachedEmbeddingModel, ChatMessage, ContentPart

_ROLE_MAP: dict[str, str] = {
    "human": "user",
    "ai": "assistant",
    "system": "system",
    "tool": "tool",
}


def _lc_to_chat_message(msg: BaseMessage) -> ChatMessage:
    """Convert a LangChain message to the SDK's ChatMessage."""
    role = _ROLE_MAP.get(msg.type, "user")

    # Handle ToolMessage first (before content parsing)
    if isinstance(msg, ToolMessage):
        content = str(msg.content) if msg.content else ""
        return ChatMessage.tool_result(
            tool_call_id=getattr(msg, "tool_call_id", ""),
            content=content,
        )

    if isinstance(msg.content, list):
        parts: list[ContentPart] = []
        for block in msg.content:
            if isinstance(block, str):
                parts.append(ContentPart.text(block))
            elif isinstance(block, dict):
                btype = block.get("type", "")
                if btype == "text":
                    parts.append(ContentPart.text(block.get("text", "")))
                elif btype == "image_url":
                    url_data = block.get("image_url", {})
                    url = url_data.get("url", "") if isinstance(url_data, dict) else str(url_data)
                    parts.append(ContentPart.image_url(url))
        cm = ChatMessage(role=role, content=[p.to_dict() for p in parts])
    else:
        content = str(msg.content) if msg.content else ""
        cm = ChatMessage(role=role, content=content)

    # Carry tool_calls from AIMessage (needed for agent loops)
    tc_list = getattr(msg, "tool_calls", None)
    if tc_list:
        cm.tool_calls = [
            {"id": tc.get("id", ""), "name": tc["name"], "arguments": tc.get("args", {})}
            for tc in tc_list
        ]
    return cm


def _response_to_ai_message(response_text: str) -> AIMessage:
    """Convert an LLM response string to a LangChain AIMessage.

    Parses tool calls from the response JSON if present.
    """
    try:
        data = json.loads(response_text)
        if isinstance(data, dict):
            content = data.get("content", "") or ""
            raw_tcs = data.get("tool_calls")
            if raw_tcs:
                tool_calls = [
                    {
                        "id": tc.get("id", ""),
                        "name": tc.get("name", ""),
                        "args": tc.get("arguments", {}),
                    }
                    for tc in raw_tcs
                ]
                return AIMessage(content=content, tool_calls=tool_calls)
            return AIMessage(content=content)
    except (json.JSONDecodeError, TypeError):
        pass
    return AIMessage(content=response_text)


def _convert_lc_tools(tools: list[Any]) -> list[dict[str, Any]]:
    """Convert LangChain tool definitions to the host-expected format."""
    result = []
    for tool in tools:
        if isinstance(tool, dict):
            if "function" in tool:
                fn = tool["function"]
                result.append({
                    "name": fn["name"],
                    "description": fn.get("description", ""),
                    "parameters": fn.get("parameters", {}),
                })
            elif "name" in tool:
                result.append(tool)
        elif hasattr(tool, "name") and hasattr(tool, "args_schema"):
            schema = {}
            if tool.args_schema:
                try:
                    schema = tool.args_schema.model_json_schema()
                except Exception:
                    if hasattr(tool.args_schema, "schema"):
                        schema = tool.args_schema.schema()
            result.append({
                "name": tool.name,
                "description": getattr(tool, "description", "") or "",
                "parameters": schema,
            })
    return result


class FlowLikeChatModel(BaseChatModel):
    """LangChain ``BaseChatModel`` backed by the WASM host runtime.

    Supports tool calling via ``bind_tools()`` for use with LangChain
    agents and LangGraph workflows.
    """

    if not _USING_STUBS:
        model_config = {"arbitrary_types_allowed": True}

    def __init__(self, *, bit: Any, ctx: Any, **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.bit = bit
        self.ctx = ctx
        self._bound_tools: list[dict[str, Any]] = []

    @property
    def _llm_type(self) -> str:
        return "flow-like-wasm"

    def bind_tools(self, tools: list[Any], **kwargs: Any) -> FlowLikeChatModel:
        """Return a copy with tools bound for function calling."""
        import copy
        bound = copy.copy(self)
        bound._bound_tools = _convert_lc_tools(tools)
        return bound

    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> ChatResult:
        sdk_messages = [_lc_to_chat_message(m) for m in messages]
        tools_for_host = self._bound_tools or None
        if "tools" in kwargs and kwargs["tools"]:
            tools_for_host = _convert_lc_tools(kwargs["tools"])
        response = self.ctx.llm_prompt(
            self.bit, sdk_messages, stream=False, tools=tools_for_host,
        )
        if response is None:
            ai_msg = AIMessage(content="")
        else:
            ai_msg = _response_to_ai_message(response)
        return ChatResult(generations=[ChatGeneration(message=ai_msg)])


class FlowLikeEmbeddings(Embeddings):
    """LangChain ``Embeddings`` backed by the WASM host runtime."""

    def __init__(self, *, model: CachedEmbeddingModel, ctx: Context) -> None:
        self._model = model
        self._ctx = ctx

    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        result = self._model.embed_document(self._ctx, texts)
        return result if result is not None else []

    def embed_query(self, text: str) -> list[float]:
        result = self._model.embed_query(self._ctx, [text])
        if result and len(result) > 0:
            return result[0]
        return []
