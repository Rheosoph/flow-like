import type {
	BotApprovalDecision,
	IBot,
	IBotApp,
	IBotPermission,
	IBotRestrictions,
	IBotSecretInfo,
	IBotTask,
	IBotToolGrant,
	IBotWorkspace,
	IConversation,
	IConversationFileDownload,
	IConversationFileReady,
	IConversationPinState,
	IConversationSegmentList,
	ICreateBotRequest,
	ICreateBotTaskRequest,
	ICreateConversationRequest,
	IInboxEntriesPage,
	IInboxEntriesQuery,
	IInboxEntry,
	IInboxImportEntry,
	IInboxImportResult,
	IInboxSegmentsQuery,
	IPinnedMessage,
	ISendInboxEntryRequest,
	IStarredMessage,
	IStarredMessagesPage,
	IStarredMessagesQuery,
	IUpdateBotRequest,
	IUpdateBotTaskRequest,
} from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { type InvokeArgs, invoke } from "@tauri-apps/api/core";

/** The store's answer for an id it doesn't hold. */
export const LOCAL_NOT_FOUND = "NOT_FOUND";

export type LocalEntriesQuery = IInboxEntriesQuery;

function isPositionQuery(query: LocalEntriesQuery): boolean {
	return [
		query.from_seq,
		query.to_seq,
		query.around_seq,
		query.before_count,
		query.after_count,
	].some((value) => value !== undefined);
}

/** A refusal or failure of this computer's bot store, with the store's `code` when it gave one. */
export class LocalStoreError extends Error {
	readonly code?: string;

	constructor(message: string, code?: string, cause?: unknown) {
		super(message, { cause });
		this.name = "LocalStoreError";
		this.code = code;
	}
}

/** The `code` a failure carries, from this computer's store or from the hub. */
export function errorCode(error: unknown): string | undefined {
	const code = isRecord(error) ? error.code : undefined;
	return typeof code === "string" ? code : undefined;
}

/** Commands reject with `{ code, message }`; anything else is read as text. */
function rejection(error: unknown): { message: string; code?: string } {
	if (typeof error === "string") return { message: error };
	if (isRecord(error)) {
		const message =
			typeof error.message === "string" ? error.message : JSON.stringify(error);
		return { message, code: errorCode(error) };
	}
	return { message: JSON.stringify(error) ?? String(error) };
}

async function run<T>(
	operation: string,
	command: string,
	args?: InvokeArgs,
): Promise<T> {
	try {
		return await invoke<T>(command, args);
	} catch (error) {
		const { message, code } = rejection(error);
		throw new LocalStoreError(
			`Could not ${operation}: ${message}`,
			code,
			error,
		);
	}
}

function markLocal<T extends { local?: boolean }>(value: T): T {
	return value.local ? value : { ...value, local: true };
}

function expectLocal<T extends { local?: boolean }>(
	value: unknown,
	operation: string,
): T {
	if (!isRecord(value)) {
		throw new Error(`Could not ${operation}: this computer returned nothing`);
	}
	return markLocal(value as T);
}

export const localBots = {
	async list(): Promise<IBot[]> {
		const bots = await run<IBot[]>(
			"list the bots on this computer",
			"local_bots_list",
		);
		return asArray(bots).map(markLocal);
	},

	/** Null when no bot with this id lives on this computer. */
	async get(botId: string): Promise<IBot | null> {
		const bot = await run<IBot | null>(
			`load local bot ${botId}`,
			"local_bots_get",
			{ botId },
		);
		return isRecord(bot) ? markLocal(bot as unknown as IBot) : null;
	},

	async create(request: ICreateBotRequest): Promise<IBot> {
		const operation = "create a bot on this computer";
		return expectLocal<IBot>(
			await run(operation, "local_bots_create", { request }),
			operation,
		);
	},

	async update(botId: string, request: IUpdateBotRequest): Promise<IBot> {
		const operation = `update local bot ${botId}`;
		return expectLocal<IBot>(
			await run(operation, "local_bots_update", { botId, request }),
			operation,
		);
	},

	/** Deletes the bot with its workspace and memory. */
	async delete(botId: string): Promise<void> {
		await run(`delete local bot ${botId}`, "local_bots_delete", { botId });
	},

	/** The app holding the bot's files and databases, made on first use; refused for Flux. */
	async workspace(botId: string): Promise<IBotWorkspace> {
		const operation = `open the workspace of local bot ${botId}`;
		const workspace = await run<unknown>(operation, "local_bots_workspace", {
			botId,
		});
		if (
			!isRecord(workspace) ||
			typeof workspace.app_id !== "string" ||
			!workspace.app_id
		) {
			throw new Error(`Could not ${operation}: this computer returned no app`);
		}
		return { app_id: workspace.app_id };
	},

	async apps(botId: string): Promise<IBotApp[]> {
		return asArray(
			await run<IBotApp[]>(
				`list the apps of local bot ${botId}`,
				"local_bots_apps",
				{ botId },
			),
		);
	},

	async attachApp(botId: string, appId: string): Promise<void> {
		await run(
			`attach app ${appId} to local bot ${botId}`,
			"local_bots_attach_app",
			{ botId, appId },
		);
	},

	async setAppRestrictions(
		botId: string,
		appId: string,
		restrictions: IBotRestrictions,
	): Promise<IBotRestrictions> {
		return run<IBotRestrictions>(
			`set the restrictions of app ${appId} for local bot ${botId}`,
			"local_bots_set_app_restrictions",
			{ botId, appId, restrictions },
		);
	},

	async detachApp(botId: string, appId: string): Promise<void> {
		await run(
			`detach app ${appId} from local bot ${botId}`,
			"local_bots_detach_app",
			{ botId, appId },
		);
	},

	async secrets(botId: string): Promise<IBotSecretInfo[]> {
		return asArray(
			await run<IBotSecretInfo[]>(
				`list the secrets of local bot ${botId}`,
				"local_bots_secrets",
				{ botId },
			),
		);
	},

	/** An omitted description keeps the stored one and `null` clears it, as on the API. */
	async setSecret(
		botId: string,
		name: string,
		value: string,
		description?: string | null,
	): Promise<IBotSecretInfo> {
		const operation = `save secret ${name} of local bot ${botId}`;
		const secret = await run(operation, "local_bots_set_secret", {
			botId,
			name,
			value,
			// Tauri can't tell null from omitted, so the store reads "" as "clear".
			description: description === null ? "" : description,
		});
		if (!isRecord(secret)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return secret as unknown as IBotSecretInfo;
	},

	async deleteSecret(botId: string, name: string): Promise<void> {
		await run(
			`delete secret ${name} of local bot ${botId}`,
			"local_bots_delete_secret",
			{ botId, name },
		);
	},
};

function expectTask(task: unknown, operation: string): IBotTask {
	if (!isRecord(task)) {
		throw new Error(`Could not ${operation}: this computer returned nothing`);
	}
	return task as unknown as IBotTask;
}

export const localTasks = {
	async list(botId: string): Promise<IBotTask[]> {
		return asArray(
			await run<IBotTask[]>(
				`list the tasks of local bot ${botId}`,
				"local_bot_tasks",
				{ botId },
			),
		);
	},

	async create(
		botId: string,
		request: ICreateBotTaskRequest,
	): Promise<IBotTask> {
		const operation = `schedule a task for local bot ${botId}`;
		return expectTask(
			await run(operation, "local_bot_create_task", { botId, request }),
			operation,
		);
	},

	async update(
		botId: string,
		taskId: string,
		request: IUpdateBotTaskRequest,
	): Promise<IBotTask> {
		const operation = `change task ${taskId} of local bot ${botId}`;
		return expectTask(
			await run(operation, "local_bot_update_task", {
				botId,
				taskId,
				request,
			}),
			operation,
		);
	},

	async delete(botId: string, taskId: string): Promise<void> {
		await run(
			`delete task ${taskId} of local bot ${botId}`,
			"local_bot_delete_task",
			{ botId, taskId },
		);
	},
};

export const localToolGrants = {
	async list(botId: string): Promise<IBotToolGrant[]> {
		return asArray(
			await run<IBotToolGrant[]>(
				`list the tool grants of local bot ${botId}`,
				"local_bot_tool_grants",
				{ botId },
			),
		);
	},

	async put(
		botId: string,
		tool: string,
		scope: string,
	): Promise<IBotToolGrant> {
		const operation = `allow ${tool} (${scope}) for local bot ${botId}`;
		const grant = await run(operation, "local_bot_put_tool_grant", {
			botId,
			tool,
			scope,
		});
		if (!isRecord(grant)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return grant as unknown as IBotToolGrant;
	},

	async delete(botId: string, tool: string, scope: string): Promise<void> {
		await run(
			`take back ${tool} (${scope}) from local bot ${botId}`,
			"local_bot_delete_tool_grant",
			{ botId, tool, scope },
		);
	},

	async answerApproval(
		botId: string,
		turnId: string,
		approvalId: string,
		decision: BotApprovalDecision,
	): Promise<void> {
		await run(
			`answer the approval of local bot ${botId}`,
			"local_bot_answer_approval",
			{ botId, turnId, approvalId, decision },
		);
	},
};

export const localPermissions = {
	async list(): Promise<IBotPermission[]> {
		return asArray(
			await run<IBotPermission[]>(
				"list the grants between bots on this computer",
				"local_bots_permissions",
			),
		);
	},

	async grant(fromBotId: string, toBotId: string): Promise<IBotPermission> {
		const operation = `let local bot ${fromBotId} ask ${toBotId}`;
		const grant = await run(operation, "local_bots_grant_permission", {
			fromBotId,
			toBotId,
		});
		if (!isRecord(grant)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return grant as unknown as IBotPermission;
	},

	async revoke(fromBotId: string, toBotId: string): Promise<void> {
		await run(
			`take back local bot ${fromBotId}'s permission to ask ${toBotId}`,
			"local_bots_revoke_permission",
			{ fromBotId, toBotId },
		);
	},
};

export const localInbox = {
	async conversations(as?: string): Promise<IConversation[]> {
		const conversations = await run<IConversation[]>(
			"list the conversations on this computer",
			"local_inbox_conversations",
			as ? { principalId: as } : undefined,
		);
		return asArray(conversations).map(markLocal);
	},

	/** Null when the conversation isn't on this computer or you left it. */
	async conversation(
		conversationId: string,
		as?: string,
	): Promise<IConversation | null> {
		const conversation = await run<IConversation | null>(
			`load local conversation ${conversationId}`,
			"local_inbox_conversation",
			{ conversationId, ...(as ? { principalId: as } : {}) },
		);
		return isRecord(conversation)
			? markLocal(conversation as unknown as IConversation)
			: null;
	},

	async create(request: ICreateConversationRequest): Promise<IConversation> {
		const operation = `create a ${request.kind} conversation on this computer`;
		return expectLocal<IConversation>(
			await run(operation, "local_inbox_create", { request }),
			operation,
		);
	},

	async rename(conversationId: string, title: string): Promise<IConversation> {
		const operation = `rename local conversation ${conversationId}`;
		return expectLocal<IConversation>(
			await run(operation, "local_inbox_rename", { conversationId, title }),
			operation,
		);
	},

	async deleteConversation(conversationId: string): Promise<void> {
		await run(
			`delete local conversation ${conversationId}`,
			"local_inbox_delete_conversation",
			{ conversationId },
		);
	},

	async setImage(
		conversationId: string,
		image: string | null,
	): Promise<IConversation> {
		const operation = `change the picture of local conversation ${conversationId}`;
		return expectLocal<IConversation>(
			await run(operation, "local_inbox_set_image", { conversationId, image }),
			operation,
		);
	},

	async addMembers(
		conversationId: string,
		members: string[],
	): Promise<IConversation> {
		const operation = `add members to local conversation ${conversationId}`;
		return expectLocal<IConversation>(
			await run(operation, "local_inbox_add_members", {
				conversationId,
				members,
			}),
			operation,
		);
	},

	async removeMember(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		await run(
			`remove ${principalId} from local conversation ${conversationId}`,
			"local_inbox_remove_member",
			{ conversationId, principalId },
		);
	},

	async entries(
		conversationId: string,
		query: LocalEntriesQuery = {},
	): Promise<IInboxEntriesPage> {
		if (isPositionQuery(query)) {
			return localInbox.entriesByPosition(conversationId, query);
		}
		const page = await run<IInboxEntriesPage | null>(
			`load entries of local conversation ${conversationId}`,
			"local_inbox_entries",
			{
				conversationId,
				...(query.as ? { principalId: query.as } : {}),
				before: query.before,
				after: query.after,
				limit: query.limit,
			},
		);
		return {
			entries: asArray(page?.entries),
			next_before: page?.next_before ?? null,
		};
	},

	async entriesByPosition(
		conversationId: string,
		query: LocalEntriesQuery,
	): Promise<IInboxEntriesPage> {
		const page = await run<IInboxEntriesPage | null>(
			`load entries of local conversation ${conversationId} by position`,
			"local_inbox_entries_by_position",
			{
				conversationId,
				...(query.as ? { principalId: query.as } : {}),
				query: {
					from_seq: query.from_seq,
					to_seq: query.to_seq,
					around_seq: query.around_seq,
					before_count: query.before_count,
					after_count: query.after_count,
					limit: query.limit,
				},
			},
		);
		return {
			entries: asArray(page?.entries),
			next_before: page?.next_before ?? null,
			next_from_seq: page?.next_from_seq ?? null,
			next_to_seq: page?.next_to_seq ?? null,
		};
	},

	async importHistory(
		conversationId: string,
		botId: string,
		entries: IInboxImportEntry[],
	): Promise<IInboxImportResult> {
		const result = await run<IInboxImportResult | null>(
			`import the earlier chat into local conversation ${conversationId}`,
			"local_inbox_import_history",
			{ conversationId, botId, entries },
		);
		return { imported: result?.imported ?? 0, skipped: result?.skipped ?? 0 };
	},

	async segments(
		conversationId: string,
		query: IInboxSegmentsQuery = {},
	): Promise<IConversationSegmentList> {
		const list = await run<IConversationSegmentList | null>(
			`load segments of local conversation ${conversationId}`,
			"local_inbox_segments",
			{
				conversationId,
				...(query.as ? { principalId: query.as } : {}),
				query: { level: query.level, after: query.after, limit: query.limit },
			},
		);
		return {
			segments: asArray(list?.segments),
			next_after: list?.next_after ?? null,
		};
	},

	async send(
		conversationId: string,
		request: ISendInboxEntryRequest,
	): Promise<IInboxEntry> {
		const operation = `send a ${request.kind} to local conversation ${conversationId}`;
		const entry = await run(operation, "local_inbox_send", {
			conversationId,
			request,
		});
		if (!isRecord(entry)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return entry as unknown as IInboxEntry;
	},

	/** Stores a file the person puts into a conversation on this computer. */
	async uploadFile(
		conversationId: string,
		file: File,
	): Promise<IConversationFileReady> {
		const operation = `store ${file.name} in local conversation ${conversationId}`;
		const bytes = new Uint8Array(await file.arrayBuffer());
		try {
			return await invoke<IConversationFileReady>(
				"local_inbox_upload_file",
				bytes,
				{
					headers: {
						"x-conversation-id": encodeURIComponent(conversationId),
						"x-file-name": encodeURIComponent(file.name),
						"x-file-mime": encodeURIComponent(file.type),
					},
				},
			);
		} catch (error) {
			const { message, code } = rejection(error);
			throw new LocalStoreError(
				`Could not ${operation}: ${message}`,
				code,
				error,
			);
		}
	},

	/** A `blob:` URL for a file kept on this computer; revoke it when done. */
	async fileUrl(
		conversationId: string,
		handle: { path: string; mime: string },
		name = handle.path,
		as?: string,
	): Promise<IConversationFileDownload> {
		const bytes = await run<ArrayBuffer>(
			`read ${name} of local conversation ${conversationId}`,
			"local_inbox_read_file",
			{ conversationId, path: handle.path, ...(as ? { principalId: as } : {}) },
		);
		return {
			url: URL.createObjectURL(new Blob([bytes], { type: handle.mime })),
			name,
			mime: handle.mime,
			bytes: bytes.byteLength,
		};
	},

	async markRead(
		conversationId: string,
		upTo?: string,
	): Promise<{ last_read_at: string }> {
		const operation = `mark local conversation ${conversationId} as read`;
		const marker = await run(operation, "local_inbox_mark_read", {
			conversationId,
			upTo,
		});
		if (!isRecord(marker)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return marker as { last_read_at: string };
	},

	async setPinned(
		conversationId: string,
		pinned: boolean,
	): Promise<IConversationPinState> {
		const operation = `${pinned ? "pin" : "unpin"} local conversation ${conversationId}`;
		const state = await run(operation, "local_inbox_set_pinned", {
			conversationId,
			pinned,
		});
		if (!isRecord(state)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return state as unknown as IConversationPinState;
	},

	async pins(conversationId: string, as?: string): Promise<IPinnedMessage[]> {
		const pins = await run(
			`list pinned messages of local conversation ${conversationId}`,
			"local_inbox_pins",
			{ conversationId, ...(as ? { principalId: as } : {}) },
		);
		return Array.isArray(pins) ? (pins as IPinnedMessage[]) : [];
	},

	async pinMessage(
		conversationId: string,
		messageId: string,
	): Promise<IPinnedMessage> {
		const operation = `pin ${messageId} in local conversation ${conversationId}`;
		const pin = await run(operation, "local_inbox_pin_message", {
			conversationId,
			messageId,
		});
		if (!isRecord(pin)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return pin as unknown as IPinnedMessage;
	},

	async unpinMessage(conversationId: string, messageId: string): Promise<void> {
		await run(
			`unpin ${messageId} in local conversation ${conversationId}`,
			"local_inbox_unpin_message",
			{ conversationId, messageId },
		);
	},

	async star(
		conversationId: string,
		messageId: string,
	): Promise<IStarredMessage> {
		const operation = `star ${messageId} in local conversation ${conversationId}`;
		const star = await run(operation, "local_inbox_star", {
			conversationId,
			messageId,
		});
		if (!isRecord(star)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return star as unknown as IStarredMessage;
	},

	async unstar(conversationId: string, messageId: string): Promise<void> {
		await run(
			`unstar ${messageId} in local conversation ${conversationId}`,
			"local_inbox_unstar",
			{ conversationId, messageId },
		);
	},

	async stars(
		query: IStarredMessagesQuery = {},
	): Promise<IStarredMessagesPage> {
		const operation = "list starred messages on this computer";
		const page = await run(operation, "local_inbox_stars", {
			conversationId: query.conversationId,
			before: query.before,
			limit: query.limit,
		});
		if (!isRecord(page)) {
			throw new Error(`Could not ${operation}: this computer returned nothing`);
		}
		return page as unknown as IStarredMessagesPage;
	},
};
