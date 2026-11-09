import {
	approvalOf,
	approvalResponseOf,
} from "@flow-like/flow-like-ui/components/bots-tab/approval-entry";
import { BOTS_ROUTE } from "@flow-like/flow-like-ui/components/bots-tab/dock-selection";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import type {
	IBot,
	IBotTurn,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import type { IConversation } from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import type {
	NativeAction,
	NativeDispatchContext,
	NativePendingAction,
	NativeSnapshot,
	NativeSnapshotSection,
} from "./native-integration";

export interface NativeBotEntity {
	id: string;
	system?: "general" | "flux";
	title: string;
	subtitle?: string;
	local: boolean;
	encrypted: boolean;
	enabled: boolean;
	surfaces: string[];
	action: NativeAction;
}
export interface NativeBotTask {
	id: string;
	botId: string;
	title: string;
	enabled: boolean;
	status?: string;
	action: NativeAction;
}
export interface NativeBotApproval {
	id: string;
	botId: string;
	conversationId: string;
	turnId: string;
	title: string;
	status: "waiting";
	action: NativeAction;
}
export interface NativeBotCatalog {
	bots: NativeBotEntity[];
	conversations: { id: string; botId?: string; title: string }[];
	botTasks: NativeBotTask[];
	botApprovals: NativeBotApproval[];
	sections: NativeSnapshotSection[];
}

export function nativeBotTitle(bot: IBot): string {
	return (
		bot.name?.trim() ||
		(bot.system === "flux"
			? "Flux"
			: bot.system === "general"
				? "Assistant"
				: "Bot")
	);
}

export function nativeBotUrl(
	action: Pick<
		NativeAction,
		"botId" | "conversationId" | "taskId" | "approvalId"
	>,
): string {
	const query = new URLSearchParams();
	if (action.conversationId) query.set("conversation", action.conversationId);
	else if (action.botId) query.set("bot", action.botId);
	if (action.taskId) query.set("panel", "tasks");
	if (action.approvalId) query.set("approval", action.approvalId);
	return `${BOTS_ROUTE}${query.size ? `?${query}` : ""}`;
}

/** Native surfaces cache display data only. Instructions, messages and encrypted content stay in the app. */
export async function loadNativeBotCatalog(
	backend: IBackendState,
	onTurnsRead?: (turns: readonly IBotTurn[]) => void,
): Promise<NativeBotCatalog> {
	const catalog: NativeBotCatalog = {
		bots: [],
		conversations: [],
		botTasks: [],
		botApprovals: [],
		sections: [],
	};
	const [botsRead, conversationsRead, turnsRead] = await Promise.allSettled([
		backend.botState?.listBots() ?? Promise.resolve([]),
		backend.inboxState?.listConversations() ?? Promise.resolve([]),
		backend.botState?.getMobileStatus?.() ??
			backend.botState?.turns?.list() ??
			Promise.resolve([]),
	]);
	if (turnsRead.status === "fulfilled") onTurnsRead?.(turnsRead.value);
	const bots =
		botsRead.status === "fulfilled" ? botsRead.value.slice(0, 100) : [];
	const conversations =
		conversationsRead.status === "fulfilled" ? conversationsRead.value : [];
	const names = new Map(bots.map((bot) => [bot.id, nativeBotTitle(bot)]));
	const turns =
		turnsRead.status === "fulfilled"
			? new Map(turnsRead.value.map((turn) => [turn.id, turn]))
			: undefined;
	catalog.bots = bots.map((bot) => ({
		id: bot.id,
		system: bot.system || undefined,
		title: nativeBotTitle(bot),
		local: bot.local === true,
		encrypted: bot.capabilities.includes("encryption"),
		enabled: true,
		surfaces: ["siri", "shortcuts", "widget", "share", "spotlight"],
		action: { kind: "open_bot", botId: bot.id },
	}));
	catalog.conversations = conversations.slice(0, 50).map((conversation) => ({
		id: conversation.id,
		botId:
			conversation.kind === "direct"
				? conversation.members.find((member) => member.kind === "bot")
						?.principal_id
				: undefined,
		title:
			conversation.encryption === "e2e"
				? "Encrypted conversation"
				: conversation.title || "Conversation",
	}));
	let tasksAvailable = botsRead.status === "fulfilled";
	const scheduled = bots.filter(
		(bot) =>
			bot.capabilities.includes("scheduling") &&
			!bot.capabilities.includes("encryption"),
	);
	for (let offset = 0; offset < scheduled.length; offset += 4) {
		const batch = scheduled.slice(offset, offset + 4);
		const reads = await Promise.allSettled(
			batch.map(
				(bot) => backend.botState?.listTasks?.(bot.id) ?? Promise.resolve([]),
			),
		);
		reads.forEach((result, index) => {
			if (result.status === "rejected") {
				tasksAvailable = false;
				return;
			}
			const bot = batch[index];
			catalog.botTasks.push(
				...result.value.slice(0, 20).map(
					(task): NativeBotTask => ({
						id: task.id,
						botId: bot.id,
						title: task.title || "Scheduled task",
						enabled: task.enabled,
						status: task.enabled ? "scheduled" : "paused",
						action: { kind: "run_bot_task", botId: bot.id, taskId: task.id },
					}),
				),
			);
		});
	}
	let approvalsAvailable =
		conversationsRead.status === "fulfilled" &&
		turnsRead.status === "fulfilled";
	const plain = conversations
		.filter(
			(conversation) =>
				conversation.encryption === "plain" &&
				turns !== undefined &&
				[...turns.values()].some(
					(turn) =>
						turn.status === "waiting" &&
						(turn.conversation_id === conversation.id ||
							(conversation.kind === "direct" &&
								conversation.members.some(
									(member) =>
										member.kind === "bot" &&
										member.principal_id === turn.bot_id,
								))),
				) &&
				conversation.members.some(
					(member) => member.kind === "bot" && names.has(member.principal_id),
				),
		)
		.slice(0, 12);
	for (let offset = 0; offset < plain.length; offset += 4) {
		const batch = plain.slice(offset, offset + 4);
		const reads = await Promise.allSettled(
			batch.map(
				(conversation) =>
					backend.inboxState?.listEntries(conversation.id, { limit: 100 }) ??
					Promise.resolve({ entries: [] }),
			),
		);
		reads.forEach((result, index) => {
			if (result.status === "rejected") {
				approvalsAvailable = false;
				return;
			}
			const answered = new Set(
				result.value.entries
					.map(approvalResponseOf)
					.filter((item) => item !== undefined)
					.map((item) => item.approval_id),
			);
			for (const entry of result.value.entries) {
				const approval = approvalOf(entry);
				if (
					!approval ||
					approval.secret ||
					answered.has(approval.id) ||
					!names.has(entry.sender_id)
				)
					continue;
				const turn = turns?.get(approval.turn_id);
				if (
					!turn ||
					turn.status !== "waiting" ||
					turn.bot_id !== entry.sender_id ||
					turn.reply_message_id !== entry.message_id
				)
					continue;
				const updatedAt = Date.parse(turn.updated_at || entry.created_at);
				if (
					!Number.isFinite(updatedAt) ||
					updatedAt + 7 * 86_400_000 <= Date.now()
				)
					continue;
				catalog.botApprovals.push({
					id: approval.id,
					botId: entry.sender_id,
					conversationId: batch[index].id,
					turnId: approval.turn_id,
					title: `${names.get(entry.sender_id)} needs approval`,
					status: "waiting",
					action: {
						kind: "open_conversation",
						botId: entry.sender_id,
						conversationId: batch[index].id,
						turnId: approval.turn_id,
						approvalId: approval.id,
					},
				});
			}
		});
	}
	catalog.botTasks = catalog.botTasks.slice(0, 100);
	catalog.botApprovals = catalog.botApprovals.slice(0, 50);
	catalog.sections = [
		{
			kind: "bots",
			title: "Bots",
			state: botsRead.status === "fulfilled" ? "ready" : "unavailable",
			items: catalog.bots
				.slice(0, 12)
				.map((bot) => ({ id: bot.id, title: bot.title, action: bot.action })),
		},
		{
			kind: "bot_tasks",
			title: "Bot tasks",
			state: tasksAvailable ? "ready" : "unavailable",
			items: catalog.botTasks.slice(0, 12).map((task) => ({
				id: task.id,
				title: task.title,
				subtitle: names.get(task.botId),
				status: task.status,
				action: { kind: "open_bot", botId: task.botId, taskId: task.id },
			})),
		},
		{
			kind: "bot_approvals",
			title: "Bot approvals",
			state: approvalsAvailable ? "ready" : "unavailable",
			items: catalog.botApprovals.slice(0, 12).map((approval) => ({
				id: approval.id,
				title: approval.title,
				status: "waiting",
				action: approval.action,
			})),
		},
	];
	if (turnsRead.status === "fulfilled")
		catalog.sections = withNativeBotTurns(
			{ sections: catalog.sections, bots: catalog.bots } as NativeSnapshot,
			turnsRead.value,
		).sections;
	return catalog;
}

export function withNativeBotTurns(
	snapshot: NativeSnapshot,
	turns: readonly IBotTurn[],
	authoritative = true,
): NativeSnapshot {
	const names = new Map(snapshot.bots?.map((bot) => [bot.id, bot.title]));
	const active = turns.filter(
		(turn) =>
			names.has(turn.bot_id) &&
			["queued", "running", "waiting"].includes(turn.status),
	);
	return {
		...snapshot,
		sections: snapshot.sections.map((section) =>
			section.kind !== "bot_tasks"
				? section
				: {
						...section,
						state: active.length > 0 ? "ready" : section.state,
						items: [
							...active.map((turn) => ({
								id: turn.id,
								title: names.get(turn.bot_id) || "Bot",
								status: turn.status,
								startedAt: turn.started_at || undefined,
								updatedAt: turn.updated_at || turn.started_at || undefined,
								live: true as const,
								action: {
									kind: "open_conversation" as const,
									botId: turn.bot_id,
									conversationId: turn.conversation_id,
									turnId: turn.id,
								},
							})),
							...section.items.filter(
								(item) =>
									!item.live ||
									(!authoritative &&
										!turns.some((turn) => turn.id === item.id)),
							),
						].slice(0, 20),
					},
		),
	};
}

export const NATIVE_BOT_ACTIONS = new Set([
	"open_bot",
	"ask_bot",
	"open_conversation",
	"run_bot_task",
	"respond_bot_approval",
	"capture_bot_voice",
	"capture_bot_document",
	"local_bot_summary",
	"private_bot_clipboard",
]);

function required(value: string | undefined, name: string): string {
	if (
		!value ||
		value.length > 512 ||
		[...value].some((character) => character.charCodeAt(0) < 32)
	)
		throw new Error(`This action is missing a valid ${name}.`);
	return value;
}

/** Re-resolve native references under the current account before using them. Cached items never grant access. */
export async function dispatchNativeBotAction(
	request: NativePendingAction,
	context: NativeDispatchContext,
): Promise<void> {
	const action = request.action;
	if (action.kind === "open_bot" && !action.botId && !action.conversationId) {
		context.navigate(BOTS_ROUTE);
		return;
	}
	let conversation: IConversation | undefined;
	if (action.conversationId) {
		if (!context.backend.inboxState)
			throw new Error("Conversations are unavailable.");
		conversation = await context.backend.inboxState.getConversation(
			required(action.conversationId, "conversation"),
		);
		if (!context.isCurrent()) return;
		if (
			action.botId &&
			!conversation.members.some(
				(member) =>
					member.kind === "bot" && member.principal_id === action.botId,
			)
		)
			throw new Error("This bot is no longer in the conversation.");
	}
	if (action.kind === "open_conversation") {
		if (!conversation)
			throw new Error("This action is missing its conversation.");
		context.navigate(nativeBotUrl(action));
		return;
	}
	const botId = required(action.botId, "bot");
	const state = context.backend.botState;
	if (!state) throw new Error("Bots are unavailable.");
	const bot = await state.getBot(botId);
	if (!context.isCurrent()) return;
	if (action.kind === "open_bot") {
		context.navigate(nativeBotUrl(action));
		return;
	}
	if (action.kind === "respond_bot_approval") {
		if (action.decision !== "once" && action.decision !== "deny")
			throw new Error("Choose Allow once or Deny for this approval.");
		if (!state.answerApproval)
			throw new Error("Approval responses are unavailable.");
		await state.answerApproval(
			botId,
			required(action.turnId, "turn"),
			required(action.approvalId, "approval"),
			action.decision,
		);
		if (context.isCurrent()) {
			context.navigate(nativeBotUrl(action));
			await context.complete?.({
				status: "success",
				text: action.decision === "once" ? "Allowed once." : "Denied.",
			});
		}
		return;
	}
	let prompt = (action.prompt || action.text || "").trim();
	if (action.kind === "run_bot_task") {
		if (!bot.capabilities.includes("scheduling") || !state.listTasks)
			throw new Error("This bot cannot run scheduled tasks.");
		const task = (await state.listTasks(botId)).find(
			(task) => task.id === required(action.taskId, "task"),
		);
		if (!context.isCurrent()) return;
		if (!task || !task.enabled)
			throw new Error("This task was deleted or paused.");
		prompt = task.instruction;
	}
	if (
		["ask_bot", "run_bot_task"].includes(action.kind) &&
		request.responseMode !== "text" &&
		prompt &&
		!action.files?.length &&
		!action.voice &&
		!bot.capabilities.includes("encryption") &&
		(!conversation || conversation.encryption === "plain") &&
		context.backend.inboxState
	) {
		if (!prompt || prompt.length > 65_536)
			throw new Error("Provide a message no larger than 64 KiB.");
		conversation ??= await context.backend.inboxState.createConversation({
			kind: "direct",
			members: [botId],
			encryption: "plain",
		});
		if (!context.isCurrent()) return;
		await context.backend.inboxState.sendEntry(conversation.id, {
			message_id: request.id,
			kind: "message",
			envelope: {
				v: 1,
				enc: "none",
				body: { text: prompt, mentions: [botId] },
			},
		});
		if (context.isCurrent()) {
			context.navigate(
				nativeBotUrl({
					botId,
					conversationId:
						conversation.kind === "group" ? conversation.id : undefined,
				}),
			);
			await context.complete?.({ status: "success", text: "Message sent." });
		}
		return;
	}
	if (conversation?.kind === "group")
		throw new Error(
			"Open this conversation to attach files or send an encrypted reply.",
		);
	if (!context.bot) throw new Error("Open the bot in Flow Like to continue.");
	await context.bot(
		action.kind === "run_bot_task"
			? { ...action, kind: "ask_bot", prompt }
			: action,
	);
}

export function isNativeDeliveryRetryable(error: unknown): boolean {
	const message = error instanceof Error ? error.message : String(error);
	return /Network unavailable|Failed to fetch|NetworkError|network request failed|connection (?:reset|refused)|offline|timed out/i.test(
		message,
	);
}
