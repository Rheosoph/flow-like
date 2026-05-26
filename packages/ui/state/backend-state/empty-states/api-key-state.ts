import type {
	IApiKeyState,
	ITechnicalUser,
	ITechnicalUserCreateInput,
	ITechnicalUserCreateResult,
} from "@flow-like/flow-like-ui";

export class EmptyApiKeyState implements IApiKeyState {
	getApiKeys(_appId: string): Promise<ITechnicalUser[]> {
		throw new Error("Method not implemented.");
	}
	createApiKey(
		_appId: string,
		_input: ITechnicalUserCreateInput,
	): Promise<ITechnicalUserCreateResult> {
		throw new Error("Method not implemented.");
	}
	deleteApiKey(_appId: string, _keyId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
