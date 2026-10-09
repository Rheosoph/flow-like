"""Low-level HTTP transport for the Flow-Like API.

Provides synchronous and asynchronous request helpers with SSE streaming,
automatic authentication, and structured error handling.
"""

from __future__ import annotations

from collections.abc import AsyncIterator, Iterator
from typing import Any
from urllib.parse import quote

import httpx

from ._auth import resolve_auth, resolve_base_url
from ._errors import APIError, NotFoundError, RateLimitError, ServerError
from ._types import SSEEvent

DEFAULT_TIMEOUT = 30.0
SSE_TIMEOUT = 300.0


def segment(value: str) -> str:
    """Encode one identifier without allowing it to change the request path."""
    if not value or value in (".", ".."):
        raise ValueError("A path identifier cannot be empty, '.' or '..'")
    return quote(str(value), safe="")


def response_json(response: httpx.Response) -> Any:
    """Return JSON, including null, or None for an empty successful response."""
    return response.json() if response.content else None


def _platform_path(path: str) -> None:
    url = httpx.URL(path)
    if url.is_absolute_url or url.host or path.startswith("//"):
        raise ValueError("Platform requests must use a relative API path")


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
        self._api_base = (
            self._base_url if self._base_url.endswith("/api/v1")
            else f"{self._base_url}/api/v1"
        )
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
        content: Any = None,
        authenticated: bool = True,
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
        if authenticated:
            _platform_path(path)
        client = self._get_client()
        request = client.build_request(
            method,
            path,
            json=json,
            data=data,
            files=files,
            content=content,
            params=params,
            headers={"x-flow-like-board-format": "2", **(headers or {})} if authenticated else headers,
            timeout=timeout if timeout is not None else self._timeout,
        )
        if not authenticated:
            request.headers.pop("Authorization", None)
            request.headers.pop("X-API-Key", None)
            request.headers.pop("Cookie", None)
            request.headers.update(headers or {})
        response = client.send(request)
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
        content: Any = None,
        authenticated: bool = True,
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
        if authenticated:
            _platform_path(path)
        client = self._get_async_client()
        request = client.build_request(
            method,
            path,
            json=json,
            data=data,
            files=files,
            content=content,
            params=params,
            headers={"x-flow-like-board-format": "2", **(headers or {})} if authenticated else headers,
            timeout=timeout if timeout is not None else self._timeout,
        )
        if not authenticated:
            request.headers.pop("Authorization", None)
            request.headers.pop("X-API-Key", None)
            request.headers.pop("Cookie", None)
            request.headers.update(headers or {})
        response = await client.send(request)
        self._raise_for_status(response)
        return response

    def _json(self, method: str, path: str, **kwargs: Any) -> Any:
        return response_json(self._request(method, path, **kwargs))

    async def _ajson(self, method: str, path: str, **kwargs: Any) -> Any:
        return response_json(await self._arequest(method, path, **kwargs))

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
        _platform_path(path)
        client = self._get_client()
        with client.stream(
            method,
            path,
            json=json,
            params=params,
            headers={"x-flow-like-board-format": "2", **(headers or {}), "Accept": "text/event-stream"},
            timeout=SSE_TIMEOUT,
        ) as response:
            if not response.is_success:
                response.read()
            self._raise_for_status(response)
            if response.headers.get("content-type", "").split(";", 1)[0].strip().lower() != "text/event-stream":
                response.read()
                raise APIError(response.status_code, "Expected a text/event-stream response", response.text)
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
        _platform_path(path)
        client = self._get_async_client()
        async with client.stream(
            method,
            path,
            json=json,
            params=params,
            headers={"x-flow-like-board-format": "2", **(headers or {}), "Accept": "text/event-stream"},
            timeout=SSE_TIMEOUT,
        ) as response:
            if not response.is_success:
                await response.aread()
            self._raise_for_status(response)
            if response.headers.get("content-type", "").split(";", 1)[0].strip().lower() != "text/event-stream":
                await response.aread()
                raise APIError(response.status_code, "Expected a text/event-stream response", response.text)
            async for event in _parse_sse_stream_async(response.aiter_lines()):
                yield event


class _SSEDecoder:
    def __init__(self) -> None:
        self.event = SSEEvent()
        self.data: list[str] = []

    def feed(self, line: str) -> SSEEvent | None:
        if not line:
            return self.finish()
        if line.startswith(":"):
            return None
        field, _, value = line.partition(":")
        if value.startswith(" "):
            value = value[1:]
        if field == "data":
            self.data.append(value)
        elif field == "event":
            self.event.event = value
        elif field == "id" and "\x00" not in value:
            self.event.id = value
        elif field == "retry" and value.isascii() and value.isdigit():
            self.event.retry = int(value)
        return None

    def finish(self) -> SSEEvent | None:
        event = self.event
        has_data = bool(self.data)
        event.data = "\n".join(self.data)
        self.event = SSEEvent(id=event.id)
        self.data = []
        return event if has_data else None


def _parse_sse_stream(lines: Iterator[str]) -> Iterator[SSEEvent]:
    decoder = _SSEDecoder()
    for line in lines:
        event = decoder.feed(line)
        if event is not None:
            yield event
    event = decoder.finish()
    if event is not None:
        yield event


async def _parse_sse_stream_async(lines: AsyncIterator[str]) -> AsyncIterator[SSEEvent]:
    decoder = _SSEDecoder()
    async for line in lines:
        event = decoder.feed(line)
        if event is not None:
            yield event
    event = decoder.finish()
    if event is not None:
        yield event


__all__ = ["HTTPClient"]
