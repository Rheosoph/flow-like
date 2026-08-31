import { type IRoleState, RolePermissions } from "@flow-like/flow-like-ui";
import type {
	IBackendRole,
	IOwnRole,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class RoleState implements IRoleState {
	constructor(private readonly backend: TauriBackend) {}

	async getRoles(appId: string): Promise<[string | undefined, IBackendRole[]]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		const roles = await fetcher<[string | undefined, IBackendRole[]]>(
			this.backend.profile,
			`apps/${appId}/roles`,
			undefined,
			this.backend.auth,
		);
		console.dir(roles);
		return roles;
	}
	async getOwnRole(appId: string): Promise<IOwnRole> {
		// A local-only app has no hub to ask and no team to belong to. Whoever
		// holds it is its only owner, so answer that here rather than letting
		// every caller special-case the offline device. `isLocalOnly`, not
		// `isOffline`: the latter also answers true for a hosted app whose
		// visibility this device has simply never cached.
		if (await this.backend.isLocalOnly(appId)) {
			return {
				role_id: "local",
				role_name: "Owner",
				permissions: Number(RolePermissions.Owner.toBigInt()),
				is_owner: true,
				can_leave: false,
			};
		}

		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher<IOwnRole>(
			this.backend.profile,
			`apps/${appId}/roles/me`,
			undefined,
			this.backend.auth,
		);
	}
	async deleteRole(appId: string, roleId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/roles/${roleId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}
	async makeRoleDefault(appId: string, roleId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/roles/${roleId}/default`,
			{
				method: "PUT",
			},
			this.backend.auth,
		);
	}
	async upsertRole(appId: string, role: IBackendRole): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/roles/${role.id}`,
			{
				method: "PUT",
				body: JSON.stringify(role, (key, value) =>
					typeof value === "bigint" ? Number(value) : value,
				),
			},
			this.backend.auth,
		);
	}
	async assignRole(appId: string, roleId: string, sub: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/roles/${roleId}/assign/${sub}`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}
}
