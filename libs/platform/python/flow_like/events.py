"""Mixin for triggering application events via the Flow-Like API."""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterator
from typing import Any

from ._http import HTTPClient
from ._types import AsyncInvokeResult, SSEEvent


class EventsMixin(HTTPClient):
    """HTTP mixin that provides event-triggering capabilities."""
    def trigger_event(
        self,
        app_id: str,
        event_id: str,
        payload: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> Iterator[SSEEvent]:
        """Trigger an event and stream the response as server-sent events.

        Args:
            app_id: The application identifier.
            event_id: The event identifier to trigger.
            payload: Optional JSON-serialisable payload for the event.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            An iterator of SSEEvent objects representing the streamed response.
        """
        return self._stream_sse(
            "POST",
            f"/apps/{app_id}/events/{event_id}/invoke",
            json=payload,
            **kwargs,
        )

    async def atrigger_event(
        self,
        app_id: str,
        event_id: str,
        payload: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> AsyncIterator[SSEEvent]:
        """Async version of ``trigger_event``.

        Args:
            app_id: The application identifier.
            event_id: The event identifier to trigger.
            payload: Optional JSON-serialisable payload for the event.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            An async iterator of SSEEvent objects.
        """
        return self._astream_sse(
            "POST",
            f"/apps/{app_id}/events/{event_id}/invoke",
            json=payload,
            **kwargs,
        )

    def trigger_event_async(
        self,
        app_id: str,
        event_id: str,
        payload: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> AsyncInvokeResult:
        """Trigger an event for asynchronous (non-blocking) execution.

        Args:
            app_id: The application identifier.
            event_id: The event identifier to trigger.
            payload: Optional JSON-serialisable payload for the event.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            An ``AsyncInvokeResult`` containing the run ID and poll token.
        """
        resp = self._request(
            "POST",
            f"/apps/{app_id}/events/{event_id}/invoke-async",
            json=payload,
            **kwargs,
        )
        data = resp.json()
        return AsyncInvokeResult(
            run_id=data.get("run_id", ""),
            poll_token=data.get("poll_token", ""),
            raw=data,
        )

    async def atrigger_event_async(
        self,
        app_id: str,
        event_id: str,
        payload: dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> AsyncInvokeResult:
        """Async version of ``trigger_event_async``.

        Args:
            app_id: The application identifier.
            event_id: The event identifier to trigger.
            payload: Optional JSON-serialisable payload for the event.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            An ``AsyncInvokeResult`` containing the run ID and poll token.
        """
        resp = await self._arequest(
            "POST",
            f"/apps/{app_id}/events/{event_id}/invoke-async",
            json=payload,
            **kwargs,
        )
        data = resp.json()
        return AsyncInvokeResult(
            run_id=data.get("run_id", ""),
            poll_token=data.get("poll_token", ""),
            raw=data,
        )


__all__ = ["EventsMixin"]
