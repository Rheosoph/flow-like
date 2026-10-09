"""Pages, routes, widgets, app connections, and installed packages."""
from __future__ import annotations
from typing import Any
from ._http import HTTPClient, segment


class ResourcesMixin(HTTPClient):
    """Pages, routes, widgets, app connections, and installed packages."""

    def list_pages(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/pages", params=params)

    async def alist_pages(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/pages", params=params)

    def get_page(self, app_id: str, page_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", params=params)

    async def aget_page(self, app_id: str, page_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", params=params)

    def upsert_page(self, app_id: str, page_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", json=body, params=params)

    async def aupsert_page(self, app_id: str, page_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", json=body, params=params)

    def delete_page(self, app_id: str, page_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", params=params)

    async def adelete_page(self, app_id: str, page_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/pages/{page_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/pages/{segment(page_id)}", params=params)

    def get_page_by_route(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/by-route. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/pages/by-route", params=params)

    async def aget_page_by_route(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/by-route. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/pages/by-route", params=params)

    def get_page_bootstrap(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/bootstrap. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/pages/bootstrap", params=params)

    async def aget_page_bootstrap(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/pages/bootstrap. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/pages/bootstrap", params=params)

    def list_routes(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/routes", params=params)

    async def alist_routes(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/routes", params=params)

    def get_route_by_path(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes/by-path. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/routes/by-path", params=params)

    async def aget_route_by_path(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes/by-path. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/routes/by-path", params=params)

    def get_default_route(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes/default. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/routes/default", params=params)

    async def aget_default_route(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/routes/default. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/routes/default", params=params)

    def create_route(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/routes. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/routes", json=body, params=params)

    async def acreate_route(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/routes. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/routes", json=body, params=params)

    def update_route(self, app_id: str, route_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/routes/{route_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/routes/{segment(route_id)}", json=body, params=params)

    async def aupdate_route(self, app_id: str, route_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/routes/{route_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/routes/{segment(route_id)}", json=body, params=params)

    def delete_route(self, app_id: str, route_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/routes/{route_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/routes/{segment(route_id)}", params=params)

    async def adelete_route(self, app_id: str, route_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/routes/{route_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/routes/{segment(route_id)}", params=params)

    def list_widgets(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/widgets", params=params)

    async def alist_widgets(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/widgets", params=params)

    def get_widget(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", params=params)

    async def aget_widget(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", params=params)

    def upsert_widget(self, app_id: str, widget_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", json=body, params=params)

    async def aupsert_widget(self, app_id: str, widget_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", json=body, params=params)

    def delete_widget(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", params=params)

    async def adelete_widget(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/widgets/{widget_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}", params=params)

    def get_widget_versions(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets/{widget_id}/versions. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}/versions", params=params)

    async def aget_widget_versions(self, app_id: str, widget_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/widgets/{widget_id}/versions. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}/versions", params=params)

    def create_widget_version(self, app_id: str, widget_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/widgets/{widget_id}/versions. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}/versions", json=body, params=params)

    async def acreate_widget_version(self, app_id: str, widget_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/widgets/{widget_id}/versions. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/widgets/{segment(widget_id)}/versions", json=body, params=params)

    def list_connections(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections", params=params)

    async def alist_connections(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections", params=params)

    def add_connection(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/connections", json=body, params=params)

    async def aadd_connection(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/connections", json=body, params=params)

    def request_connection(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/connections/request. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/connections/request", json=body, params=params)

    async def arequest_connection(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/connections/request. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/connections/request", json=body, params=params)

    def update_connection(self, app_id: str, connection_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/connections/{connection_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/connections/{segment(connection_id)}", json=body, params=params)

    async def aupdate_connection(self, app_id: str, connection_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/connections/{connection_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/connections/{segment(connection_id)}", json=body, params=params)

    def remove_connection(self, app_id: str, connection_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/connections/{connection_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/connections/{segment(connection_id)}", params=params)

    async def aremove_connection(self, app_id: str, connection_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/connections/{connection_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/connections/{segment(connection_id)}", params=params)

    def accept_connection(self, app_id: str, connection_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections/queue/{connection_id}. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/connections/queue/{segment(connection_id)}", json=body, params=params)

    async def aaccept_connection(self, app_id: str, connection_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections/queue/{connection_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/connections/queue/{segment(connection_id)}", json=body, params=params)

    def reject_connection(self, app_id: str, connection_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/connections/queue/{connection_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/connections/queue/{segment(connection_id)}", params=params)

    async def areject_connection(self, app_id: str, connection_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/connections/queue/{connection_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/connections/queue/{segment(connection_id)}", params=params)

    def list_accessible_apps(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/accessible. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections/accessible", params=params)

    async def alist_accessible_apps(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/accessible. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections/accessible", params=params)

    def get_connection_graph(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/graph. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections/graph", params=params)

    async def aget_connection_graph(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/graph. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections/graph", params=params)

    def get_remote_tables(self, app_id: str, target_app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/tables. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/tables", params=params)

    async def aget_remote_tables(self, app_id: str, target_app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/tables. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/tables", params=params)

    def get_remote_events(self, app_id: str, target_app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/events. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/events", params=params)

    async def aget_remote_events(self, app_id: str, target_app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/events. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/events", params=params)

    def get_remote_event_detail(self, app_id: str, target_app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/events/{event_id}/detail. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/events/{segment(event_id)}/detail", params=params)

    async def aget_remote_event_detail(self, app_id: str, target_app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/connections/{target_app_id}/events/{event_id}/detail. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/events/{segment(event_id)}/detail", params=params)

    def create_connection_token(self, app_id: str, target_app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections/{target_app_id}/token. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/token", json=body, params=params)

    async def acreate_connection_token(self, app_id: str, target_app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/connections/{target_app_id}/token. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/connections/{segment(target_app_id)}/token", json=body, params=params)

    def list_packages(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/packages", params=params)

    async def alist_packages(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/packages", params=params)

    def add_package(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/packages. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/packages", json=body, params=params)

    async def aadd_package(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/packages. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/packages", json=body, params=params)

    def update_package(self, app_id: str, package_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/packages/{package_id}. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/packages/{segment(package_id)}", json=body, params=params)

    async def aupdate_package(self, app_id: str, package_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/packages/{package_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/packages/{segment(package_id)}", json=body, params=params)

    def remove_package(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/packages/{package_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/packages/{segment(package_id)}", params=params)

    async def aremove_package(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/packages/{package_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/packages/{segment(package_id)}", params=params)

    def get_package_updates(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages/updates. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/packages/updates", params=params)

    async def aget_package_updates(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages/updates. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/packages/updates", params=params)

    def reactivate_package(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/packages/{package_id}/reactivate. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/packages/{segment(package_id)}/reactivate", params=params)

    async def areactivate_package(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/packages/{package_id}/reactivate. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/packages/{segment(package_id)}/reactivate", params=params)

    def get_package_patch_info(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages/{package_id}/patch-info. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/packages/{segment(package_id)}/patch-info", params=params)

    async def aget_package_patch_info(self, app_id: str, package_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/packages/{package_id}/patch-info. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/packages/{segment(package_id)}/patch-info", params=params)


__all__ = ["ResourcesMixin"]
