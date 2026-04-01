"""Mixin for application and health-check endpoints."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient
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
        resp = self._request("GET", f"/apps/{app_id}")
        data = resp.json()
        return App(id=data.get("id", app_id), name=data.get("name"), raw=data)

    async def aget_app(self, app_id: str) -> App:
        """Async version of get_app."""
        resp = await self._arequest("GET", f"/apps/{app_id}")
        data = resp.json()
        return App(id=data.get("id", app_id), name=data.get("name"), raw=data)

    def create_app(self, name: str, description: str | None = None) -> App:
        """Create a new app.

        Args:
            name: Display name for the app.
            description: Optional description.

        Returns:
            The newly created App.
        """
        body: dict[str, Any] = {"name": name}
        if description is not None:
            body["description"] = description
        resp = self._request("POST", "/apps", json=body)
        data = resp.json()
        return App(id=data.get("id", ""), name=data.get("name", name), raw=data)

    async def acreate_app(self, name: str, description: str | None = None) -> App:
        """Async version of create_app."""
        body: dict[str, Any] = {"name": name}
        if description is not None:
            body["description"] = description
        resp = await self._arequest("POST", "/apps", json=body)
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


__all__ = ["AppsMixin"]
