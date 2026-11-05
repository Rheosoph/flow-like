// @vitest-environment happy-dom

import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import type { RunWidgetAction } from "../interfaces/chat-default/chat-widget-execution";
import { useGlobalChatRunWidgetAction } from "./use-global-widget-action";

const state = vi.hoisted(() => ({
	committed: [] as unknown[],
	persisted: [] as unknown[],
	executeBoard: vi.fn(),
}));

vi.mock("../../index", () => ({
	IRole: { Assistant: "assistant" },
	useBackend: () => ({ boardState: { executeBoard: state.executeBoard } }),
}));
vi.mock("../../state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: {
		getState: () => ({
			activeConversationId: "conversation",
			messages: [],
			runs: {},
			commitMessage: (message: unknown) =>
				state.committed.push(structuredClone(message)),
			addInteractions: vi.fn(),
		}),
	},
}));
vi.mock("../../state/global-chat/global-chat-stream", () => ({
	makeGlobalChatMessage: () => ({
		id: "first",
		appId: "global-chat",
		sessionId: "conversation",
		timestamp: 1,
		inner: { role: "assistant", content: "" },
		files: [],
	}),
	persistGlobalChatMessage: async (message: unknown) => {
		state.persisted.push(structuredClone(message));
	},
}));
vi.mock("../a2ui/frontend-state", () => ({ getFrontendStateStore: vi.fn() }));

test("widget actions save every pushed message with its originating board", async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.committed = [];
	state.persisted = [];
	state.executeBoard.mockImplementation(
		async (_app, _board, _payload, _stream, _start, emit) => {
			for (const messageId of ["one", "two"]) {
				emit([
					{
						event_type: "chat_stream",
						payload: {
							message_id: messageId,
							response: {
								choices: [
									{
										index: 0,
										message: { role: "assistant", content: messageId },
									},
								],
							},
							attachments: [`${messageId}.pdf`],
							widgets: [
								{
									instance_id: messageId,
									widget_id: "w",
									surface_id: messageId,
									component: {},
								},
							],
						},
					},
				]);
			}
			return {};
		},
	);
	let run!: RunWidgetAction;
	function Host() {
		run = useGlobalChatRunWidgetAction();
		return null;
	}
	const element = document.createElement("div");
	const root = createRoot(element);
	try {
		await act(async () => root.render(<Host />));
		await run("source-app", "source-board", { id: "action" });
		const messages = state.persisted as IMessage[];
		expect(messages.every((message) => message.appId === "global-chat")).toBe(
			true,
		);
		expect(messages.map((message) => message.inner.content)).toEqual([
			"one",
			"two",
		]);
		expect(messages.map((message) => message.files)).toEqual([
			["one.pdf"],
			["two.pdf"],
		]);
		expect(
			messages.every(
				(message) => message.widgets?.[0].origin?.boardId === "source-board",
			),
		).toBe(true);
		expect(state.committed).toHaveLength(2);
	} finally {
		await act(async () => root.unmount());
	}
});
