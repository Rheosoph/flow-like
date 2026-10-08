"""Embeddings mixin for the Flow-Like Python SDK."""

from __future__ import annotations

from typing import Any, Literal, Sequence

from ._http import HTTPClient
from ._types import EmbeddingInput, EmbeddingResult, UsageInfo


def _parse_usage(raw: dict[str, Any]) -> UsageInfo:
    """Convert a raw usage dict into a typed ``UsageInfo``."""
    return UsageInfo(
        prompt_tokens=raw.get("prompt_tokens", 0),
        completion_tokens=raw.get("completion_tokens", 0),
        total_tokens=raw.get("total_tokens", 0),
        raw=raw,
    )


class EmbeddingsMixin(HTTPClient):
    """Generate text and media embeddings with a compatible model bit."""
    def embed(
        self,
        bit_id: str,
        input: EmbeddingInput | Sequence[EmbeddingInput],
        embed_type: Literal["query", "document"] = "query",
        **kwargs: Any,
    ) -> EmbeddingResult:
        """Generate embeddings for text, media, or a mixed batch.

        Args:
            bit_id: Identifier of the embedding model bit to use.
            input: A string, structured input, or sequence of inputs. Media
                fields accept HTTP(S) URLs, base64 data URLs, or raw base64.
                A ``"multimodal"`` input produces one combined embedding.
            embed_type: Whether the input is a ``"query"`` or ``"document"``.
            **kwargs: Additional payload fields forwarded to the API.

        Returns:
            An ``EmbeddingResult`` containing the embedding vectors and usage.
        """
        inputs = [input] if isinstance(input, (str, dict)) else list(input)
        payload: dict[str, Any] = {
            "model": bit_id,
            "input": inputs,
            "embed_type": embed_type,
            **kwargs,
        }
        resp = self._request("POST", "/embeddings/embed", json=payload)
        data = resp.json()
        return EmbeddingResult(
            embeddings=data.get("embeddings", []),
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
            usage_estimated=data.get("usage_estimated"),
            usage_available=data.get("usage_available"),
        )

    async def aembed(
        self,
        bit_id: str,
        input: EmbeddingInput | Sequence[EmbeddingInput],
        embed_type: Literal["query", "document"] = "query",
        **kwargs: Any,
    ) -> EmbeddingResult:
        """Async version of ``embed``."""
        inputs = [input] if isinstance(input, (str, dict)) else list(input)
        payload: dict[str, Any] = {
            "model": bit_id,
            "input": inputs,
            "embed_type": embed_type,
            **kwargs,
        }
        resp = await self._arequest("POST", "/embeddings/embed", json=payload)
        data = resp.json()
        return EmbeddingResult(
            embeddings=data.get("embeddings", []),
            usage=_parse_usage(data.get("usage", {})),
            raw=data,
            usage_estimated=data.get("usage_estimated"),
            usage_available=data.get("usage_available"),
        )


__all__ = ["EmbeddingsMixin"]
