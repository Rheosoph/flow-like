// @vitest-environment happy-dom

import {
	type ReactNode,
	act,
	createRef,
	forwardRef,
	useImperativeHandle,
} from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../../lib/schema/llm/history";
import { Chat, type IChatProps, type IChatRef } from "./chat";
import type { IMessage } from "./chat-db";
import type { ISendMessageFunction } from "./chatbox";

const state = vi.hoisted(() => ({
	send: undefined as unknown as ISendMessageFunction,
	focus: vi.fn(),
	defaultTools: [] as string[],
}));

vi.mock("@flow-like/locales", () => ({
	i18n: { t: (_key: string, fallback: string) => fallback },
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("./VoiceMode", () => ({ VoiceMode: () => null }));
vi.mock("./ai-disclosure", () => ({ ChatAiDisclosure: () => null }));
vi.mock("./chat-run-controls", () => ({ ChatRunControls: () => null }));
vi.mock("./interaction", () => ({
	Interaction: () => null,
	InteractionGroup: () => null,
}));
vi.mock("./main-widget-pane", () => ({
	MainWidgetPane: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("./message-shell", () => ({
	MessageShell: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("./use-answer-playback", () => ({
	useAnswerPlayback: () => ({
		analyser: null,
		isPlaying: false,
		stop: vi.fn(),
	}),
}));
vi.mock("./voice-config", () => ({
	isVoiceEnabled: () => false,
	resolveChatVoiceConfig: () => ({
		invoke: "manual",
		mode: "record",
		playback: "none",
	}),
}));
vi.mock("./chatbox", () => ({
	ChatBox: forwardRef(
		(
			{
				onSendMessage,
				defaultActiveTools,
			}: { onSendMessage: ISendMessageFunction; defaultActiveTools: string[] },
			ref,
		) => {
			state.send = onSendMessage;
			state.defaultTools = defaultActiveTools;
			useImperativeHandle(ref, () => ({ focusInput: state.focus }));
			return <div data-test-composer />;
		},
	),
}));
vi.mock("./message", () => ({
	MessageComponent: ({
		message,
		onReply,
		onReaction,
		interactionDisabled,
		pending,
	}: {
		message: IMessage;
		onReply?: (message: IMessage) => void;
		onReaction?: (
			message: IMessage,
			emoji: string,
			operation: "add" | "remove",
		) => Promise<void>;
		interactionDisabled?: boolean;
		pending?: { phase: string };
	}) => (
		<div data-message-id={message.id} data-pending={pending?.phase}>
			<button
				type="button"
				disabled={interactionDisabled}
				aria-label={`Reply ${message.id}`}
				onClick={() => onReply?.(message)}
			>
				Reply
			</button>
			<button
				type="button"
				disabled={interactionDisabled}
				aria-label={`React ${message.id}`}
				onClick={() => void onReaction?.(message, "❤️", "add")}
			>
				React
			</button>
		</div>
	),
}));

let root: Root;
let container: HTMLDivElement;
let chatRef = createRef<IChatRef>();
const answer: IMessage = {
	id: "older-answer",
	appId: "app",
	sessionId: "session",
	timestamp: 1,
	inner: {
		role: IRole.Assistant,
		content: "The second option uses PostgreSQL.",
	},
	files: [],
};
const send = vi.fn<ISendMessageFunction>(async () => {});
const reaction = vi.fn(async () => {});
const render = async (props: Partial<IChatProps> = {}) => {
	await act(async () =>
		root.render(
			<Chat
				ref={chatRef}
				messages={[answer]}
				sessionId="session"
				onSendMessage={send}
				onMessageReaction={reaction}
				config={{ default_tools: [] }}
				{...props}
			/>,
		),
	);
};
const click = async (label: string) => {
	await act(async () =>
		container
			.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)
			?.click(),
	);
};

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	send.mockReset().mockResolvedValue(undefined);
	reaction.mockClear();
	state.focus.mockClear();
	chatRef = createRef<IChatRef>();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("quoted chat replies", () => {
	test("hides saved reaction turns while keeping emoji messages and reaction replies visible", async () => {
		const question: IMessage = {
			...answer,
			id: "question",
			timestamp: 0,
			inner: { role: IRole.User, content: "Which database?" },
			tools: ["search"],
		};
		const reactionTurn: IMessage = {
			...question,
			id: "reaction-turn",
			timestamp: 2,
			inner: { role: IRole.User, content: "❤️" },
			tools: [],
			interaction: {
				type: "reaction",
				target: {
					messageId: answer.id,
					role: IRole.Assistant,
					text: String(answer.inner.content),
				},
				emoji: "❤️",
				operation: "add",
			},
		};
		await render({
			messages: [question, answer, reactionTurn],
			config: { tools: ["search"] },
		});
		expect(
			container.querySelector('[data-message-id="reaction-turn"]'),
		).toBeNull();
		expect(state.defaultTools).toEqual(["search"]);
		await act(async () => {
			chatRef.current?.pushMessage({ ...reactionTurn, id: "local-reaction" });
			chatRef.current?.pushCurrentMessageUpdate({
				...answer,
				id: "response",
				timestamp: 3,
				inner: { role: IRole.Assistant, content: "" },
			});
			await new Promise(requestAnimationFrame);
		});
		expect(
			container.querySelector('[data-message-id="local-reaction"]'),
		).toBeNull();
		expect(container.querySelector('[data-message-id="response"]')).toBeNull();
		expect(container.querySelector("[data-pending]")).toBeNull();
		await act(async () => {
			chatRef.current?.pushCurrentMessageUpdate({
				...answer,
				id: "response",
				timestamp: 3,
			});
			await new Promise(requestAnimationFrame);
		});
		expect(
			container.querySelector('[data-message-id="response"]'),
		).not.toBeNull();
		await render({
			messages: [
				question,
				answer,
				reactionTurn,
				{
					...reactionTurn,
					id: "typed-emoji",
					timestamp: 4,
					interaction: undefined,
				},
			],
		});
		expect(
			container.querySelector('[data-message-id="typed-emoji"]'),
		).not.toBeNull();
		expect(
			container.querySelector('[data-message-id="reaction-turn"]'),
		).toBeNull();
	});

	test("does not flash an optimistic message for an imperative reaction send", async () => {
		await render();
		let finish = () => {};
		send.mockImplementationOnce(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		let sending: Promise<void> | undefined;
		const interaction = {
			type: "reaction" as const,
			target: {
				messageId: answer.id,
				role: IRole.Assistant,
				text: String(answer.inner.content),
			},
			emoji: "❤️",
			operation: "add" as const,
		};
		await act(async () => {
			sending = chatRef.current?.sendMessage(
				"❤️",
				undefined,
				undefined,
				undefined,
				interaction,
			);
		});
		expect(send.mock.calls[0]?.[4]).toEqual(interaction);
		expect(container.querySelector('[data-fl-chat-message="user"]')).toBeNull();
		await act(async () => {
			finish();
			await sending;
		});
	});

	test("selects an older message without running and sends its snapshot with the reply", async () => {
		await render();
		await click("Reply older-answer");
		expect(send).not.toHaveBeenCalled();
		expect(container.querySelector("blockquote")?.textContent).toContain(
			"PostgreSQL",
		);
		expect(state.focus).toHaveBeenCalled();
		const files = [new File(["notes"], "notes.txt")];
		await act(async () => state.send("Why that option?", files, ["search"]));
		expect(send).toHaveBeenCalledWith(
			"Why that option?",
			files,
			["search"],
			undefined,
			{
				type: "reply",
				target: {
					messageId: answer.id,
					role: IRole.Assistant,
					text: answer.inner.content,
				},
			},
		);
		expect(container.querySelector("blockquote")).toBeNull();
	});

	test("a reaction leaves the pending quote for the composer's own reply", async () => {
		await render();
		await click("Reply older-answer");
		await click("React older-answer");
		expect(reaction).toHaveBeenCalledWith(answer, "❤️", "add");
		expect(send).not.toHaveBeenCalled();
		expect(container.querySelector("blockquote")).not.toBeNull();
		await act(async () => state.send("Tell me more"));
		expect(send.mock.calls[0]?.[4]).toMatchObject({
			type: "reply",
			target: { messageId: answer.id },
		});
	});

	test("cancel and a session change both remove the pending quote", async () => {
		await render();
		await click("Reply older-answer");
		await click("Cancel reply");
		await act(async () => state.send("Plain question"));
		expect(send.mock.calls[0]?.[4]).toBeUndefined();
		await click("Reply older-answer");
		await render({ sessionId: "other-session", messages: [] });
		expect(container.querySelector("blockquote")).toBeNull();
		await act(async () => state.send("New conversation"));
		expect(send.mock.calls[1]?.[4]).toBeUndefined();
	});

	test("a rejected send restores the quote and busy chats disable interaction controls", async () => {
		await render();
		await click("Reply older-answer");
		send.mockRejectedValueOnce(new Error("Busy"));
		await act(async () => {
			await expect(state.send("Retry later")).rejects.toThrow("Busy");
		});
		expect(container.querySelector("blockquote")).not.toBeNull();
		await render({ isStreamActive: true });
		expect(
			container.querySelector<HTMLButtonElement>(
				'button[aria-label="Reply older-answer"]',
			)?.disabled,
		).toBe(true);
		expect(
			container.querySelector<HTMLButtonElement>(
				'button[aria-label="React older-answer"]',
			)?.disabled,
		).toBe(true);
	});
});
