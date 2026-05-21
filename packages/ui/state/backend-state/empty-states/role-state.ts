import type { IBackendRole, IRoleState } from "@flow-like/flow-like-ui";

export class EmptyRoleState implements IRoleState {
	getRoles(appId: string): Promise<[string | undefined, IBackendRole[]]> {
		throw new Error("Method not implemented.");
	}
	deleteRole(appId: string, roleId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	makeRoleDefault(appId: string, roleId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	upsertRole(appId: string, role: IBackendRole): Promise<void> {
		throw new Error("Method not implemented.");
	}
	assignRole(appId: string, roleId: string, sub: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
