"""Mixin for querying and polling workflow execution status."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient, segment
from ._types import PollResult, RunStatus


def _poll_headers(headers: dict[str, str] | None, token: str) -> dict[str, str]:
    clean = {key: value for key, value in (headers or {}).items()
             if key.lower() not in ("authorization", "x-api-key", "cookie")}
    return {**clean, "Authorization": f"Bearer {token}"}


class ExecutionMixin(HTTPClient):
    """HTTP mixin that provides execution monitoring capabilities."""
    def get_run_status(self, run_id: str) -> RunStatus:
        """Retrieve the current status of a workflow run.

        Args:
            run_id: The unique identifier of the run.

        Returns:
            A ``RunStatus`` describing the run's current state.
        """
        resp = self._request("GET", f"/execution/run/{segment(run_id)}")
        data = resp.json()
        return RunStatus(
            run_id=data.get("run_id", run_id),
            status=data.get("status", "unknown"),
            result=data.get("result"),
            error=data.get("error"),
            raw=data,
        )

    async def aget_run_status(self, run_id: str) -> RunStatus:
        """Async version of ``get_run_status``.

        Args:
            run_id: The unique identifier of the run.

        Returns:
            A ``RunStatus`` describing the run's current state.
        """
        resp = await self._arequest("GET", f"/execution/run/{segment(run_id)}")
        data = resp.json()
        return RunStatus(
            run_id=data.get("run_id", run_id),
            status=data.get("status", "unknown"),
            result=data.get("result"),
            error=data.get("error"),
            raw=data,
        )

    def poll_execution(
        self,
        poll_token: str,
        after_sequence: int = -1,
        timeout: int = 30,
        **kwargs: Any,
    ) -> PollResult:
        """Long-poll for new execution events.

        Args:
            poll_token: Token returned by an async invocation.
            after_sequence: Only return events after this sequence number.
            timeout: Server-side long-poll timeout in seconds.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A ``PollResult`` with the collected events and a done flag.
        """
        resp = self._request(
            "GET",
            "/execution/poll",
            params={
                "after_sequence": after_sequence,
                "timeout": timeout,
            },
            headers=_poll_headers(kwargs.pop("headers", None), poll_token),
            authenticated=False,
            timeout=float(timeout + 5),
            **kwargs,
        )
        data = resp.json()
        return PollResult(
            events=data.get("events", []),
            done=data.get("status", "").lower() in {"completed", "failed", "cancelled", "timeout", "timed_out"},
            run_id=data.get("run_id", ""),
            status=data.get("status", "unknown"),
            progress=data.get("progress", 0),
            current_step=data.get("current_step"),
            error=data.get("error"),
            next_sequence=max((event.get("sequence", after_sequence) for event in data.get("events", [])), default=after_sequence),
            raw=data,
        )

    async def apoll_execution(
        self,
        poll_token: str,
        after_sequence: int = -1,
        timeout: int = 30,
        **kwargs: Any,
    ) -> PollResult:
        """Async version of ``poll_execution``.

        Args:
            poll_token: Token returned by an async invocation.
            after_sequence: Only return events after this sequence number.
            timeout: Server-side long-poll timeout in seconds.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A ``PollResult`` with the collected events and a done flag.
        """
        resp = await self._arequest(
            "GET",
            "/execution/poll",
            params={
                "after_sequence": after_sequence,
                "timeout": timeout,
            },
            headers=_poll_headers(kwargs.pop("headers", None), poll_token),
            authenticated=False,
            timeout=float(timeout + 5),
            **kwargs,
        )
        data = resp.json()
        return PollResult(
            events=data.get("events", []),
            done=data.get("status", "").lower() in {"completed", "failed", "cancelled", "timeout", "timed_out"},
            run_id=data.get("run_id", ""),
            status=data.get("status", "unknown"),
            progress=data.get("progress", 0),
            current_step=data.get("current_step"),
            error=data.get("error"),
            next_sequence=max((event.get("sequence", after_sequence) for event in data.get("events", [])), default=after_sequence),
            raw=data,
        )

    def list_runs(self, app_id: str, board_id: str, *, limit: int | None = None, offset: int | None = None, params: dict[str, Any] | None = None) -> Any:
        """List execution history, preserving server pagination and filters."""
        params = dict(params or {})
        if limit is not None:
            params["limit"] = limit
        if offset is not None:
            params["offset"] = offset
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs", params=params)

    async def alist_runs(self, app_id: str, board_id: str, *, limit: int | None = None, offset: int | None = None, params: dict[str, Any] | None = None) -> Any:
        """List execution history, preserving server pagination and filters."""
        params = dict(params or {})
        if limit is not None:
            params["limit"] = limit
        if offset is not None:
            params["offset"] = offset
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs", params=params)

    def get_run_logs(self, app_id: str, board_id: str, run_id: str, *, query: dict[str, Any] | None = None, offset: int = 0, limit: int = 50) -> Any:
        """Read structured run logs using the backend LogQuery filter."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/query", json={"run_id": run_id, "query": query or {}, "offset": offset, "limit": limit})

    async def aget_run_logs(self, app_id: str, board_id: str, run_id: str, *, query: dict[str, Any] | None = None, offset: int = 0, limit: int = 50) -> Any:
        """Read structured run logs using the backend LogQuery filter."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/query", json={"run_id": run_id, "query": query or {}, "offset": offset, "limit": limit})

    def query_run_logs(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/logs/query. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/query", json=body, params=params)

    async def aquery_run_logs(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/logs/query. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/query", json=body, params=params)

    def count_run_logs(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/logs/count. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/count", json=body, params=params)

    async def acount_run_logs(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/logs/count. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/count", json=body, params=params)

    def get_run_summary(self, app_id: str, board_id: str, run_id: str) -> Any:
        """Read a completed run's summary; the response can be null."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/summary", params={"run_id": run_id})

    async def aget_run_summary(self, app_id: str, board_id: str, run_id: str) -> Any:
        """Read a completed run's summary; the response can be null."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/logs/summary", params={"run_id": run_id})

    def get_run_payload(self, app_id: str, board_id: str, run_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/runs/{run_id}/payload. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/{segment(run_id)}/payload", params=params)

    async def aget_run_payload(self, app_id: str, board_id: str, run_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/runs/{run_id}/payload. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/{segment(run_id)}/payload", params=params)

    def get_execution_elements(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/elements. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/elements", params=params)

    async def aget_execution_elements(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/elements. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/elements", params=params)

    def get_element_demand(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/element-demand. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/element-demand", params=params)

    async def aget_element_demand(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/element-demand. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/element-demand", params=params)

    def cancel_run(self, run_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /execution/run/{run_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/execution/run/{segment(run_id)}", params=params)

    async def acancel_run(self, run_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /execution/run/{run_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/execution/run/{segment(run_id)}", params=params)

    def report_run(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/runs/report. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/report", json=body, params=params)

    async def areport_run(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/runs/report. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/report", json=body, params=params)

    def upload_run_logs(self, app_id: str, board_id: str, run_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/runs/{run_id}/logs. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/{segment(run_id)}/logs", json=body, params=params)

    async def aupload_run_logs(self, app_id: str, board_id: str, run_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/runs/{run_id}/logs. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/runs/{segment(run_id)}/logs", json=body, params=params)


__all__ = ["ExecutionMixin"]
