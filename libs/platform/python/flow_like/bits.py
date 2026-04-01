"""Mixin for bit search and model discovery endpoints."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient
from ._types import ModelInfo


def _extract_model_info(bit: dict[str, Any], lang: str = "en") -> ModelInfo:
    """Build a ModelInfo from a raw bit dict."""
    meta = bit.get("meta", {})
    localized = meta.get(lang) or next(iter(meta.values()), {})
    params = bit.get("parameters", {}) or {}
    provider = params.get("provider", params)

    return ModelInfo(
        bit_id=bit.get("id", ""),
        name=localized.get("name", bit.get("id", "")),
        description=localized.get("description", ""),
        provider_name=provider.get("provider_name") if isinstance(provider, dict) else None,
        model_id=provider.get("model_id") if isinstance(provider, dict) else None,
        context_length=params.get("context_length"),
        vector_length=params.get("vector_length"),
        languages=params.get("languages", []),
        tags=localized.get("tags", []),
    )


def _has_remote_provider(bit: dict[str, Any]) -> bool:
    """Return True if the bit is backed by a remote/hosted provider."""
    params = bit.get("parameters", {}) or {}

    # LLM/VLM pattern: provider_name starts with "hosted"
    provider = params.get("provider", params)
    if isinstance(provider, dict):
        name = provider.get("provider_name", "")
        if isinstance(name, str) and name.lower().startswith("hosted"):
            return True

    # Embedding pattern: remote config with endpoint + implementation
    remote = params.get("remote")
    if isinstance(remote, dict) and remote.get("endpoint") and remote.get("implementation"):
        return True

    return False


class BitsMixin(HTTPClient):
    """HTTP methods for searching bits and listing available models."""

    def search_bits(
        self,
        search: str | None = None,
        bit_types: list[str] | None = None,
        limit: int = 50,
        offset: int = 0,
    ) -> list[dict[str, Any]]:
        """Search the bit catalog.

        Args:
            search: Free-text query string.
            bit_types: Filter by bit type names (e.g. ``["Llm"]``).
            limit: Maximum results to return.
            offset: Pagination offset.

        Returns:
            Raw bit dicts from the API.
        """
        body: dict[str, Any] = {"limit": limit, "offset": offset}
        if search is not None:
            body["search"] = search
        if bit_types is not None:
            body["bit_types"] = bit_types
        resp = self._request("POST", "/bit", json=body)
        return resp.json()

    async def asearch_bits(
        self,
        search: str | None = None,
        bit_types: list[str] | None = None,
        limit: int = 50,
        offset: int = 0,
    ) -> list[dict[str, Any]]:
        """Async version of search_bits."""
        body: dict[str, Any] = {"limit": limit, "offset": offset}
        if search is not None:
            body["search"] = search
        if bit_types is not None:
            body["bit_types"] = bit_types
        resp = await self._arequest("POST", "/bit", json=body)
        return resp.json()

    def get_bit(self, bit_id: str) -> dict[str, Any]:
        """Fetch a single bit by ID.

        Args:
            bit_id: Unique bit identifier.

        Returns:
            Raw bit dict.
        """
        resp = self._request("GET", f"/bit/{bit_id}")
        return resp.json()

    async def aget_bit(self, bit_id: str) -> dict[str, Any]:
        """Async version of get_bit."""
        resp = await self._arequest("GET", f"/bit/{bit_id}")
        return resp.json()

    def list_llms(self, search: str | None = None, limit: int = 50) -> list[ModelInfo]:
        """List available remote LLM/VLM models.

        Args:
            search: Optional text filter.
            limit: Maximum results.

        Returns:
            List of ModelInfo for hosted LLM/VLM bits.
        """
        bits = self.search_bits(search=search, bit_types=["Llm", "Vlm"], limit=limit)
        return [
            _extract_model_info(b)
            for b in bits
            if b.get("type") in ("Llm", "Vlm") and _has_remote_provider(b)
        ]

    async def alist_llms(self, search: str | None = None, limit: int = 50) -> list[ModelInfo]:
        """Async version of list_llms."""
        bits = await self.asearch_bits(search=search, bit_types=["Llm", "Vlm"], limit=limit)
        return [
            _extract_model_info(b)
            for b in bits
            if b.get("type") in ("Llm", "Vlm") and _has_remote_provider(b)
        ]

    def list_embedding_models(
        self, search: str | None = None, limit: int = 50
    ) -> list[ModelInfo]:
        """List available remote embedding models.

        Args:
            search: Optional text filter.
            limit: Maximum results.

        Returns:
            List of ModelInfo for hosted embedding bits.
        """
        bits = self.search_bits(search=search, bit_types=["Embedding"], limit=limit)
        return [
            _extract_model_info(b)
            for b in bits
            if b.get("type") == "Embedding" and _has_remote_provider(b)
        ]

    async def alist_embedding_models(
        self, search: str | None = None, limit: int = 50
    ) -> list[ModelInfo]:
        """Async version of list_embedding_models."""
        bits = await self.asearch_bits(search=search, bit_types=["Embedding"], limit=limit)
        return [
            _extract_model_info(b)
            for b in bits
            if b.get("type") == "Embedding" and _has_remote_provider(b)
        ]


__all__ = ["BitsMixin"]
