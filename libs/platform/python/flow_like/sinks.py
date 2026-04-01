"""Mixin for triggering HTTP sink endpoints."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient


class SinksMixin(HTTPClient):
    """HTTP mixin that provides sink-triggering capabilities."""
    def trigger_http_sink(
        self,
        app_id: str,
        path: str,
        method: str = "POST",
        body: dict[str, Any] | list[Any] | None = None,
        **kwargs: Any,
    ) -> dict[str, Any]:
        """Send an HTTP request to a sink endpoint.

        Args:
            app_id: The application identifier.
            path: The sink path to invoke.
            method: HTTP method to use (default ``"POST"``).
            body: Optional JSON-serialisable request body.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            The JSON response from the sink as a dict.
        """
        resp = self._request(
            method.upper(),
            f"/sink/trigger/http/{app_id}/{path}",
            json=body,
            **kwargs,
        )
        return resp.json()

    async def atrigger_http_sink(
        self,
        app_id: str,
        path: str,
        method: str = "POST",
        body: dict[str, Any] | list[Any] | None = None,
        **kwargs: Any,
    ) -> dict[str, Any]:
        """Async version of ``trigger_http_sink``.

        Args:
            app_id: The application identifier.
            path: The sink path to invoke.
            method: HTTP method to use (default ``"POST"``).
            body: Optional JSON-serialisable request body.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            The JSON response from the sink as a dict.
        """
        resp = await self._arequest(
            method.upper(),
            f"/sink/trigger/http/{app_id}/{path}",
            json=body,
            **kwargs,
        )
        return resp.json()


__all__ = ["SinksMixin"]
