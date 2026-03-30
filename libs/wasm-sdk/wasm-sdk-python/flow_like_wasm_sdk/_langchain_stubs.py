"""Pure-Python langchain-compatible stubs for WASM environments.

Real langchain_core requires pydantic_core (native Rust extension) which
cannot run inside WASM.  This module provides lightweight replacements for
the subset of langchain_core types used by :mod:`flow_like_wasm_sdk.langchain`
and by user nodes.

These classes are API-compatible with langchain_core for the features used
by the SDK (message types, BaseChatModel, Embeddings, tool calling).
"""

from __future__ import annotations

import copy
from abc import ABC, abstractmethod
from typing import Any


# ── Messages ────────────────────────────────────────────────────────────


class BaseMessage:
    """Minimal langchain-compatible message base class."""

    type: str = "base"

    def __init__(self, content: Any = "", **kwargs: Any) -> None:
        self.content = content
        self.tool_calls: list[dict[str, Any]] = kwargs.get("tool_calls", [])
        self.tool_call_id: str = kwargs.get("tool_call_id", "")
        for k, v in kwargs.items():
            if k not in ("tool_calls", "tool_call_id"):
                setattr(self, k, v)


class HumanMessage(BaseMessage):
    type = "human"


class SystemMessage(BaseMessage):
    type = "system"


class AIMessage(BaseMessage):
    type = "ai"

    def __init__(self, content: Any = "", **kwargs: Any) -> None:
        super().__init__(content, **kwargs)
        if not self.tool_calls:
            self.tool_calls = []


class ToolMessage(BaseMessage):
    type = "tool"

    def __init__(self, content: Any = "", **kwargs: Any) -> None:
        super().__init__(content, **kwargs)
        self.tool_call_id = kwargs.get("tool_call_id", "")


# ── Outputs ─────────────────────────────────────────────────────────────


class ChatGeneration:
    def __init__(self, *, message: BaseMessage | None = None, text: str = "") -> None:
        self.message = message or AIMessage(content=text)
        self.text = text or (str(self.message.content) if self.message else "")


class ChatResult:
    def __init__(self, *, generations: list[ChatGeneration] | None = None) -> None:
        self.generations = generations or []


# ── BaseChatModel ───────────────────────────────────────────────────────


class CallbackManagerForLLMRun:
    """Placeholder — only used as a type hint."""


class BaseChatModel(ABC):
    """Minimal langchain-compatible BaseChatModel.

    Subclasses implement ``_generate()``; ``invoke()`` and ``bind_tools()``
    are provided here.
    """

    def __init__(self, **kwargs: Any) -> None:
        for k, v in kwargs.items():
            setattr(self, k, v)

    @property
    @abstractmethod
    def _llm_type(self) -> str: ...

    @abstractmethod
    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> ChatResult: ...

    def invoke(
        self,
        input: Any,
        config: Any = None,
        **kwargs: Any,
    ) -> BaseMessage:
        if isinstance(input, str):
            messages = [HumanMessage(content=input)]
        elif isinstance(input, list):
            messages = input
        else:
            messages = [HumanMessage(content=str(input))]
        result = self._generate(messages, **kwargs)
        if result.generations:
            return result.generations[0].message
        return AIMessage(content="")

    def bind_tools(self, tools: list[Any], **kwargs: Any) -> BaseChatModel:
        bound = copy.copy(self)
        bound._bound_tools = tools  # type: ignore[attr-defined]
        return bound

    def model_copy(self) -> BaseChatModel:
        return copy.copy(self)


# ── Embeddings ──────────────────────────────────────────────────────────


class Embeddings(ABC):
    @abstractmethod
    def embed_documents(self, texts: list[str]) -> list[list[float]]: ...

    @abstractmethod
    def embed_query(self, text: str) -> list[float]: ...
