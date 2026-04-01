"""Embeddings mixin for the Flow-Like Python SDK."""

from __future__ import annotations

from typing import Any, Literal

from ._http import HTTPClient
from ._types import EmbeddingResult, UsageInfo


def _parse_usage(raw: dict[str, Any]) -> UsageInfo:
    """Convert a raw usage dict into a typed ``UsageInfo``."""
    return UsageInfo(
        prompt_tokens=raw.get("prompt_tokens", 0),
        completion_tokens=raw.get("completion_tokens", 0),
        total_tokens=raw.get("total_tokens", 0),
        raw=raw,
    )


class EmbeddingsMixin(HTTPClient):
    """Mixin providing text embedding capabilities."""
    def embed(
        self,
        bit_id: str,
        input: str | list[str],
        embed_type: Literal["query", "document"] = "query",
        **kwargs: Any,
    ) -> EmbeddingResult:
        """Generate embeddings for one or more texts.

        Args:
            bit_id: Identifier of the embedding model bit to use.
            input: A single string or list of strings to embed.
            embed_type: Whether the input is a ``"query"`` or ``"document"``.
            **kwargs: Additional payload fields forwarded to the API.

        Returns:
            An ``EmbeddingResult`` containing the embedding vectors and usage.
        """
        texts = [input] if isinstance(input, str) else input
        payload: dict[str, Any] = {
            "model": bit_id,
            "input": texts,
            "embed_type": embed_type,
            **kwargs,
        }
        resp = self._request("POST", "/embeddings/embed", json=payload)
        data = resp.json()
        return EmbeddingResult(
            embeddings=data.get("embeddings", []),
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
        )

    async def aembed(
        self,
        bit_id: str,
        input: str | list[str],
        embed_type: Literal["query", "document"] = "query",
        **kwargs: Any,
    ) -> EmbeddingResult:
        """Async version of ``embed``."""
        texts = [input] if isinstance(input, str) else input
        payload: dict[str, Any] = {
            "model": bit_id,
            "input": texts,
            "embed_type": embed_type,
            **kwargs,
        }
        resp = await self._arequest("POST", "/embeddings/embed", json=payload)
        data = resp.json()
        return EmbeddingResult(
            embeddings=data.get("embeddings", []),
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
        )


__all__ = ["EmbeddingsMixin"]
