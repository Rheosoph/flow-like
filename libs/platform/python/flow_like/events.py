"""Invoke events and manage their published definitions."""
from __future__ import annotations
from collections.abc import AsyncIterator, Iterator
from typing import Any
from ._http import HTTPClient, segment
from ._types import AsyncInvokeResult, SSEEvent


class EventsMixin(HTTPClient):
    """Event invocation and lifecycle operations."""

    def trigger_event(
        self, app_id: str, event_id: str, payload: Any = None, *,
        version: str | None = None, token: str | None = None,
        runtime_variables: dict[str, Any] | None = None,
        oauth_tokens: dict[str, Any] | None = None, profile_id: str | None = None,
        correlation: dict[str, str] | None = None,
        page_trigger: dict[str, Any] | None = None, variant: str | None = None,
        **kwargs: Any,
    ) -> Iterator[SSEEvent]:
        """Invoke an event and stream its server-sent events.

        The payload is nested in the invocation envelope. Use params for route options.
        """
        body = {key: value for key, value in {
            "payload": payload, "version": version, "token": token,
            "runtime_variables": runtime_variables, "oauth_tokens": oauth_tokens,
            "profile_id": profile_id, "correlation": correlation, "page_trigger": page_trigger,
        }.items() if value is not None}
        params = dict(kwargs.pop("params", None) or {})
        if variant is not None:
            params["__variant"] = variant
        return self._stream_sse(
            "POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/invoke",
            json=body, params=params, **kwargs,
        )

    def trigger_event_async(
        self, app_id: str, event_id: str, payload: Any = None, *,
        version: str | None = None, token: str | None = None,
        runtime_variables: dict[str, Any] | None = None,
        oauth_tokens: dict[str, Any] | None = None, profile_id: str | None = None,
        correlation: dict[str, str] | None = None,
        page_trigger: dict[str, Any] | None = None, variant: str | None = None,
        **kwargs: Any,
    ) -> AsyncInvokeResult:
        """Enqueue an event and return its polling token.

        The payload is nested in the invocation envelope. Use params for route options.
        """
        body = {key: value for key, value in {
            "payload": payload, "version": version, "token": token,
            "runtime_variables": runtime_variables, "oauth_tokens": oauth_tokens,
            "profile_id": profile_id, "correlation": correlation, "page_trigger": page_trigger,
        }.items() if value is not None}
        params = dict(kwargs.pop("params", None) or {})
        if variant is not None:
            params["__variant"] = variant
        data = self._json(
            "POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/invoke/async",
            json=body, params=params, **kwargs,
        )
        return AsyncInvokeResult(run_id=data["run_id"], poll_token=data["poll_token"], raw=data)

    async def atrigger_event(
        self, app_id: str, event_id: str, payload: Any = None, *,
        version: str | None = None, token: str | None = None,
        runtime_variables: dict[str, Any] | None = None,
        oauth_tokens: dict[str, Any] | None = None, profile_id: str | None = None,
        correlation: dict[str, str] | None = None,
        page_trigger: dict[str, Any] | None = None, variant: str | None = None,
        **kwargs: Any,
    ) -> AsyncIterator[SSEEvent]:
        """Invoke an event and stream its server-sent events.

        The payload is nested in the invocation envelope. Use params for route options.
        """
        body = {key: value for key, value in {
            "payload": payload, "version": version, "token": token,
            "runtime_variables": runtime_variables, "oauth_tokens": oauth_tokens,
            "profile_id": profile_id, "correlation": correlation, "page_trigger": page_trigger,
        }.items() if value is not None}
        params = dict(kwargs.pop("params", None) or {})
        if variant is not None:
            params["__variant"] = variant
        return self._astream_sse(
            "POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/invoke",
            json=body, params=params, **kwargs,
        )

    async def atrigger_event_async(
        self, app_id: str, event_id: str, payload: Any = None, *,
        version: str | None = None, token: str | None = None,
        runtime_variables: dict[str, Any] | None = None,
        oauth_tokens: dict[str, Any] | None = None, profile_id: str | None = None,
        correlation: dict[str, str] | None = None,
        page_trigger: dict[str, Any] | None = None, variant: str | None = None,
        **kwargs: Any,
    ) -> AsyncInvokeResult:
        """Enqueue an event and return its polling token.

        The payload is nested in the invocation envelope. Use params for route options.
        """
        body = {key: value for key, value in {
            "payload": payload, "version": version, "token": token,
            "runtime_variables": runtime_variables, "oauth_tokens": oauth_tokens,
            "profile_id": profile_id, "correlation": correlation, "page_trigger": page_trigger,
        }.items() if value is not None}
        params = dict(kwargs.pop("params", None) or {})
        if variant is not None:
            params["__variant"] = variant
        data = await self._ajson(
            "POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/invoke/async",
            json=body, params=params, **kwargs,
        )
        return AsyncInvokeResult(run_id=data["run_id"], poll_token=data["poll_token"], raw=data)

    def list_events(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events", params=params)

    async def alist_events(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events", params=params)

    def get_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}", params=params)

    async def aget_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}", params=params)

    def upsert_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}", json=body, params=params)

    async def aupsert_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}", json=body, params=params)

    def delete_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/events/{segment(event_id)}", params=params)

    async def adelete_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/events/{event_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/events/{segment(event_id)}", params=params)

    def get_event_versions(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/versions. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/versions", params=params)

    async def aget_event_versions(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/versions. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/versions", params=params)

    def get_event_timeline(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/timeline. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/timeline", params=params)

    async def aget_event_timeline(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/timeline. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/timeline", params=params)

    def get_event_runs(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/runs. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/runs", params=params)

    async def aget_event_runs(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/runs. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/runs", params=params)

    def validate_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/validate. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/validate", params=params)

    async def avalidate_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/validate. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/validate", params=params)

    def setup_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/setup. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/setup", json=body, params=params)

    async def asetup_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/setup. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/setup", json=body, params=params)

    def restore_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/restore. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/restore", json=body, params=params)

    async def arestore_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/restore. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/restore", json=body, params=params)

    def get_event_setups(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/setups. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/setups", params=params)

    async def aget_event_setups(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/setups. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/setups", params=params)

    def get_event_registrations(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/registrations. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/registrations", params=params)

    async def aget_event_registrations(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/registrations. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/registrations", params=params)

    def prerun_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/prerun. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/prerun", params=params)

    async def aprerun_event(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/prerun. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/prerun", params=params)

    def prerun_page_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/prerun. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/prerun", json=body, params=params)

    async def aprerun_page_event(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/prerun. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/prerun", json=body, params=params)

    def get_event_canary(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/canary/explain. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/explain", params=params)

    async def aget_event_canary(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/canary/explain. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/explain", params=params)

    def get_event_canary_stats(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/canary/stats. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/stats", params=params)

    async def aget_event_canary_stats(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/canary/stats. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/stats", params=params)

    def update_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/events/{event_id}/canary. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary", json=body, params=params)

    async def aupdate_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/events/{event_id}/canary. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary", json=body, params=params)

    def promote_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/canary/promote. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/promote", json=body, params=params)

    async def apromote_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/canary/promote. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/promote", json=body, params=params)

    def abort_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/canary/abort. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/abort", json=body, params=params)

    async def aabort_event_canary(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/events/{event_id}/canary/abort. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/events/{segment(event_id)}/canary/abort", json=body, params=params)

    def set_event_variants(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}/variants. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}/variants", json=body, params=params)

    async def aset_event_variants(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}/variants. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}/variants", json=body, params=params)

    def list_schedules(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /user/schedules. Bodies and query keys follow the REST API."""
        return self._json("GET", "/user/schedules", params=params)

    async def alist_schedules(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /user/schedules. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/user/schedules", params=params)

    def list_event_aliases(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/alias. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias", params=params)

    async def alist_event_aliases(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/alias. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias", params=params)

    def get_event_alias(self, app_id: str, event_id: str, slug: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", params=params)

    async def aget_event_alias(self, app_id: str, event_id: str, slug: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", params=params)

    def upsert_event_alias(self, app_id: str, event_id: str, slug: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", json=body, params=params)

    async def aupsert_event_alias(self, app_id: str, event_id: str, slug: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", json=body, params=params)

    def delete_event_alias(self, app_id: str, event_id: str, slug: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", params=params)

    async def adelete_event_alias(self, app_id: str, event_id: str, slug: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/events/{event_id}/alias/{slug}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/events/{segment(event_id)}/alias/{segment(slug)}", params=params)


__all__ = ["EventsMixin"]
