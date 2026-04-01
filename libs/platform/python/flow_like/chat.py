"""Chat completions mixin for the Flow-Like Python SDK."""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterator
from typing import Any

from ._http import HTTPClient
from ._types import ChatChoice, ChatCompletionResult, ChatMessage, SSEEvent, UsageInfo


def _build_messages(messages: list[ChatMessage | dict[str, str]]) -> list[dict[str, str]]:
    """Normalize a list of messages into plain dicts."""
    return [
        {"role": m.role, "content": m.content} if isinstance(m, ChatMessage) else m
        for m in messages
    ]


def _parse_usage(raw: dict[str, Any]) -> UsageInfo:
    """Convert a raw usage dict into a typed ``UsageInfo``."""
    return UsageInfo(
        prompt_tokens=raw.get("prompt_tokens", 0),
        completion_tokens=raw.get("completion_tokens", 0),
        total_tokens=raw.get("total_tokens", 0),
        raw=raw,
    )


def _parse_choice(raw: dict[str, Any]) -> ChatChoice:
    """Convert a raw choice dict into a typed ``ChatChoice``."""
    msg = raw.get("message", {})
    return ChatChoice(
        index=raw.get("index", 0),
        message=ChatMessage(
            role=msg.get("role", "assistant"),
            content=msg.get("content", ""),
        ),
        finish_reason=raw.get("finish_reason"),
        raw=raw,
    )


class ChatMixin(HTTPClient):
    """Mixin providing chat completion capabilities."""

    def chat_completions(
        self,
        messages: list[ChatMessage | dict[str, str]],
        bit_id: str,
        stream: bool = False,
        **kwargs: Any,
    ) -> ChatCompletionResult | Iterator[SSEEvent]:
        """Send a chat completion request.

        Args:
            messages: Conversation history as ``ChatMessage`` objects or plain dicts.
            bit_id: Identifier of the model bit to use.
            stream: If ``True``, return an iterator of server-sent events.
            **kwargs: Additional payload fields forwarded to the API.

        Returns:
            A ``ChatCompletionResult`` when *stream* is ``False``, or an
            ``Iterator[SSEEvent]`` when *stream* is ``True``.
        """
        payload: dict[str, Any] = {
            "messages": _build_messages(messages),
            "model": bit_id,
            "stream": stream,
            **kwargs,
        }
        if stream:
            return self._stream_sse("POST", "/chat/completions", json=payload)
        resp = self._request("POST", "/chat/completions", json=payload)
        data = resp.json()
        return ChatCompletionResult(
            choices=[_parse_choice(c) for c in data.get("choices", [])],
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
        )

    async def achat_completions(
        self,
        messages: list[ChatMessage | dict[str, str]],
        bit_id: str,
        stream: bool = False,
        **kwargs: Any,
    ) -> ChatCompletionResult | AsyncIterator[SSEEvent]:
        """Async version of ``chat_completions``."""
        payload: dict[str, Any] = {
            "messages": _build_messages(messages),
            "model": bit_id,
            "stream": stream,
            **kwargs,
        }
        if stream:
            return self._astream_sse("POST", "/chat/completions", json=payload)
        resp = await self._arequest("POST", "/chat/completions", json=payload)
        data = resp.json()
        return ChatCompletionResult(
            choices=[_parse_choice(c) for c in data.get("choices", [])],
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
        )

    def get_usage(self) -> dict[str, Any]:
        """Retrieve aggregated chat usage statistics.

        Returns:
            A dict containing usage metrics.
        """
        resp = self._request("GET", "/chat/usage")
        return resp.json()

    async def aget_usage(self) -> dict[str, Any]:
        """Async version of ``get_usage``."""
        resp = await self._arequest("GET", "/chat/usage")
        return resp.json()


__all__ = ["ChatMixin"]
