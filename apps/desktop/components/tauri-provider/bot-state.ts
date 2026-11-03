import type {
	BotApprovalDecision,
	IBot,
	IBotApp,
	IBotRestrictions,
	IBotComputerHost,
	IBotEncryptionHost,
	IBotMailAddress,
	IBotMailAvailability,
	IBotMailThread,
	IBotMailThreadView,
	IBotPermission,
	IBotPermissions,
	IBotPresence,
	IBotSecretInfo,
	IBotState,
	IBotTask,
	IBotToolGrant,
	IBotTurnHost,
	IBotWebhook,
	IBotWorkspace,
	ICreateBotRequest,
	ICreateBotTaskRequest,
	IIssuedBotWebhook,
	ILocalBotHost,
	IUpdateBotRequest,
	IUpdateBotTaskRequest,
	IUpsertBotSecretRequest,
} from "@flow-like/flow-like-ui";
import { getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { IAppVisibility } from "@flow-like/flow-like-ui/lib/schema/app/app";
import { invoke } from "@tauri-apps/api/core";
import { del, get, patch, post, put } from "../../lib/api";
import { appsDB } from "../../lib/apps-db";
import {
	AccountListReads,
	type IAccountRoute,
	accountSession,
	botPath,
	listAccountBots,
} from "../../lib/local-bots/account";
import {
	LocalStoreError,
	errorCode,
	localBots,
	localPermissions,
	localTasks,
	localToolGrants,
} from "../../lib/local-bots/commands";
import { BotComputerHost } from "../../lib/local-bots/computer";
import { BotEncryptionHost } from "../../lib/local-bots/encryption";
import { mergeBots } from "../../lib/local-bots/merge";
import { LocalBotMover } from "../../lib/local-bots/move";
import { idsOf, localRegistries } from "../../lib/local-bots/registry";
import { BotTurnHost } from "../../lib/local-bots/turns";
import type { TauriBackend } from "../tauri-provider";

function appPath(botId: string, appId: string): string {
	return `${botPath(botId)}/apps/${encodeURIComponent(appId)}`;
}

function permissionPath(fromBotId: string, toBotId: string): string {
	return `${botPath(fromBotId)}/permissions/${encodeURIComponent(toBotId)}`;
}

function toolGrantsPath(botId: string): string {
	return `${botPath(botId)}/tool-grants`;
}

function secretPath(botId: string, name: string): string {
	return `${botPath(botId)}/secrets/${encodeURIComponent(name)}`;
}

function expectBot(response: unknown, operation: string): IBot {
	if (!isRecord(response)) {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as IBot;
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

function expectTask(response: unknown): IBotTask {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error("Unexpected response while saving a task");
	}
	return response as unknown as IBotTask;
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

/** A failing local store must not hide the account's bots. */
function localBotsBesideAccount(): Promise<IBot[] | undefined> {
	return localBots.list().catch((error) => {
		console.warn("[LocalBots] Listing account bots only:", error);
		return undefined;
	});
}

/**
 * Signed out, every bot lives on this computer. Signed in, new bots go to the
 * account and lists show both; a bot on this computer is served from here
 * until it moves, and its local id then stands for its account id.
 */
export class BotState implements IBotState {
	readonly local: ILocalBotHost;
	readonly turns: IBotTurnHost;
	readonly encryption: IBotEncryptionHost = new BotEncryptionHost();
	readonly computer: IBotComputerHost = new BotComputerHost();
	private readonly accountBots = new AccountListReads<IBot>();

	constructor(private readonly backend: TauriBackend) {
		this.local = new LocalBotMover(backend);
		this.turns = new BotTurnHost(
			async (botId) => (await this.accountFor(botId)) === undefined,
			async (turn) => {
				const account = await this.accountFor(turn.bot_id);
				if (!account) return;
				await post<unknown>(
					account.profile,
					`${botPath(account.id)}/turns/${encodeURIComponent(turn.id)}/cancel`,
					undefined,
					account.auth,
				);
			},
		);
	}

	private get ids() {
		return localRegistries(this.backend).bots;
	}

	/** Where calls about this bot go; undefined when it lives on this computer. */
	private async accountFor(botId: string): Promise<IAccountRoute | undefined> {
		const session = accountSession(this.backend);
		if (!session || (await this.ids.isLocal(botId, session.sub))) return;
		return { ...session, id: this.ids.accountId(botId, session.sub) };
	}

	private here<T>(botId: string, call: () => Promise<T>): Promise<T> {
		return this.ids.onLocal(botId, call);
	}

	async listBots(): Promise<IBot[]> {
		const session = accountSession(this.backend);
		if (!session) {
			const bots = await localBots.list();
			this.ids.setLocal(idsOf(bots));
			return bots;
		}
		const [remote, local] = await Promise.all([
			this.accountBots.read(session.sub, () => listAccountBots(session)),
			localBotsBesideAccount(),
		]);
		if (local) this.ids.setLocal(idsOf(local));
		this.ids.addRemote(session.sub, idsOf(remote));
		return mergeBots(remote, local ?? []);
	}

	async getBot(botId: string): Promise<IBot> {
		const account = await this.accountFor(botId);
		if (!account) {
			const bot = await this.here(botId, () => localBots.get(botId));
			if (bot) return bot;
			this.ids.forgetLocal(botId);
			throw new Error(`Bot ${botId} was not found on this computer`);
		}
		return expectBot(
			await get<IBot>(account.profile, botPath(account.id), account.auth),
			`load bot ${botId}`,
		);
	}

	/**
	 * Created on first use; a repeat returns the same app. A local bot's workspace is an app on
	 * this computer. An account bot's lives on the hub and is registered here as Private, since
	 * app ids this computer doesn't know are read as offline and would send its files and
	 * tables to the local store.
	 */
	async getWorkspace(botId: string): Promise<IBotWorkspace> {
		const account = await this.accountFor(botId);
		if (!account) return this.here(botId, () => localBots.workspace(botId));
		const workspace = expectWorkspace(
			await post<IBotWorkspace>(
				account.profile,
				`${botPath(account.id)}/workspace`,
				undefined,
				account.auth,
			),
			botId,
		);
		await appsDB.visibility.put({
			appId: workspace.app_id,
			visibility: IAppVisibility.Private,
		});
		return workspace;
	}

	async createBot(request: ICreateBotRequest): Promise<IBot> {
		const session = accountSession(this.backend);
		if (!session) {
			const bot = await localBots.create(request);
			this.ids.addLocal(bot.id);
			return bot;
		}
		const bot = expectBot(
			await post<IBot>(session.profile, "bots", request, session.auth),
			"create a bot",
		);
		this.ids.addRemote(session.sub, [bot.id]);
		return bot;
	}

	/** The copy lives where the bot lives: on this computer, or in the account. */
	async cloneBot(botId: string): Promise<IBot> {
		const account = await this.accountFor(botId);
		if (!account) {
			const copy = await this.here(botId, async () => {
				try {
					return await invoke<unknown>("local_bots_clone", { botId });
				} catch (error) {
					throw new LocalStoreError(
						`Could not duplicate local bot ${botId}: ${getErrorMessage(error)}`,
						errorCode(error),
						error,
					);
				}
			});
			const bot = expectBot(copy, `duplicate local bot ${botId}`);
			this.ids.addLocal(bot.id);
			return { ...bot, local: true };
		}
		const bot = expectBot(
			await post<IBot>(
				account.profile,
				`${botPath(account.id)}/clone`,
				undefined,
				account.auth,
			),
			`duplicate bot ${botId}`,
		);
		this.ids.addRemote(account.sub, [bot.id]);
		return bot;
	}

	async updateBot(botId: string, request: IUpdateBotRequest): Promise<IBot> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localBots.update(botId, request));
		}
		return expectBot(
			await patch<IBot>(
				account.profile,
				botPath(account.id),
				request,
				account.auth,
			),
			`update bot ${botId}`,
		);
	}

	async deleteBot(botId: string): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			await this.here(botId, () => localBots.delete(botId));
			this.ids.forgetLocal(botId);
			return;
		}
		await del<void>(
			account.profile,
			botPath(account.id),
			undefined,
			account.auth,
		);
	}

	async listBotApps(botId: string): Promise<IBotApp[]> {
		const account = await this.accountFor(botId);
		if (!account) return this.here(botId, () => localBots.apps(botId));
		const response = await get<{ apps?: IBotApp[] }>(
			account.profile,
			`${botPath(account.id)}/apps`,
			account.auth,
		);
		return asArray(response?.apps);
	}

	async attachBotApp(botId: string, appId: string): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localBots.attachApp(botId, appId));
		}
		await put<void>(
			account.profile,
			appPath(account.id, appId),
			undefined,
			account.auth,
		);
	}

	async setBotAppRestrictions(
		botId: string,
		appId: string,
		restrictions: IBotRestrictions,
	): Promise<IBotRestrictions> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () =>
				localBots.setAppRestrictions(botId, appId, restrictions),
			);
		}
		return put<IBotRestrictions>(
			account.profile,
			`${appPath(account.id, appId)}/restrictions`,
			restrictions,
			account.auth,
		);
	}

	async detachBotApp(botId: string, appId: string): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localBots.detachApp(botId, appId));
		}
		await del<void>(
			account.profile,
			appPath(account.id, appId),
			undefined,
			account.auth,
		);
	}

	async listBotSecrets(botId: string): Promise<IBotSecretInfo[]> {
		const account = await this.accountFor(botId);
		if (!account) return this.here(botId, () => localBots.secrets(botId));
		const response = await get<{ secrets?: IBotSecretInfo[] }>(
			account.profile,
			`${botPath(account.id)}/secrets`,
			account.auth,
		);
		return asArray(response?.secrets);
	}

	async upsertBotSecret(
		botId: string,
		name: string,
		request: IUpsertBotSecretRequest,
	): Promise<IBotSecretInfo> {
		const account = await this.accountFor(botId);
		if (!account) {
			const { value, description } = request;
			if (value === undefined) {
				throw new Error(
					`Could not save secret ${name} of bot ${botId}: a bot on this computer takes a plain value`,
				);
			}
			return this.here(botId, () =>
				localBots.setSecret(botId, name, value, description),
			);
		}
		const response = await put<IBotSecretInfo>(
			account.profile,
			secretPath(account.id, name),
			request,
			account.auth,
		);
		if (!isRecord(response)) {
			throw new Error(
				`Unexpected response while saving secret ${name} of bot ${botId}`,
			);
		}
		return response as unknown as IBotSecretInfo;
	}

	async deleteBotSecret(botId: string, name: string): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localBots.deleteSecret(botId, name));
		}
		await del<void>(
			account.profile,
			secretPath(account.id, name),
			undefined,
			account.auth,
		);
	}

	async getPresence(): Promise<Record<string, IBotPresence>> {
		const session = accountSession(this.backend);
		if (!session) return {};
		const response = await get<Record<string, IBotPresence>>(
			session.profile,
			"bots/presence",
			session.auth,
		);
		return isRecord(response) ? response : {};
	}

	async listPermissions(botId: string): Promise<IBotPermissions> {
		const account = await this.accountFor(botId);
		if (!account) {
			const all = await this.here(botId, () => localPermissions.list());
			return {
				granted: all.filter((grant) => grant.from_bot_id === botId),
				received: all.filter((grant) => grant.to_bot_id === botId),
			};
		}
		const response = await get<Partial<IBotPermissions>>(
			account.profile,
			`${botPath(account.id)}/permissions`,
			account.auth,
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
		const account = await this.accountFor(fromBotId);
		if (!account) {
			return this.here(fromBotId, () =>
				localPermissions.grant(fromBotId, toBotId),
			);
		}
		const response = await put<IBotPermission>(
			account.profile,
			permissionPath(account.id, this.ids.accountId(toBotId, account.sub)),
			undefined,
			account.auth,
		);
		if (!isRecord(response)) {
			throw new Error(
				`Unexpected response while letting bot ${fromBotId} ask ${toBotId}`,
			);
		}
		return response as unknown as IBotPermission;
	}

	async listTasks(botId: string): Promise<IBotTask[]> {
		const account = await this.accountFor(botId);
		if (!account) return this.here(botId, () => localTasks.list(botId));
		const response = await get<{ tasks?: IBotTask[] }>(
			account.profile,
			`${botPath(account.id)}/tasks`,
			account.auth,
		);
		return asArray(response?.tasks);
	}

	async createTask(
		botId: string,
		request: ICreateBotTaskRequest,
	): Promise<IBotTask> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localTasks.create(botId, request));
		}
		return expectTask(
			await post<IBotTask>(
				account.profile,
				`${botPath(account.id)}/tasks`,
				request,
				account.auth,
			),
		);
	}

	async updateTask(
		botId: string,
		taskId: string,
		request: IUpdateBotTaskRequest,
	): Promise<IBotTask> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localTasks.update(botId, taskId, request));
		}
		return expectTask(
			await patch<IBotTask>(
				account.profile,
				`${botPath(account.id)}/tasks/${encodeURIComponent(taskId)}`,
				request,
				account.auth,
			),
		);
	}

	async deleteTask(botId: string, taskId: string): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localTasks.delete(botId, taskId));
		}
		await del<void>(
			account.profile,
			`${botPath(account.id)}/tasks/${encodeURIComponent(taskId)}`,
			undefined,
			account.auth,
		);
	}

	private async mailRoute(botId: string): Promise<IAccountRoute> {
		const account = await this.accountFor(botId);
		if (!account) {
			throw new Error(
				"Email needs a bot on an account; this bot lives on this device",
			);
		}
		return account;
	}

	async getMailAddress(botId: string): Promise<IBotMailAddress> {
		const account = await this.accountFor(botId);
		if (!account) {
			return { configured: false, enabled: false };
		}
		return get<IBotMailAddress>(
			account.profile,
			`${botPath(account.id)}/email-address`,
			account.auth,
		);
	}

	async checkMailAddress(
		botId: string,
		alias: string,
	): Promise<IBotMailAvailability> {
		const account = await this.mailRoute(botId);
		return get<IBotMailAvailability>(
			account.profile,
			`${botPath(account.id)}/email-address/check/${encodeURIComponent(alias)}`,
			account.auth,
		);
	}

	async claimMailAddress(
		botId: string,
		alias: string,
	): Promise<IBotMailAddress> {
		const account = await this.mailRoute(botId);
		return put<IBotMailAddress>(
			account.profile,
			`${botPath(account.id)}/email-address`,
			{ alias },
			account.auth,
		);
	}

	async releaseMailAddress(botId: string): Promise<void> {
		const account = await this.mailRoute(botId);
		await del<void>(
			account.profile,
			`${botPath(account.id)}/email-address`,
			undefined,
			account.auth,
		);
	}

	async listMailThreads(botId: string): Promise<IBotMailThread[]> {
		const account = await this.accountFor(botId);
		if (!account) return [];
		const response = await get<{ threads?: IBotMailThread[] }>(
			account.profile,
			`${botPath(account.id)}/mail/threads`,
			account.auth,
		);
		return asArray(response?.threads);
	}

	async getMailThread(
		botId: string,
		threadId: string,
	): Promise<IBotMailThreadView> {
		const account = await this.mailRoute(botId);
		return get<IBotMailThreadView>(
			account.profile,
			`${botPath(account.id)}/mail/threads/${encodeURIComponent(threadId)}`,
			account.auth,
		);
	}

	private async webhookRoute(botId: string): Promise<IAccountRoute> {
		const account = await this.accountFor(botId);
		if (!account) {
			throw new Error(
				"Webhooks need a bot on an account; this bot lives on this device",
			);
		}
		return account;
	}

	async listWebhooks(botId: string): Promise<IBotWebhook[]> {
		const account = await this.webhookRoute(botId);
		const response = await get<{ webhooks?: IBotWebhook[] }>(
			account.profile,
			`${botPath(account.id)}/webhooks`,
			account.auth,
		);
		return asArray(response?.webhooks);
	}

	async createWebhook(
		botId: string,
		request: { name: string; instruction?: string },
	): Promise<IIssuedBotWebhook> {
		const account = await this.webhookRoute(botId);
		return expectIssued(
			await post<IIssuedBotWebhook>(
				account.profile,
				`${botPath(account.id)}/webhooks`,
				request,
				account.auth,
			),
		);
	}

	async rotateWebhook(
		botId: string,
		hookId: string,
	): Promise<IIssuedBotWebhook> {
		const account = await this.webhookRoute(botId);
		return expectIssued(
			await post<IIssuedBotWebhook>(
				account.profile,
				`${botPath(account.id)}/webhooks/${encodeURIComponent(hookId)}/rotate`,
				undefined,
				account.auth,
			),
		);
	}

	async deleteWebhook(botId: string, hookId: string): Promise<void> {
		const account = await this.webhookRoute(botId);
		await del<void>(
			account.profile,
			`${botPath(account.id)}/webhooks/${encodeURIComponent(hookId)}`,
			undefined,
			account.auth,
		);
	}

	async webhookUrl(botId: string, webhook: IBotWebhook): Promise<string> {
		const account = await this.webhookRoute(botId);
		return getApiUrl(account.profile, webhook.path.replace(/^\//, ""));
	}

	async listToolGrants(botId: string): Promise<IBotToolGrant[]> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localToolGrants.list(botId));
		}
		const response = await get<{ grants?: IBotToolGrant[] }>(
			account.profile,
			toolGrantsPath(account.id),
			account.auth,
		);
		return asArray(response?.grants);
	}

	async putToolGrant(
		botId: string,
		tool: string,
		scope: string,
	): Promise<IBotToolGrant> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localToolGrants.put(botId, tool, scope));
		}
		const response = await put<IBotToolGrant>(
			account.profile,
			toolGrantsPath(account.id),
			{ tool, scope },
			account.auth,
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
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () => localToolGrants.delete(botId, tool, scope));
		}
		await del<unknown>(
			account.profile,
			toolGrantsPath(account.id),
			{ tool, scope },
			account.auth,
		);
	}

	async answerApproval(
		botId: string,
		turnId: string,
		approvalId: string,
		decision: BotApprovalDecision,
	): Promise<void> {
		const account = await this.accountFor(botId);
		if (!account) {
			return this.here(botId, () =>
				localToolGrants.answerApproval(botId, turnId, approvalId, decision),
			);
		}
		await post<unknown>(
			account.profile,
			`${botPath(account.id)}/turns/${encodeURIComponent(turnId)}/approval`,
			{ approval_id: approvalId, decision },
			account.auth,
		);
	}

	async revokePermission(fromBotId: string, toBotId: string): Promise<void> {
		const account = await this.accountFor(fromBotId);
		if (!account) {
			return this.here(fromBotId, () =>
				localPermissions.revoke(fromBotId, toBotId),
			);
		}
		await del<void>(
			account.profile,
			permissionPath(account.id, this.ids.accountId(toBotId, account.sub)),
			undefined,
			account.auth,
		);
	}
}
