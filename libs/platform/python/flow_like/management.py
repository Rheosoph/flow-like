"""App membership and hub device REST management. Encrypted agent commands use a separate protocol."""
from __future__ import annotations
from typing import Any
from ._http import HTTPClient, segment


class ManagementMixin(HTTPClient):
    """App membership and hub device REST management. Encrypted agent commands use a separate protocol."""

    def list_roles(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/roles. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/roles", params=params)

    async def alist_roles(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/roles. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/roles", params=params)

    def get_own_role(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/roles/me. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/roles/me", params=params)

    async def aget_own_role(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/roles/me. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/roles/me", params=params)

    def upsert_role(self, app_id: str, role_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/roles/{role_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/roles/{segment(role_id)}", json=body, params=params)

    async def aupsert_role(self, app_id: str, role_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/roles/{role_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/roles/{segment(role_id)}", json=body, params=params)

    def delete_role(self, app_id: str, role_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/roles/{role_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/roles/{segment(role_id)}", params=params)

    async def adelete_role(self, app_id: str, role_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/roles/{role_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/roles/{segment(role_id)}", params=params)

    def make_role_default(self, app_id: str, role_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/roles/{role_id}/default. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/roles/{segment(role_id)}/default", params=params)

    async def amake_role_default(self, app_id: str, role_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/roles/{role_id}/default. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/roles/{segment(role_id)}/default", params=params)

    def assign_role(self, app_id: str, role_id: str, sub: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/roles/{role_id}/assign/{sub}. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/roles/{segment(role_id)}/assign/{segment(sub)}", params=params)

    async def aassign_role(self, app_id: str, role_id: str, sub: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/roles/{role_id}/assign/{sub}. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/roles/{segment(role_id)}/assign/{segment(sub)}", params=params)

    def get_team(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/team", params=params)

    async def aget_team(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/team", params=params)

    def remove_team_member(self, app_id: str, sub: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/{sub}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/team/{segment(sub)}", params=params)

    async def aremove_team_member(self, app_id: str, sub: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/{sub}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/team/{segment(sub)}", params=params)

    def invite_user(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/invite. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/team/invite", json=body, params=params)

    async def ainvite_user(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/invite. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/team/invite", json=body, params=params)

    def list_invites(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/invites. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/team/invites", params=params)

    async def alist_invites(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/invites. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/team/invites", params=params)

    def revoke_invite(self, app_id: str, invite_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/invites/{invite_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/team/invites/{segment(invite_id)}", params=params)

    async def arevoke_invite(self, app_id: str, invite_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/invites/{invite_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/team/invites/{segment(invite_id)}", params=params)

    def create_invite_link(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/link. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/team/link", json=body, params=params)

    async def acreate_invite_link(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/link. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/team/link", json=body, params=params)

    def list_invite_links(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/link. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/team/link", params=params)

    async def alist_invite_links(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/link. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/team/link", params=params)

    def delete_invite_link(self, app_id: str, link_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/link/{link_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/team/link/{segment(link_id)}", params=params)

    async def adelete_invite_link(self, app_id: str, link_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/link/{link_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/team/link/{segment(link_id)}", params=params)

    def join_invite_link(self, app_id: str, token: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/team/link/join/{token}. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/team/link/join/{segment(token)}", params=params)

    async def ajoin_invite_link(self, app_id: str, token: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/team/link/join/{token}. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/team/link/join/{segment(token)}", params=params)

    def list_join_requests(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/queue. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/team/queue", params=params)

    async def alist_join_requests(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/team/queue. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/team/queue", params=params)

    def request_join(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/queue. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/team/queue", json=body, params=params)

    async def arequest_join(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/team/queue. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/team/queue", json=body, params=params)

    def accept_join_request(self, app_id: str, request_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/team/queue/{request_id}. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/team/queue/{segment(request_id)}", params=params)

    async def aaccept_join_request(self, app_id: str, request_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/team/queue/{request_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/team/queue/{segment(request_id)}", params=params)

    def reject_join_request(self, app_id: str, request_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/queue/{request_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/team/queue/{segment(request_id)}", params=params)

    async def areject_join_request(self, app_id: str, request_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/team/queue/{request_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/team/queue/{segment(request_id)}", params=params)

    def create_api_key(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/api. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/api", json=body, params=params)

    async def acreate_api_key(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/api. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/api", json=body, params=params)

    def list_api_keys(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/api. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/api", params=params)

    async def alist_api_keys(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/api. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/api", params=params)

    def delete_api_key(self, app_id: str, key_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/api/{key_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/api/{segment(key_id)}", params=params)

    async def adelete_api_key(self, app_id: str, key_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/api/{key_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/api/{segment(key_id)}", params=params)

    def list_devices(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices", params=params)

    async def alist_devices(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices", params=params)

    def get_device_setup(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/setup. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/setup", params=params)

    async def aget_device_setup(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/setup. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/setup", params=params)

    def get_device_usage(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/usage. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/usage", params=params)

    async def aget_device_usage(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/usage. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/usage", params=params)

    def get_device(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}", params=params)

    async def aget_device(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}", params=params)

    def rename_device(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /devices/{device_id}. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/devices/{segment(device_id)}", json=body, params=params)

    async def arename_device(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /devices/{device_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/devices/{segment(device_id)}", json=body, params=params)

    def revoke_device(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}", params=params)

    async def arevoke_device(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}", params=params)

    def list_device_enrollments(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/enrollments. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/enrollments", params=params)

    async def alist_device_enrollments(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/enrollments. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/enrollments", params=params)

    def create_device_enrollment(self, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/enrollments. Bodies and query keys follow the REST API."""
        return self._json("POST", "/devices/enrollments", json=body, params=params)

    async def acreate_device_enrollment(self, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/enrollments. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", "/devices/enrollments", json=body, params=params)

    def cancel_device_enrollment(self, enrollment_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/enrollments/{enrollment_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/enrollments/{segment(enrollment_id)}", params=params)

    async def acancel_device_enrollment(self, enrollment_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/enrollments/{enrollment_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/enrollments/{segment(enrollment_id)}", params=params)

    def get_device_identity(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/identity. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/identity", params=params)

    async def aget_device_identity(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/identity. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/identity", params=params)

    def get_device_policy(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/management/policy. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/management/policy", params=params)

    async def aget_device_policy(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/management/policy. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/management/policy", params=params)

    def set_device_policy(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/management/policy. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/{segment(device_id)}/management/policy", json=body, params=params)

    async def aset_device_policy(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/management/policy. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/{segment(device_id)}/management/policy", json=body, params=params)

    def get_device_access(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/management/my-access. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/management/my-access", params=params)

    async def aget_device_access(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/management/my-access. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/management/my-access", params=params)

    def signal_device_controller(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/signaling/controller. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/signaling/controller", json=body, params=params)

    async def asignal_device_controller(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/signaling/controller. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/signaling/controller", json=body, params=params)

    def list_controller_vaults(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/controller-vaults. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/controller-vaults", params=params)

    async def alist_controller_vaults(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/controller-vaults. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/controller-vaults", params=params)

    def get_controller_vault(self, vault_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/controller-vaults/{vault_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/controller-vaults/{segment(vault_id)}", params=params)

    async def aget_controller_vault(self, vault_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/controller-vaults/{vault_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/controller-vaults/{segment(vault_id)}", params=params)

    def put_controller_vault(self, vault_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/controller-vaults/{vault_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/controller-vaults/{segment(vault_id)}", json=body, params=params)

    async def aput_controller_vault(self, vault_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/controller-vaults/{vault_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/controller-vaults/{segment(vault_id)}", json=body, params=params)

    def get_device_archive_usage(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/archive-usage. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/archive-usage", params=params)

    async def aget_device_archive_usage(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/archive-usage. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/archive-usage", params=params)

    def list_device_archives(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/archives. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/archives", params=params)

    async def alist_device_archives(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/archives. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/archives", params=params)

    def create_device_archive(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/archives. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/archives", json=body, params=params)

    async def acreate_device_archive(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/archives. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/archives", json=body, params=params)

    def get_device_archive(self, device_id: str, archive_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/archives/{archive_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/archives/{segment(archive_id)}", params=params)

    async def aget_device_archive(self, device_id: str, archive_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/archives/{archive_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/archives/{segment(archive_id)}", params=params)

    def get_device_inventory(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/inventory/{key}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/inventory/{segment(key)}", params=params)

    async def aget_device_inventory(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/inventory/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/inventory/{segment(key)}", params=params)

    def put_device_inventory(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/inventory/{key}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/{segment(device_id)}/inventory/{segment(key)}", json=body, params=params)

    async def aput_device_inventory(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/inventory/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/{segment(device_id)}/inventory/{segment(key)}", json=body, params=params)

    def list_fleet_certificates(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/certificate-inventory. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/certificate-inventory", params=params)

    async def alist_fleet_certificates(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/certificate-inventory. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/certificate-inventory", params=params)

    def get_device_certificates(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-inventory. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/certificate-inventory", params=params)

    async def aget_device_certificates(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-inventory. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/certificate-inventory", params=params)

    def put_device_certificates(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/certificate-inventory. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/{segment(device_id)}/certificate-inventory", json=body, params=params)

    async def aput_device_certificates(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/certificate-inventory. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/{segment(device_id)}/certificate-inventory", json=body, params=params)

    def get_device_certificate_notices(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-notices. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/certificate-notices", params=params)

    async def aget_device_certificate_notices(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-notices. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/certificate-notices", params=params)

    def get_device_certificate_mutes(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/certificate-notices/mute", params=params)

    async def aget_device_certificate_mutes(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/certificate-notices/mute", params=params)

    def mute_device_certificate_notices(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/{segment(device_id)}/certificate-notices/mute", json=body, params=params)

    async def amute_device_certificate_notices(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/{segment(device_id)}/certificate-notices/mute", json=body, params=params)

    def unmute_device_certificate_notices(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}/certificate-notices/mute", params=params)

    async def aunmute_device_certificate_notices(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/certificate-notices/mute. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}/certificate-notices/mute", params=params)

    def test_device_certificate_notice(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/certificate-notices/test. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/certificate-notices/test", json=body, params=params)

    async def atest_device_certificate_notice(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/certificate-notices/test. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/certificate-notices/test", json=body, params=params)

    def get_fleet_reader(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", params=params)

    async def aget_fleet_reader(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", params=params)

    def put_fleet_reader(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", json=body, params=params)

    async def aput_fleet_reader(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", json=body, params=params)

    def delete_fleet_reader(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", json=body, params=params)

    async def adelete_fleet_reader(self, device_id: str, key: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/fleet/readers/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}/fleet/readers/{segment(key)}", json=body, params=params)

    def get_fleet_recipients(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/fleet/recipients. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/fleet/recipients", json=body, params=params)

    async def aget_fleet_recipients(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/fleet/recipients. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/fleet/recipients", json=body, params=params)

    def put_fleet_snapshot(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/fleet/snapshots. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/fleet/snapshots", json=body, params=params)

    async def aput_fleet_snapshot(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/fleet/snapshots. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/fleet/snapshots", json=body, params=params)

    def get_fleet_snapshot(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/fleet/snapshots/{key}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/fleet/snapshots/{segment(key)}", params=params)

    async def aget_fleet_snapshot(self, device_id: str, key: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/fleet/snapshots/{key}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/fleet/snapshots/{segment(key)}", params=params)

    def get_device_resource_summary(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/resource-summary. Bodies and query keys follow the REST API."""
        return self._json("GET", "/devices/resource-summary", params=params)

    async def aget_device_resource_summary(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/resource-summary. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/devices/resource-summary", params=params)

    def list_device_resource_grants(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/resource-grants", params=params)

    async def alist_device_resource_grants(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/resource-grants", params=params)

    def create_device_resource_grant(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/resource-grants. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/resource-grants", json=body, params=params)

    async def acreate_device_resource_grant(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/resource-grants. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/resource-grants", json=body, params=params)

    def get_device_resource_grant(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}", params=params)

    async def aget_device_resource_grant(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}", params=params)

    def revoke_device_resource_grant(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/resource-grants/{grant_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}", params=params)

    async def arevoke_device_resource_grant(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/resource-grants/{grant_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}", params=params)

    def get_resource_grant_billing(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}/billing. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing", params=params)

    async def aget_resource_grant_billing(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}/billing. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing", params=params)

    def approve_resource_grant_billing(self, device_id: str, grant_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/resource-grants/{grant_id}/billing. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing", json=body, params=params)

    async def aapprove_resource_grant_billing(self, device_id: str, grant_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/resource-grants/{grant_id}/billing. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing", json=body, params=params)

    def get_billing_eligibility(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}/billing/eligibility. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing/eligibility", params=params)

    async def aget_billing_eligibility(self, device_id: str, grant_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/resource-grants/{grant_id}/billing/eligibility. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/resource-grants/{segment(grant_id)}/billing/eligibility", params=params)

    def list_device_billing_grants(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/billing-grants", params=params)

    async def alist_device_billing_grants(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/billing-grants", params=params)

    def get_device_billing_grant(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants/{billing_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}", params=params)

    async def aget_device_billing_grant(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants/{billing_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}", params=params)

    def revoke_device_billing_grant(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/billing-grants/{billing_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}", params=params)

    async def arevoke_device_billing_grant(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/billing-grants/{billing_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}", params=params)

    def get_device_billing_usage(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants/{billing_id}/usage. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}/usage", params=params)

    async def aget_device_billing_usage(self, device_id: str, billing_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/billing-grants/{billing_id}/usage. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/billing-grants/{segment(billing_id)}/usage", params=params)

    def list_device_instances(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/instances. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/devices/{segment(device_id)}/instances", params=params)

    async def alist_device_instances(self, device_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /devices/{device_id}/instances. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/devices/{segment(device_id)}/instances", params=params)

    def register_device_instance(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/instances. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/devices/{segment(device_id)}/instances", json=body, params=params)

    async def aregister_device_instance(self, device_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /devices/{device_id}/instances. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/devices/{segment(device_id)}/instances", json=body, params=params)

    def retire_device_instance(self, device_id: str, instance_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/instances/{instance_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/devices/{segment(device_id)}/instances/{segment(instance_id)}", params=params)

    async def aretire_device_instance(self, device_id: str, instance_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /devices/{device_id}/instances/{instance_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/devices/{segment(device_id)}/instances/{segment(instance_id)}", params=params)

    def get_device_metadata(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/device-metadata. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/device-metadata", params=params)

    async def aget_device_metadata(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/device-metadata. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/device-metadata", params=params)

    def get_device_placements(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/device-placements. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/device-placements", params=params)

    async def aget_device_placements(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/device-placements. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/device-placements", params=params)

    def release_device_schedule(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/device-schedules/{event_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/device-schedules/{segment(event_id)}", json=body, params=params)

    async def arelease_device_schedule(self, app_id: str, event_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/device-schedules/{event_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/device-schedules/{segment(event_id)}", json=body, params=params)

    def give_back_device_schedule(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/device-schedules/{event_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/device-schedules/{segment(event_id)}", params=params)

    async def agive_back_device_schedule(self, app_id: str, event_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/device-schedules/{event_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/device-schedules/{segment(event_id)}", params=params)


__all__ = ["ManagementMixin"]
