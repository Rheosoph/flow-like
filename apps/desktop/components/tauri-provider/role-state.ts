import type { IRoleState } from "@flow-like/flow-like-ui";
import type { IBackendRole } from "@flow-like/flow-like-ui/state/backend-state/types";
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
