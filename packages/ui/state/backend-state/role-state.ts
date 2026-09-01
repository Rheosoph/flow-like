import type { IBackendRole, IOwnRole } from "./types";

export interface IRoleState {
	getRoles(appId: string): Promise<[string | undefined, IBackendRole[]]>;
	/**
	 * The caller's own role. Readable by every member, unlike `getRoles`,
	 * which needs `ReadRoles`.
	 */
	getOwnRole(appId: string): Promise<IOwnRole>;
	deleteRole(appId: string, roleId: string): Promise<void>;
	makeRoleDefault(appId: string, roleId: string): Promise<void>;
	upsertRole(appId: string, role: IBackendRole): Promise<void>;
	assignRole(appId: string, roleId: string, sub: string): Promise<void>;
}
