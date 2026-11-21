// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import {
	takeCompactionRequest,
	useCompactionRequestStore,
} from "../../state/global-chat/compaction-request";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { ContextMeter } from "./context-meter";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown, options?: Record<string, unknown>) =>
			(typeof fallback === "string" ? fallback : key).replace(
				/\{\{(\w+)\}\}/g,
				(_, name: string) => String(options?.[name] ?? ""),
			),
		i18n: { language: "en" },
	}),
}));

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	useCompactionRequestStore.setState({ requested: {} });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

function message(
	id: string,
	role: IRole,
	context?: { estimated_tokens: number; context_window: number },
): IMessage {
	return {
		id,
		inner: { role, content: id },
		usage_stats: context
			? [
					{
						step_name: "Assistant",
						stats: {
							usage: {
								prompt_tokens: 10,
								completion_tokens: 5,
								total_tokens: 15,
							},
							context: { ...context, reserved_output_tokens: 1_000 },
						},
					},
				]
			: undefined,
	} as unknown as IMessage;
}

async function render(
	messages: IMessage[],
	canCompact = true,
	conversationId = "bot:bot_scout",
) {
	await act(async () => {
		useGlobalChatStore.setState({
			activeConversationId: conversationId,
			messages,
		});
		root.render(<ContextMeter canCompact={canCompact} />);
	});
}

const trigger = () => host.querySelector("button");

async function open() {
	await act(async () => trigger()?.click());
}

const button = (text: string) =>
	Array.from(document.body.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.trim() === text,
	);

describe("the context meter", () => {
	test("reads the newest finished reply, not one still streaming", async () => {
		await render([
			message("old", IRole.Assistant, {
				estimated_tokens: 10_000,
				context_window: 100_000,
			}),
			message("ask", IRole.User),
			message("last", IRole.Assistant, {
				estimated_tokens: 62_000,
				context_window: 100_000,
			}),
			message("ask-again", IRole.User),
			message("streaming", IRole.Assistant),
		]);
		expect(trigger()?.getAttribute("aria-label")).toBe(
			"Context window: 62% used",
		);
	});

	test("stays in the composer before the first reply", async () => {
		await render([]);
		expect(trigger()?.getAttribute("aria-label")).toBe("Context window");
	});

	test("queues a compaction for the open chat's next reply only, once", async () => {
		await render([
			message("last", IRole.Assistant, {
				estimated_tokens: 90_000,
				context_window: 100_000,
			}),
		]);
		await open();
		await act(async () => button("Compact before next reply")?.click());
		expect(useCompactionRequestStore.getState().requested).toEqual({
			"bot:bot_scout": true,
		});
		expect(document.body.textContent).toContain(
			"Your next message starts by summarizing the earlier conversation.",
		);
		expect(takeCompactionRequest("flux-notes")).toBe(false);
		expect(takeCompactionRequest("bot:bot_scout")).toBe(true);
		expect(takeCompactionRequest("bot:bot_scout")).toBe(false);
	});

	test("a queued compaction can be called off", async () => {
		useCompactionRequestStore.getState().setRequested("bot:bot_scout", true);
		await render([]);
		await open();
		await act(async () => button("Don't compact")?.click());
		expect(useCompactionRequestStore.getState().requested).toEqual({});
	});

	test("offers no compaction where the bot answers on its own host", async () => {
		await render([], false);
		await open();
		expect(button("Compact before next reply")).toBeUndefined();
		expect(document.body.textContent).toContain(
			"This bot answers on its own host",
		);
	});
});
