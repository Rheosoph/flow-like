import type {
	IApiKeyState,
	ITechnicalUser,
	ITechnicalUserCreateInput,
	ITechnicalUserCreateResult,
} from "@flow-like/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class ApiKeyState implements IApiKeyState {
	constructor(private readonly backend: TauriBackend) {}

	async getApiKeys(appId: string): Promise<ITechnicalUser[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		const result = await fetcher<
			{
				id: string;
				name: string;
				description?: string;
				role_id?: string;
				role_name?: string;
				role_permissions?: number;
				valid_until?: number;
				created_at: number;
			}[]
		>(
			this.backend.profile,
			`apps/${appId}/api`,
			{ method: "GET" },
			this.backend.auth,
		);

		return result.map((item) => ({
			...item,
			role_permissions: item.role_permissions
				? BigInt(item.role_permissions)
				: undefined,
		}));
	}

	async createApiKey(
		appId: string,
		input: ITechnicalUserCreateInput,
	): Promise<ITechnicalUserCreateResult> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher<ITechnicalUserCreateResult>(
			this.backend.profile,
			`apps/${appId}/api`,
			{
				method: "PUT",
				body: JSON.stringify(input),
			},
			this.backend.auth,
		);
	}

	async deleteApiKey(appId: string, keyId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/api/${keyId}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}
}
