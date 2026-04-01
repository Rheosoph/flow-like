"""Low-level HTTP transport for the Flow-Like API.

Provides synchronous and asynchronous request helpers with SSE streaming,
automatic authentication, and structured error handling.
"""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterator
from typing import Any

import httpx

from ._auth import resolve_auth, resolve_base_url
from ._errors import APIError, NotFoundError, RateLimitError, ServerError
from ._types import SSEEvent

DEFAULT_TIMEOUT = 30.0
SSE_TIMEOUT = 300.0


class HTTPClient:
    """Thin wrapper around ``httpx`` for sync and async API calls."""
    def __init__(
        self,
        base_url: str | None = None,
        pat: str | None = None,
        api_key: str | None = None,
        timeout: float = DEFAULT_TIMEOUT,
    ):
        """Initialise the HTTP client.

        Args:
            base_url: Override for the API base URL.
            pat: Personal access token for authentication.
            api_key: API key for authentication.
            timeout: Default request timeout in seconds.
        """
        self._base_url = resolve_base_url(base_url)
        self._api_base = f"{self._base_url}/api/v1"
        self._auth_headers = resolve_auth(pat=pat, api_key=api_key)
        self._token = self._auth_headers.get("Authorization") or self._auth_headers.get("X-API-Key", "")
        self._timeout = timeout
        self._client: httpx.Client | None = None
        self._async_client: httpx.AsyncClient | None = None

    def _get_client(self) -> httpx.Client:
        """Return the lazily-initialised synchronous HTTP client."""
        if self._client is None:
            self._client = httpx.Client(
                base_url=self._api_base,
                headers=self._auth_headers,
                timeout=self._timeout,
            )
        return self._client

    def _get_async_client(self) -> httpx.AsyncClient:
        """Return the lazily-initialised asynchronous HTTP client."""
        if self._async_client is None:
            self._async_client = httpx.AsyncClient(
                base_url=self._api_base,
                headers=self._auth_headers,
                timeout=self._timeout,
            )
        return self._async_client

    def close(self) -> None:
        """Close underlying sync and async transports."""
        if self._client is not None:
            self._client.close()
            self._client = None
        if self._async_client is not None:
            import asyncio

            try:
                loop = asyncio.get_running_loop()
                loop.create_task(self._async_client.aclose())
            except RuntimeError:
                asyncio.run(self._async_client.aclose())
            self._async_client = None

    async def aclose(self) -> None:
        """Async close of underlying transports."""
        if self._client is not None:
            self._client.close()
            self._client = None
        if self._async_client is not None:
            await self._async_client.aclose()
            self._async_client = None

    def __enter__(self) -> HTTPClient:
        """Enter sync context manager."""
        return self

    def __exit__(self, *args: Any) -> None:
        """Exit sync context manager and close transports."""
        self.close()

    async def __aenter__(self) -> HTTPClient:
        """Enter async context manager."""
        return self

    async def __aexit__(self, *args: Any) -> None:
        """Exit async context manager and close transports."""
        await self.aclose()

    @staticmethod
    def _raise_for_status(response: httpx.Response) -> None:
        """Raise a typed ``APIError`` for non-success HTTP responses."""
        if response.is_success:
            return
        body = response.text
        msg = f"{response.reason_phrase}: {body}"
        if response.status_code == 404:
            raise NotFoundError(response.status_code, msg, body)
        if response.status_code == 429:
            raise RateLimitError(response.status_code, msg, body)
        if response.status_code >= 500:
            raise ServerError(response.status_code, msg, body)
        raise APIError(response.status_code, msg, body)

    def _request(
        self,
        method: str,
        path: str,
        *,
        json: dict[str, Any] | list[Any] | None = None,
        data: Any = None,
        files: Any = None,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> httpx.Response:
        """Send a synchronous HTTP request.

        Args:
            method: HTTP method (GET, POST, …).
            path: URL path relative to the API base.
            json: JSON-serialisable request body.
            data: Form-encoded request body.
            files: Multipart file uploads.
            params: Query-string parameters.
            headers: Extra headers merged with defaults.
            timeout: Per-request timeout override.

        Returns:
            The validated ``httpx.Response``.

        Raises:
            APIError: On any non-success status code.
        """
        client = self._get_client()
        response = client.request(
            method,
            path,
            json=json,
            data=data,
            files=files,
            params=params,
            headers=headers,
            timeout=timeout or self._timeout,
        )
        self._raise_for_status(response)
        return response

    async def _arequest(
        self,
        method: str,
        path: str,
        *,
        json: dict[str, Any] | list[Any] | None = None,
        data: Any = None,
        files: Any = None,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> httpx.Response:
        """Send an asynchronous HTTP request.

        Args:
            method: HTTP method (GET, POST, …).
            path: URL path relative to the API base.
            json: JSON-serialisable request body.
            data: Form-encoded request body.
            files: Multipart file uploads.
            params: Query-string parameters.
            headers: Extra headers merged with defaults.
            timeout: Per-request timeout override.

        Returns:
            The validated ``httpx.Response``.

        Raises:
            APIError: On any non-success status code.
        """
        client = self._get_async_client()
        response = await client.request(
            method,
            path,
            json=json,
            data=data,
            files=files,
            params=params,
            headers=headers,
            timeout=timeout or self._timeout,
        )
        self._raise_for_status(response)
        return response

    def _stream_sse(
        self,
        method: str,
        path: str,
        *,
        json: dict[str, Any] | list[Any] | None = None,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> Iterator[SSEEvent]:
        """Stream Server-Sent Events synchronously.

        Args:
            method: HTTP method.
            path: URL path relative to the API base.
            json: JSON-serialisable request body.
            params: Query-string parameters.
            headers: Extra headers merged with defaults.

        Yields:
            Parsed ``SSEEvent`` instances.

        Raises:
            APIError: On any non-success status code.
        """
        client = self._get_client()
        with client.stream(
            method,
            path,
            json=json,
            params=params,
            headers={**(headers or {}), "Accept": "text/event-stream"},
            timeout=SSE_TIMEOUT,
        ) as response:
            self._raise_for_status(response)
            yield from _parse_sse_stream(response.iter_lines())

    async def _astream_sse(
        self,
        method: str,
        path: str,
        *,
        json: dict[str, Any] | list[Any] | None = None,
        params: dict[str, Any] | None = None,
        headers: dict[str, str] | None = None,
    ) -> AsyncIterator[SSEEvent]:
        """Stream Server-Sent Events asynchronously.

        Args:
            method: HTTP method.
            path: URL path relative to the API base.
            json: JSON-serialisable request body.
            params: Query-string parameters.
            headers: Extra headers merged with defaults.

        Yields:
            Parsed ``SSEEvent`` instances.

        Raises:
            APIError: On any non-success status code.
        """
        client = self._get_async_client()
        async with client.stream(
            method,
            path,
            json=json,
            params=params,
            headers={**(headers or {}), "Accept": "text/event-stream"},
            timeout=SSE_TIMEOUT,
        ) as response:
            self._raise_for_status(response)
            async for event in _parse_sse_stream_async(response.aiter_lines()):
                yield event


def _parse_sse_stream(lines: Iterator[str]) -> Iterator[SSEEvent]:
    """Parse a synchronous line iterator into SSE events."""
    event = SSEEvent()
    for line in lines:
        if line == "":
            if event.data:
                yield event
            event = SSEEvent()
            continue
        if line.startswith("event:"):
            event.event = line[len("event:"):].strip()
        elif line.startswith("data:"):
            event.data += line[len("data:"):].strip()
        elif line.startswith("id:"):
            event.id = line[len("id:"):].strip()
        elif line.startswith("retry:"):
            try:
                event.retry = int(line[len("retry:"):].strip())
            except ValueError:
                pass
    if event.data:
        yield event


async def _parse_sse_stream_async(lines: AsyncIterator[str]) -> AsyncIterator[SSEEvent]:
    """Parse an asynchronous line iterator into SSE events."""
    event = SSEEvent()
    async for line in lines:
        if line == "":
            if event.data:
                yield event
            event = SSEEvent()
            continue
        if line.startswith("event:"):
            event.event = line[len("event:"):].strip()
        elif line.startswith("data:"):
            event.data += line[len("data:"):].strip()
        elif line.startswith("id:"):
            event.id = line[len("id:"):].strip()
        elif line.startswith("retry:"):
            try:
                event.retry = int(line[len("retry:"):].strip())
            except ValueError:
                pass
    if event.data:
        yield event


__all__ = ["HTTPClient"]
