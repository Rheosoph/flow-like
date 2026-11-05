import type {
	BotAppearance,
	BotApprovalDecision,
	IBot,
	IBotApp,
	IBotAvatar,
	IBotConnector,
	IConnectedAccount,
	ICreateBotConnectorRequest,
	IUpdateBotConnectorRequest,
	IBotMailAddress,
	IBotMailAvailability,
	IBotMailThread,
	IBotMailThreadView,
	IBotPermission,
	IBotPermissions,
	IBotPresence,
	IBotRestrictions,
	IBotSecretInfo,
	IBotState,
	IBotSkill,
	IBotTask,
	IBotToolGrant,
	IBotWebhook,
	IBotWorkspace,
	ICreateBotRequest,
	ICreateBotSkillRequest,
	ICreateBotTaskRequest,
	IIssuedBotWebhook,
	IUpdateBotRequest,
	IUpdateBotSkillRequest,
	IUpdateBotTaskRequest,
	IUpsertBotSecretRequest,
} from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
	constructApiUrl,
} from "./api-utils";

function botPath(botId: string): string {
	return `bots/${encodeURIComponent(botId)}`;
}

function toolGrantsPath(botId: string): string {
	return `${botPath(botId)}/tool-grants`;
}

function permissionPath(fromBotId: string, toBotId: string): string {
	return `${botPath(fromBotId)}/permissions/${encodeURIComponent(toBotId)}`;
}

function expectBot(response: unknown, operation: string): IBot {
	if (!isRecord(response)) {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as IBot;
}

function expectAvatar(response: unknown, botId: string): IBotAvatar {
	if (!isRecord(response)) {
		throw new Error(
			`Unexpected response while loading the avatar of bot ${botId}`,
		);
	}
	return { url: typeof response.url === "string" ? response.url : null };
}

function expectWorkspace(response: unknown, botId: string): IBotWorkspace {
	if (
		!isRecord(response) ||
		typeof response.app_id !== "string" ||
		!response.app_id
	) {
		throw new Error(
			`Unexpected response while opening the workspace of bot ${botId}`,
		);
	}
	return { app_id: response.app_id };
}

function expectIssued(response: unknown): IIssuedBotWebhook {
	if (
		!isRecord(response) ||
		!isRecord(response.webhook) ||
		typeof response.secret !== "string"
	) {
		throw new Error("Unexpected response while issuing a webhook");
	}
	return response as unknown as IIssuedBotWebhook;
}

function expectSkill(response: unknown): IBotSkill {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error("Unexpected response while saving a skill");
	}
	return response as unknown as IBotSkill;
}

function expectConnector(response: unknown): IBotConnector {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error("Unexpected response while saving a connector");
	}
	return response as unknown as IBotConnector;
}

function base64Of(bytes: Uint8Array): string {
	let binary = "";
	const chunk = 0x8000;
	for (let index = 0; index < bytes.length; index += chunk) {
		binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
	}
	return btoa(binary);
}

function expectTask(response: unknown): IBotTask {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error("Unexpected response while saving a task");
	}
	return response as unknown as IBotTask;
}

export class WebBotState implements IBotState {
	constructor(private readonly backend: WebBackendRef) {}

	/** Created on first use; the hub answers a repeat with the same app. */
	async getWorkspace(botId: string): Promise<IBotWorkspace> {
		return expectWorkspace(
			await apiPost<IBotWorkspace>(
				`${botPath(botId)}/workspace`,
				undefined,
				this.backend.auth,
			),
			botId,
		);
	}

	async getAvatar(botId: string): Promise<IBotAvatar> {
		return expectAvatar(
			await apiGet<IBotAvatar>(`${botPath(botId)}/avatar`, this.backend.auth),
			botId,
		);
	}

	async putAvatar(
		botId: string,
		appearance: BotAppearance,
		image: string,
	): Promise<IBotAvatar> {
		return expectAvatar(
			await apiPut<IBotAvatar>(
				`${botPath(botId)}/avatar`,
				{ appearance, image },
				this.backend.auth,
			),
			botId,
		);
	}

	async getPresence(): Promise<Record<string, IBotPresence>> {
		const response = await apiGet<Record<string, IBotPresence>>(
			"bots/presence",
			this.backend.auth,
		);
		return isRecord(response) ? response : {};
	}

	async listPermissions(botId: string): Promise<IBotPermissions> {
		const response = await apiGet<Partial<IBotPermissions>>(
			`${botPath(botId)}/permissions`,
			this.backend.auth,
		);
		return {
			granted: asArray(response?.granted),
			received: asArray(response?.received),
		};
	}

	async grantPermission(
		fromBotId: string,
		toBotId: string,
	): Promise<IBotPermission> {
		const response = await apiPut<IBotPermission>(
			permissionPath(fromBotId, toBotId),
			undefined,
			this.backend.auth,
		);
		if (!isRecord(response)) {
			throw new Error(
				`Unexpected response while letting bot ${fromBotId} ask ${toBotId}`,
			);
		}
		return response as unknown as IBotPermission;
	}

	async revokePermission(fromBotId: string, toBotId: string): Promise<void> {
		await apiDelete(permissionPath(fromBotId, toBotId), this.backend.auth);
	}

	async listToolGrants(botId: string): Promise<IBotToolGrant[]> {
		const response = await apiGet<{ grants?: IBotToolGrant[] }>(
			toolGrantsPath(botId),
			this.backend.auth,
		);
		return asArray(response?.grants);
	}

	async putToolGrant(
		botId: string,
		tool: string,
		scope: string,
	): Promise<IBotToolGrant> {
		const response = await apiPut<IBotToolGrant>(
			toolGrantsPath(botId),
			{ tool, scope },
			this.backend.auth,
		);
		if (!isRecord(response)) {
			throw new Error(
				`Unexpected response while allowing ${tool} (${scope}) for bot ${botId}`,
			);
		}
		return response as unknown as IBotToolGrant;
	}

	async deleteToolGrant(
		botId: string,
		tool: string,
		scope: string,
	): Promise<void> {
		await apiDelete(toolGrantsPath(botId), this.backend.auth, { tool, scope });
	}

	async answerApproval(
		botId: string,
		turnId: string,
		approvalId: string,
		decision: BotApprovalDecision,
	): Promise<void> {
		await apiPost(
			`${botPath(botId)}/turns/${encodeURIComponent(turnId)}/approval`,
			{ approval_id: approvalId, decision },
			this.backend.auth,
		);
	}

	async getMailAddress(botId: string): Promise<IBotMailAddress> {
		return apiGet<IBotMailAddress>(
			`${botPath(botId)}/email-address`,
			this.backend.auth,
		);
	}

	async checkMailAddress(
		botId: string,
		alias: string,
	): Promise<IBotMailAvailability> {
		return apiGet<IBotMailAvailability>(
			`${botPath(botId)}/email-address/check/${encodeURIComponent(alias)}`,
			this.backend.auth,
		);
	}

	async claimMailAddress(
		botId: string,
		alias: string,
	): Promise<IBotMailAddress> {
		return apiPut<IBotMailAddress>(
			`${botPath(botId)}/email-address`,
			{ alias },
			this.backend.auth,
		);
	}

	async releaseMailAddress(botId: string): Promise<void> {
		await apiDelete(`${botPath(botId)}/email-address`, this.backend.auth);
	}

	async listMailThreads(botId: string): Promise<IBotMailThread[]> {
		const response = await apiGet<{ threads?: IBotMailThread[] }>(
			`${botPath(botId)}/mail/threads`,
			this.backend.auth,
		);
		return asArray(response?.threads);
	}

	async getMailThread(
		botId: string,
		threadId: string,
	): Promise<IBotMailThreadView> {
		return apiGet<IBotMailThreadView>(
			`${botPath(botId)}/mail/threads/${encodeURIComponent(threadId)}`,
			this.backend.auth,
		);
	}

	async listWebhooks(botId: string): Promise<IBotWebhook[]> {
		const response = await apiGet<{ webhooks?: IBotWebhook[] }>(
			`${botPath(botId)}/webhooks`,
			this.backend.auth,
		);
		return asArray(response?.webhooks);
	}

	async createWebhook(
		botId: string,
		request: { name: string; instruction?: string },
	): Promise<IIssuedBotWebhook> {
		return expectIssued(
			await apiPost<IIssuedBotWebhook>(
				`${botPath(botId)}/webhooks`,
				request,
				this.backend.auth,
			),
		);
	}

	async rotateWebhook(
		botId: string,
		hookId: string,
	): Promise<IIssuedBotWebhook> {
		return expectIssued(
			await apiPost<IIssuedBotWebhook>(
				`${botPath(botId)}/webhooks/${encodeURIComponent(hookId)}/rotate`,
				undefined,
				this.backend.auth,
			),
		);
	}

	async deleteWebhook(botId: string, hookId: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/webhooks/${encodeURIComponent(hookId)}`,
			this.backend.auth,
		);
	}

	async webhookUrl(_botId: string, webhook: IBotWebhook): Promise<string> {
		return constructApiUrl(webhook.path.replace(/^\//, ""));
	}

	async listTasks(botId: string): Promise<IBotTask[]> {
		const response = await apiGet<{ tasks?: IBotTask[] }>(
			`${botPath(botId)}/tasks`,
			this.backend.auth,
		);
		return asArray(response?.tasks);
	}

	async createTask(
		botId: string,
		request: ICreateBotTaskRequest,
	): Promise<IBotTask> {
		return expectTask(
			await apiPost<IBotTask>(
				`${botPath(botId)}/tasks`,
				request,
				this.backend.auth,
			),
		);
	}

	async updateTask(
		botId: string,
		taskId: string,
		request: IUpdateBotTaskRequest,
	): Promise<IBotTask> {
		return expectTask(
			await apiPatch<IBotTask>(
				`${botPath(botId)}/tasks/${encodeURIComponent(taskId)}`,
				request,
				this.backend.auth,
			),
		);
	}

	async deleteTask(botId: string, taskId: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/tasks/${encodeURIComponent(taskId)}`,
			this.backend.auth,
		);
	}

	async listSkills(botId: string): Promise<IBotSkill[]> {
		const response = await apiGet<{ skills?: IBotSkill[] }>(
			`${botPath(botId)}/skills`,
			this.backend.auth,
		);
		return asArray(response?.skills);
	}

	async createSkill(
		botId: string,
		request: ICreateBotSkillRequest,
	): Promise<IBotSkill> {
		return expectSkill(
			await apiPost<IBotSkill>(
				`${botPath(botId)}/skills`,
				request,
				this.backend.auth,
			),
		);
	}

	async updateSkill(
		botId: string,
		skillId: string,
		request: IUpdateBotSkillRequest,
	): Promise<IBotSkill> {
		return expectSkill(
			await apiPatch<IBotSkill>(
				`${botPath(botId)}/skills/${encodeURIComponent(skillId)}`,
				request,
				this.backend.auth,
			),
		);
	}

	async approveSkill(botId: string, skillId: string): Promise<IBotSkill> {
		return expectSkill(
			await apiPost<IBotSkill>(
				`${botPath(botId)}/skills/${encodeURIComponent(skillId)}/approve`,
				undefined,
				this.backend.auth,
			),
		);
	}

	async disableSkill(botId: string, skillId: string): Promise<IBotSkill> {
		return expectSkill(
			await apiPost<IBotSkill>(
				`${botPath(botId)}/skills/${encodeURIComponent(skillId)}/disable`,
				undefined,
				this.backend.auth,
			),
		);
	}

	async deleteSkill(botId: string, skillId: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/skills/${encodeURIComponent(skillId)}`,
			this.backend.auth,
		);
	}

	async importSkill(botId: string, zip: Uint8Array): Promise<IBotSkill> {
		return expectSkill(
			await apiPost<IBotSkill>(
				`${botPath(botId)}/skills/import`,
				{ zip_base64: base64Of(zip) },
				this.backend.auth,
			),
		);
	}

	async listBots(): Promise<IBot[]> {
		const response = await apiGet<{ bots?: IBot[] }>("bots", this.backend.auth);
		return asArray(response?.bots);
	}

	async getBot(botId: string): Promise<IBot> {
		return expectBot(
			await apiGet<IBot>(botPath(botId), this.backend.auth),
			`load bot ${botId}`,
		);
	}

	async createBot(request: ICreateBotRequest): Promise<IBot> {
		return expectBot(
			await apiPost<IBot>("bots", request, this.backend.auth),
			"create a bot",
		);
	}

	async cloneBot(botId: string): Promise<IBot> {
		return expectBot(
			await apiPost<IBot>(
				`${botPath(botId)}/clone`,
				undefined,
				this.backend.auth,
			),
			`duplicate bot ${botId}`,
		);
	}

	async updateBot(botId: string, request: IUpdateBotRequest): Promise<IBot> {
		return expectBot(
			await apiPatch<IBot>(botPath(botId), request, this.backend.auth),
			`update bot ${botId}`,
		);
	}

	async deleteBot(botId: string): Promise<void> {
		await apiDelete(botPath(botId), this.backend.auth);
	}

	async listBotApps(botId: string): Promise<IBotApp[]> {
		const response = await apiGet<{ apps?: IBotApp[] }>(
			`${botPath(botId)}/apps`,
			this.backend.auth,
		);
		return asArray(response?.apps);
	}

	async attachBotApp(botId: string, appId: string): Promise<void> {
		await apiPut(
			`${botPath(botId)}/apps/${encodeURIComponent(appId)}`,
			undefined,
			this.backend.auth,
		);
	}

	async setBotAppRestrictions(
		botId: string,
		appId: string,
		restrictions: IBotRestrictions,
	): Promise<IBotRestrictions> {
		return apiPut<IBotRestrictions>(
			`${botPath(botId)}/apps/${encodeURIComponent(appId)}/restrictions`,
			restrictions,
			this.backend.auth,
		);
	}

	async detachBotApp(botId: string, appId: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/apps/${encodeURIComponent(appId)}`,
			this.backend.auth,
		);
	}

	async listBotSecrets(botId: string): Promise<IBotSecretInfo[]> {
		const response = await apiGet<{ secrets?: IBotSecretInfo[] }>(
			`${botPath(botId)}/secrets`,
			this.backend.auth,
		);
		return asArray(response?.secrets);
	}

	async upsertBotSecret(
		botId: string,
		name: string,
		request: IUpsertBotSecretRequest,
	): Promise<IBotSecretInfo> {
		const response = await apiPut<IBotSecretInfo>(
			`${botPath(botId)}/secrets/${encodeURIComponent(name)}`,
			request,
			this.backend.auth,
		);
		if (!isRecord(response)) {
			throw new Error(
				`Unexpected response while saving secret ${name} of bot ${botId}`,
			);
		}
		return response as unknown as IBotSecretInfo;
	}

	async deleteBotSecret(botId: string, name: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/secrets/${encodeURIComponent(name)}`,
			this.backend.auth,
		);
	}

	async listConnectors(botId: string): Promise<IBotConnector[]> {
		const response = await apiGet<{ connectors?: IBotConnector[] }>(
			`${botPath(botId)}/connectors`,
			this.backend.auth,
		);
		return asArray(response?.connectors);
	}

	async createConnector(
		botId: string,
		request: ICreateBotConnectorRequest,
	): Promise<IBotConnector> {
		return expectConnector(
			await apiPost<IBotConnector>(
				`${botPath(botId)}/connectors`,
				request,
				this.backend.auth,
			),
		);
	}

	async updateConnector(
		botId: string,
		connectorId: string,
		request: IUpdateBotConnectorRequest,
	): Promise<IBotConnector> {
		return expectConnector(
			await apiPatch<IBotConnector>(
				`${botPath(botId)}/connectors/${encodeURIComponent(connectorId)}`,
				request,
				this.backend.auth,
			),
		);
	}

	async deleteConnector(botId: string, connectorId: string): Promise<void> {
		await apiDelete(
			`${botPath(botId)}/connectors/${encodeURIComponent(connectorId)}`,
			this.backend.auth,
		);
	}

	async refreshConnector(
		botId: string,
		connectorId: string,
	): Promise<IBotConnector> {
		return expectConnector(
			await apiPost<IBotConnector>(
				`${botPath(botId)}/connectors/${encodeURIComponent(connectorId)}/refresh`,
				undefined,
				this.backend.auth,
			),
		);
	}

	async listConnectedAccounts(): Promise<IConnectedAccount[]> {
		const response = await apiGet<{
			connections?: {
				handle: string;
				provider_id?: string | null;
				state: string;
				scopes?: string[];
			}[];
		}>("oauth/connections", this.backend.auth);
		return asArray(response?.connections).map((connection) => ({
			handle: connection.handle,
			provider_id: connection.provider_id ?? null,
			state: connection.state,
			scopes: connection.scopes ?? [],
		}));
	}
}
