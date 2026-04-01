"""Mixin for querying and polling workflow execution status."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient
from ._types import PollResult, RunStatus


class ExecutionMixin(HTTPClient):
    """HTTP mixin that provides execution monitoring capabilities."""
    def get_run_status(self, run_id: str) -> RunStatus:
        """Retrieve the current status of a workflow run.

        Args:
            run_id: The unique identifier of the run.

        Returns:
            A ``RunStatus`` describing the run's current state.
        """
        resp = self._request("GET", f"/execution/run/{run_id}")
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
        resp = await self._arequest("GET", f"/execution/run/{run_id}")
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
        after_sequence: int = 0,
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
                "poll_token": poll_token,
                "after_sequence": after_sequence,
                "timeout": timeout,
            },
            timeout=float(timeout + 5),
            **kwargs,
        )
        data = resp.json()
        return PollResult(
            events=data.get("events", []),
            done=data.get("done", False),
            raw=data,
        )

    async def apoll_execution(
        self,
        poll_token: str,
        after_sequence: int = 0,
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
                "poll_token": poll_token,
                "after_sequence": after_sequence,
                "timeout": timeout,
            },
            timeout=float(timeout + 5),
            **kwargs,
        )
        data = resp.json()
        return PollResult(
            events=data.get("events", []),
            done=data.get("done", False),
            raw=data,
        )


__all__ = ["ExecutionMixin"]
