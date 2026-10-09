"""Mixin for application and health-check endpoints."""

from __future__ import annotations

from typing import Any
import time

from ._http import HTTPClient, segment
from ._types import App, HealthStatus


def _parse_app_item(item: Any) -> App:
    """Parse an app from the API response.

    The API returns Vec<(App, Option<Metadata>)>, which serializes as
    [[app_dict, metadata_or_null], ...]. Each item can be either a
    [app, meta] tuple-array or a plain dict.
    """
    if isinstance(item, list) and len(item) >= 1:
        app_data = item[0] if isinstance(item[0], dict) else {}
        meta = item[1] if len(item) > 1 and isinstance(item[1], dict) else {}
        name = meta.get("name") or app_data.get("name")
        return App(id=app_data.get("id", ""), name=name, raw={"app": app_data, "meta": meta})
    if isinstance(item, dict):
        return App(id=item.get("id", ""), name=item.get("name"), raw=item)
    return App(id="", name=None, raw={"value": item})


class AppsMixin(HTTPClient):
    """HTTP methods for managing apps and checking service health."""

    def list_apps(self) -> list[App]:
        """Return all apps visible to the current user.

        Returns:
            List of App objects.
        """
        resp = self._request("GET", "/apps")
        data = resp.json()
        items = data if isinstance(data, list) else data.get("apps", [])
        return [_parse_app_item(a) for a in items]

    async def alist_apps(self) -> list[App]:
        """Async version of list_apps."""
        resp = await self._arequest("GET", "/apps")
        data = resp.json()
        items = data if isinstance(data, list) else data.get("apps", [])
        return [_parse_app_item(a) for a in items]

    def get_app(self, app_id: str) -> App:
        """Fetch a single app by ID.

        Args:
            app_id: Unique identifier of the app.

        Returns:
            The matching App.
        """
        resp = self._request("GET", f"/apps/{segment(app_id)}")
        data = resp.json()
        return App(id=data.get("id", app_id), name=data.get("name"), raw=data)

    async def aget_app(self, app_id: str) -> App:
        """Async version of get_app."""
        resp = await self._arequest("GET", f"/apps/{segment(app_id)}")
        data = resp.json()
        return App(id=data.get("id", app_id), name=data.get("name"), raw=data)

    def create_app(self, name: str, description: str | None = None, *, bits: list[str] | None = None, language: str | None = None) -> App:
        """Create a new app.

        Args:
            name: Display name for the app.
            description: Optional description.

        Returns:
            The newly created App.
        """
        timestamp = {"secs_since_epoch": int(time.time()), "nanos_since_epoch": 0}
        body: dict[str, Any] = {"meta": {
            "name": name, "description": description or "", "tags": [],
            "preview_media": [], "created_at": timestamp, "updated_at": timestamp,
        }, "bits": bits or []}
        resp = self._request("PUT", "/apps/new", json=body, params={"language": language} if language else None)
        data = resp.json()
        return App(id=data.get("id", ""), name=data.get("name", name), raw=data)

    async def acreate_app(self, name: str, description: str | None = None, *, bits: list[str] | None = None, language: str | None = None) -> App:
        """Async version of create_app."""
        timestamp = {"secs_since_epoch": int(time.time()), "nanos_since_epoch": 0}
        body: dict[str, Any] = {"meta": {
            "name": name, "description": description or "", "tags": [],
            "preview_media": [], "created_at": timestamp, "updated_at": timestamp,
        }, "bits": bits or []}
        resp = await self._arequest("PUT", "/apps/new", json=body, params={"language": language} if language else None)
        data = resp.json()
        return App(id=data.get("id", ""), name=data.get("name", name), raw=data)

    def health(self) -> HealthStatus:
        """Check the health of the backend service.

        Returns:
            Current HealthStatus.
        """
        resp = self._request("GET", "/health")
        data = resp.json()
        return HealthStatus(healthy=data.get("healthy", True), raw=data)

    async def ahealth(self) -> HealthStatus:
        """Async version of health."""
        resp = await self._arequest("GET", "/health")
        data = resp.json()
        return HealthStatus(healthy=data.get("healthy", True), raw=data)

    def update_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}", json=body, params=params)

    async def aupdate_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}", json=body, params=params)

    def delete_app(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}", params=params)

    async def adelete_app(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}", params=params)

    def get_app_detail(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/detail. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/detail", params=params)

    async def aget_app_detail(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/detail. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/detail", params=params)

    def get_app_meta(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/meta. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/meta", params=params)

    async def aget_app_meta(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/meta. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/meta", params=params)

    def update_app_meta(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/meta. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/meta", json=body, params=params)

    async def aupdate_app_meta(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/meta. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/meta", json=body, params=params)

    def set_app_visibility(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/visibility. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/visibility", json=body, params=params)

    async def aset_app_visibility(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/visibility. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/visibility", json=body, params=params)

    def fork_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/fork. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/fork", json=body, params=params)

    async def afork_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/fork. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/fork", json=body, params=params)

    def list_publication_requests(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/publication. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/publication", params=params)

    async def alist_publication_requests(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/publication. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/publication", params=params)

    def publish_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/publication/request. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/publication/request", json=body, params=params)

    async def apublish_app(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/publication/request. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/publication/request", json=body, params=params)

    def get_app_appearance(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/settings/appearance. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/settings/appearance", params=params)

    async def aget_app_appearance(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/settings/appearance. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/settings/appearance", params=params)

    def update_app_appearance(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/settings/appearance. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/settings/appearance", json=body, params=params)

    async def aupdate_app_appearance(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/settings/appearance. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/settings/appearance", json=body, params=params)

    def get_app_forking(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/settings/forking. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/settings/forking", params=params)

    async def aget_app_forking(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/settings/forking. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/settings/forking", params=params)

    def update_app_forking(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/settings/forking. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/settings/forking", json=body, params=params)

    async def aupdate_app_forking(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/settings/forking. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/settings/forking", json=body, params=params)


__all__ = ["AppsMixin"]
