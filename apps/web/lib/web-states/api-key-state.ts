import type {
	IApiKeyState,
	ITechnicalUser,
	ITechnicalUserCreateInput,
	ITechnicalUserCreateResult,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, apiDelete, apiGet, apiPut } from "./api-utils";

export class WebApiKeyState implements IApiKeyState {
	constructor(private readonly backend: WebBackendRef) {}

	async getApiKeys(appId: string): Promise<ITechnicalUser[]> {
		try {
			const result = await apiGet<
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
			>(`apps/${appId}/api`, this.backend.auth);

			return result.map((item) => ({
				...item,
				role_permissions: item.role_permissions
					? BigInt(item.role_permissions)
					: undefined,
			}));
		} catch {
			return [];
		}
	}

	async createApiKey(
		appId: string,
		input: ITechnicalUserCreateInput,
	): Promise<ITechnicalUserCreateResult> {
		return await apiPut<ITechnicalUserCreateResult>(
			`apps/${appId}/api`,
			input,
			this.backend.auth,
		);
	}

	async deleteApiKey(appId: string, keyId: string): Promise<void> {
		await apiDelete(`apps/${appId}/api/${keyId}`, this.backend.auth);
	}
}
