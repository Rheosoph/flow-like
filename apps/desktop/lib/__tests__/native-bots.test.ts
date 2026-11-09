import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import type {
	IBot,
	IBotTurn,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import type {
	IConversation,
	IInboxEntry,
} from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import { describe, expect, test, vi } from "vitest";
import {
	type NativeBotActivityToken,
	createBotActivityRegistrar,
} from "../native-bot-activities";
import {
	isNativeDeliveryRetryable,
	loadNativeBotCatalog,
	withNativeBotTurns,
} from "../native-bots";
import {
	type NativeDispatchContext,
	type NativePendingAction,
	type NativeSnapshot,
	dispatchNativeAction,
	nativeNavigationRequest,
	withNativeActivePage,
} from "../native-integration";

const scope = JSON.stringify(["https://hub.test", "profile", "owner"]);
const bot = {
	id: "bot",
	name: "Scout",
	owner_id: "owner",
	capabilities: ["scheduling"],
	instructions: "PRIVATE INSTRUCTIONS",
} as IBot;
const conversation = {
	id: "room",
	kind: "direct",
	encryption: "plain",
	members: [
		{ kind: "bot", principal_id: "bot" },
		{ kind: "user", principal_id: "owner" },
	],
} as IConversation;
const turn: IBotTurn = {
	id: "turn",
	bot_id: "bot",
	conversation_id: "room",
	status: "waiting",
	reply_message_id: "approval-message",
	updated_at: new Date().toISOString(),
};
const entry = {
	id: "entry",
	message_id: "approval-message",
	sender_id: "bot",
	conversation_id: "room",
	kind: "message",
	created_at: new Date().toISOString(),
	envelope: {
		v: 1,
		enc: "none",
		body: {
			text: "PRIVATE MESSAGE",
			approval: {
				id: "approval",
				turn_id: "turn",
				tool: "shell",
				summary: "PRIVATE ARGUMENT",
				scope: "PRIVATE PATH",
				args_hash: "hash",
			},
		},
	},
} as IInboxEntry;

function fixture() {
	const backend = {
		botState: {
			listBots: vi.fn(async () => [bot]),
			getBot: vi.fn(async () => bot),
			getMobileStatus: vi.fn(async () => [turn]),
			answerApproval: vi.fn(async () => {}),
			listTasks: vi.fn(async () => [
				{
					id: "task",
					title: "Daily report",
					instruction: "FRESH INSTRUCTION",
					enabled: true,
				},
			]),
		},
		inboxState: {
			listConversations: vi.fn(async () => [conversation]),
			getConversation: vi.fn(async () => conversation),
			createConversation: vi.fn(async () => conversation),
			listEntries: vi.fn(async () => ({ entries: [entry] })),
			sendEntry: vi.fn(async () => entry),
		},
	};
	const context: NativeDispatchContext = {
		backend: backend as unknown as IBackendState,
		scope,
		isCurrent: () => true,
		navigate: vi.fn(),
		bot: vi.fn(async () => {}),
		flowpilot: vi.fn(),
		execute: vi.fn(async () => {}),
		complete: vi.fn(async () => {}),
	};
	const request = (
		action: NativePendingAction["action"],
	): NativePendingAction => ({
		id: "dc0384ce-c906-48f6-90cb-5fc86531655a",
		scope,
		action,
	});
	return { backend, context, request };
}

describe("native bot delivery", () => {
	test("surfaces a terminal background failure without resending the action", async () => {
		const { backend, context, request } = fixture();
		await expect(
			dispatchNativeAction(
				{
					...request({ kind: "ask_bot", botId: "bot", prompt: "hello" }),
					backgroundError: "The bot rejected this reply.",
				},
				context,
			),
		).rejects.toThrow("rejected this reply");
		expect(backend.botState.getBot).not.toHaveBeenCalled();
		expect(backend.inboxState.sendEntry).not.toHaveBeenCalled();
	});

	test("retries an explicit recent background reply and expires it after one day", async () => {
		const { backend, context, request } = fixture();
		const queued = {
			...request({
				kind: "ask_bot",
				botId: "bot",
				conversationId: "room",
				prompt: "hello",
			}),
			backgroundDelivery: true,
			createdAt: new Date(Date.now() - 3_600_000).toISOString(),
		};
		await dispatchNativeAction(queued, context);
		expect(backend.inboxState.sendEntry).toHaveBeenCalledOnce();
		await expect(
			dispatchNativeAction(
				{
					...queued,
					createdAt: new Date(Date.now() - 86_400_001).toISOString(),
				},
				context,
			),
		).rejects.toThrow("expired");
		expect(backend.inboxState.sendEntry).toHaveBeenCalledOnce();
	});

	test("Siri asks wait for the real bot answer through the native chat completion path", async () => {
		const { backend, context, request } = fixture();
		await dispatchNativeAction(
			{
				...request({ kind: "ask_bot", botId: "bot", prompt: "What changed?" }),
				responseMode: "text",
			},
			context,
		);
		expect(context.bot).toHaveBeenCalledWith(
			expect.objectContaining({ botId: "bot", prompt: "What changed?" }),
		);
		expect(backend.inboxState.sendEntry).not.toHaveBeenCalled();
		expect(context.complete).not.toHaveBeenCalled();
	});
	test("rejects a queued action after account or workspace change", async () => {
		const { backend, context, request } = fixture();
		await expect(
			dispatchNativeAction(
				{
					...request({ kind: "ask_bot", botId: "bot", prompt: "hello" }),
					scope: "another account",
				},
				context,
			),
		).rejects.toThrow("different account");
		expect(backend.botState.getBot).not.toHaveBeenCalled();
	});

	test("rechecks the bot and conversation before sending an idempotent reply", async () => {
		const { backend, context, request } = fixture();
		const action = request({
			kind: "ask_bot",
			botId: "bot",
			conversationId: "room",
			prompt: "Continue",
		});
		await dispatchNativeAction(action, context);
		await dispatchNativeAction(action, context);
		expect(backend.inboxState.getConversation).toHaveBeenCalledWith("room");
		expect(backend.botState.getBot).toHaveBeenCalledWith("bot");
		for (const args of backend.inboxState.sendEntry.mock.calls)
			expect(args).toEqual([
				"room",
				{
					message_id: action.id,
					kind: "message",
					envelope: {
						v: 1,
						enc: "none",
						body: { text: "Continue", mentions: ["bot"] },
					},
				},
			]);
		expect(context.bot).not.toHaveBeenCalled();
	});

	test("does not send after the account changes during a read", async () => {
		const { backend, context, request } = fixture();
		backend.botState.getBot.mockImplementation(async () => {
			context.isCurrent = () => false;
			return bot;
		});
		await dispatchNativeAction(
			request({ kind: "ask_bot", botId: "bot", prompt: "hello" }),
			context,
		);
		expect(backend.inboxState.sendEntry).not.toHaveBeenCalled();
	});

	test("rejects a notification whose bot no longer belongs to its room", async () => {
		const { backend, context, request } = fixture();
		backend.inboxState.getConversation.mockResolvedValue({
			...conversation,
			members: [],
		});
		await expect(
			dispatchNativeAction(
				request({
					kind: "ask_bot",
					botId: "bot",
					conversationId: "room",
					prompt: "hello",
				}),
				context,
			),
		).rejects.toThrow("no longer");
		expect(backend.inboxState.sendEntry).not.toHaveBeenCalled();
	});

	test("uses fresh task instructions and stable message IDs without altering its schedule", async () => {
		const { backend, context, request } = fixture();
		const action = request({
			kind: "run_bot_task",
			botId: "bot",
			taskId: "task",
			prompt: "STALE INSTRUCTION",
		});
		await dispatchNativeAction(action, context);
		expect(backend.inboxState.sendEntry).toHaveBeenCalledWith(
			"room",
			expect.objectContaining({
				message_id: action.id,
				envelope: expect.objectContaining({
					body: { text: "FRESH INSTRUCTION", mentions: ["bot"] },
				}),
			}),
		);
		backend.botState.listTasks.mockResolvedValue([
			{
				id: "task",
				title: "Daily report",
				instruction: "anything",
				enabled: false,
			},
		]);
		await expect(dispatchNativeAction(action, context)).rejects.toThrow(
			"paused",
		);
	});

	test("native approval actions permit once or deny, never a standing grant", async () => {
		const { backend, context, request } = fixture();
		const action = request({
			kind: "respond_bot_approval",
			botId: "bot",
			turnId: "turn",
			approvalId: "approval",
			decision: "once",
		});
		await dispatchNativeAction(action, context);
		expect(backend.botState.answerApproval).toHaveBeenCalledWith(
			"bot",
			"turn",
			"approval",
			"once",
		);
		await expect(
			dispatchNativeAction(
				{
					...action,
					action: { ...action.action, decision: "always" as "once" },
				},
				context,
			),
		).rejects.toThrow("Allow once or Deny");
		await expect(
			dispatchNativeAction(
				{ ...action, responseDeadline: "2000-01-01T00:00:00Z" },
				context,
			),
		).rejects.toThrow("expired");
		expect(backend.botState.answerApproval).toHaveBeenCalledTimes(1);
	});

	test("routes encrypted input through the existing encrypted chat instead of a plain envelope", async () => {
		const { backend, context, request } = fixture();
		backend.botState.getBot.mockResolvedValue({
			...bot,
			capabilities: ["encryption"],
		});
		await dispatchNativeAction(
			request({ kind: "ask_bot", botId: "bot", prompt: "private" }),
			context,
		);
		expect(backend.inboxState.sendEntry).not.toHaveBeenCalled();
		expect(context.bot).toHaveBeenCalledWith(
			expect.objectContaining({ prompt: "private" }),
		);
	});

	test("only scoped navigation links can open bots, and links cannot approve actions", () => {
		const url = `flow-like://native/bot?scope=${encodeURIComponent(scope)}&botId=bot&decision=once&approvalId=approval`;
		expect(nativeNavigationRequest(url, scope)?.action.kind).toBe("open_bot");
		expect(nativeNavigationRequest(url, "other")).toBeUndefined();
		expect(
			nativeNavigationRequest(
				`flow-like://native/bots?scope=${encodeURIComponent(scope)}`,
				scope,
			)?.action.kind,
		).toBe("open_bot");
	});

	test("network failures remain retryable, authorization failures do not", () => {
		expect(
			isNativeDeliveryRetryable(new Error("Network unavailable: entries")),
		).toBe(true);
		expect(isNativeDeliveryRetryable(new Error("Forbidden"))).toBe(false);
		expect(isNativeDeliveryRetryable(new Error("Approval expired"))).toBe(
			false,
		);
	});
});

describe("native bot display cache", () => {
	test("publishes display labels and pending approvals without private message/tool data", async () => {
		const { context } = fixture();
		const catalog = await loadNativeBotCatalog(context.backend);
		expect(catalog.bots[0]).toMatchObject({ id: "bot", title: "Scout" });
		expect(catalog.botTasks[0]).toMatchObject({
			id: "task",
			action: { kind: "run_bot_task", botId: "bot" },
		});
		expect(catalog.botApprovals).toHaveLength(1);
		expect(JSON.stringify(catalog)).not.toMatch(
			/PRIVATE|FRESH INSTRUCTION|hash/,
		);
	});

	test("excludes resolved, expired, and secret-request approvals", async () => {
		const { backend, context } = fixture();
		backend.botState.getMobileStatus.mockResolvedValue([
			{ ...turn, status: "done" },
		]);
		expect((await loadNativeBotCatalog(context.backend)).botApprovals).toEqual(
			[],
		);
		backend.botState.getMobileStatus.mockResolvedValue([
			{ ...turn, updated_at: "2000-01-01T00:00:00Z" },
		]);
		expect((await loadNativeBotCatalog(context.backend)).botApprovals).toEqual(
			[],
		);
		backend.botState.getMobileStatus.mockResolvedValue([turn]);
		const secret = structuredClone(entry);
		(
			secret.envelope as { body: { approval: Record<string, unknown> } }
		).body.approval.secret = { name: "TOKEN", description: "Enter it" };
		backend.inboxState.listEntries.mockResolvedValue({ entries: [secret] });
		expect((await loadNativeBotCatalog(context.backend)).botApprovals).toEqual(
			[],
		);
	});

	test("includes room-triggered approvals delivered to the owner's direct chat", async () => {
		const { backend, context } = fixture();
		backend.botState.getMobileStatus.mockResolvedValue([
			{ ...turn, conversation_id: "group-room" },
		]);
		const catalog = await loadNativeBotCatalog(context.backend);
		expect(catalog.botApprovals).toEqual([
			expect.objectContaining({
				id: "approval",
				conversationId: "room",
				botId: "bot",
				turnId: "turn",
			}),
		]);
	});

	test("never reads encrypted conversation entries or exports encrypted task instructions", async () => {
		const { backend, context } = fixture();
		backend.botState.listBots.mockResolvedValue([
			{ ...bot, capabilities: ["encryption", "scheduling"] },
		]);
		backend.inboxState.listConversations.mockResolvedValue([
			{ ...conversation, encryption: "e2e", title: "PRIVATE TITLE" },
		]);
		const catalog = await loadNativeBotCatalog(context.backend);
		expect(backend.inboxState.listEntries).not.toHaveBeenCalled();
		expect(backend.botState.listTasks).not.toHaveBeenCalled();
		expect(JSON.stringify(catalog)).not.toContain("PRIVATE");
	});

	test("finishing a turn removes its live status while retaining scheduled tasks", async () => {
		const { context } = fixture();
		const catalog = await loadNativeBotCatalog(context.backend);
		const snapshot = {
			...catalog,
			scope,
			apps: [],
		} as unknown as NativeSnapshot;
		const updated = withNativeBotTurns(snapshot, [{ ...turn, status: "done" }]);
		expect(
			updated.sections
				.find((section) => section.kind === "bot_tasks")
				?.items.map((item) => item.id),
		).toEqual(["task"]);
		expect(
			withNativeActivePage(snapshot, "/bots", "bot=bot", "https://app.test")
				.activePage,
		).toEqual({ title: "Scout", url: "https://app.test/bots?bot=bot" });
	});

	test("authoritative sync removes absent turns while incremental updates retain them", async () => {
		const { context } = fixture();
		const catalog = await loadNativeBotCatalog(context.backend);
		const snapshot = {
			...catalog,
			scope,
			apps: [],
		} as unknown as NativeSnapshot;
		const items = (value: NativeSnapshot) =>
			value.sections.find((section) => section.kind === "bot_tasks")?.items;
		expect(
			items(withNativeBotTurns(snapshot, [], false))?.some((item) => item.live),
		).toBe(true);
		expect(
			items(withNativeBotTurns(snapshot, []))?.map((item) => item.id),
		).toEqual(["task"]);
		const unavailable = {
			...snapshot,
			sections: snapshot.sections.map((section) => ({
				...section,
				state: "unavailable" as const,
			})),
		};
		const refreshed = withNativeBotTurns(unavailable, [turn], false);
		expect(
			refreshed.sections.find((section) => section.kind === "bot_tasks")?.state,
		).toBe("ready");
		expect(items(refreshed)?.find((item) => item.live)?.updatedAt).toBe(
			turn.updated_at,
		);
	});
});

test("Live Activity tokens bind to scope and target and re-register on token rotation", async () => {
	const register = createBotActivityRegistrar();
	const send = vi.fn(async () => {});
	const token = {
		activityId: "activity",
		botId: "bot",
		turnId: "turn",
		scope,
		pushToken: "aa".repeat(32),
	};
	await register(
		scope,
		"device",
		[{ ...token, scope: "foreign" }, token],
		send,
		() => true,
	);
	await register(scope, "device", [token], send, () => true);
	expect(send).toHaveBeenCalledTimes(1);
	await register(
		scope,
		"device",
		[{ ...token, pushToken: "bb".repeat(32) }],
		send,
		() => true,
	);
	await register(scope, "other-device", [token], send, () => true);
	expect(send).toHaveBeenCalledTimes(3);
	await register(scope, "third-device", [token], send, () => false);
	expect(send).toHaveBeenCalledTimes(3);
	const remove = vi.fn(async () => {});
	await register(scope, "device", [], send, () => true, remove);
	expect(remove).toHaveBeenCalledOnce();
	expect(remove).toHaveBeenCalledWith(
		expect.objectContaining({
			activityId: "activity",
			pushToken: "bb".repeat(32),
		}),
	);
});

test("a rejected activity registration does not block other tokens and retries later", async () => {
	const register = createBotActivityRegistrar();
	const local = {
		activityId: "local-activity",
		botId: "local-bot",
		turnId: "local-turn",
		scope,
		pushToken: "aa".repeat(32),
	};
	const account = { ...local, activityId: "account-activity", botId: "bot" };
	const send = vi.fn(async (token: NativeBotActivityToken) => {
		if (token === local) throw new Error("Bot not found");
	});
	await register(scope, "device", [local, account], send, () => true);
	await register(scope, "device", [local, account], send, () => true);
	expect(send.mock.calls.map(([token]) => token.activityId)).toEqual([
		"local-activity",
		"account-activity",
		"local-activity",
	]);
});

test("a failed activity removal does not block other removals or new registrations", async () => {
	const register = createBotActivityRegistrar();
	const tokens = ["first", "second", "third"].map((activityId) => ({
		activityId,
		botId: "bot",
		turnId: activityId,
		scope,
		pushToken: "aa".repeat(32),
	}));
	const send = vi.fn(async () => {});
	await register(scope, "device", tokens.slice(0, 2), send, () => true);
	const remove = vi
		.fn(async (_token: NativeBotActivityToken) => {})
		.mockRejectedValueOnce(new Error("Temporary failure"));
	await register(scope, "device", tokens.slice(2), send, () => true, remove);
	await register(scope, "device", tokens.slice(2), send, () => true, remove);
	expect(remove.mock.calls.map(([token]) => token.activityId)).toEqual([
		"first",
		"second",
		"first",
	]);
	expect(send).toHaveBeenCalledTimes(3);
	expect(send).toHaveBeenLastCalledWith(tokens[2]);
});

test("the native catalog reports only successful authoritative turn reads", async () => {
	const { backend, context } = fixture();
	const onTurnsRead = vi.fn();
	await loadNativeBotCatalog(context.backend, onTurnsRead);
	expect(onTurnsRead).toHaveBeenCalledExactlyOnceWith([turn]);
	onTurnsRead.mockClear();
	backend.botState.getMobileStatus.mockRejectedValueOnce(new Error("Offline"));
	await loadNativeBotCatalog(context.backend, onTurnsRead);
	expect(onTurnsRead).not.toHaveBeenCalled();
	backend.botState.getMobileStatus.mockResolvedValueOnce([]);
	await loadNativeBotCatalog(context.backend, onTurnsRead);
	expect(onTurnsRead).toHaveBeenCalledExactlyOnceWith([]);
});
