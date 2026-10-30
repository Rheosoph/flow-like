// @vitest-environment happy-dom

import "fake-indexeddb/auto";
import { act } from "react";
import type { Root } from "react-dom/client";
import { afterEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../../lib/schema/llm/history";
import type { IMessage } from "./chat-db";

vi.mock("./event-processor", () => ({ finalizePlanSteps: vi.fn() }));
vi.mock("../../ui/text-editor", () => ({
	TextEditor: ({ initialContent }: { initialContent: string }) => (
		<div>{initialContent}</div>
	),
}));
vi.mock("../../ui/streaming-text-editor", () => ({
	StreamingTextEditor: () => null,
}));
vi.mock("./hooks/use-processed-attachments", () => ({
	useProcessedAttachments: () => [],
}));
vi.mock("./attachment", () => ({ getDisplayFileName: (name: string) => name }));
vi.mock("./attachment-dialog", () => ({
	FileDialog: () => null,
	FileDialogPreview: () => null,
	canPreviewFile: () => false,
	downloadFile: vi.fn(),
}));
vi.mock("./attachment-strip", () => ({ AttachmentStrip: () => null }));
vi.mock("./app-references", () => ({ AppReferences: () => null }));
vi.mock("./message-widgets", () => ({ MessageWidgets: () => null }));
vi.mock("./agent-debug-report", () => ({ AgentDebugReport: () => null }));
vi.mock("./usage-stats", () => ({ UsageStats: () => null }));

let root: Root | undefined;
let chatDb: typeof import("./chat-db").chatDb | undefined;

async function setup() {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	return { container, root };
}

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
	await chatDb?.messages.delete("reaction-message");
});

describe("message reactions", () => {
	test("sends add and remove actions through the host callback and reloads their saved state", async () => {
		const { root, container } = await setup();
		const db = await import("./chat-db");
		chatDb = db.chatDb;
		const { decodeMessageRow, putChatMessage } = db;
		const { MessageComponent } = await import("./message");
		const { applyMessageReaction } = await import("./chat-message-interaction");
		let message: IMessage = {
			id: "reaction-message",
			appId: "app",
			sessionId: "reaction-session",
			timestamp: 1,
			inner: { role: IRole.Assistant, content: "Ready" },
			files: [],
			rating: 1,
		};
		const actions: Array<{
			messageId: string;
			emoji: string;
			operation: string;
		}> = [];
		const onMessageUpdate = vi.fn();
		let saving: Promise<void> | undefined;
		const onReaction = (
			target: IMessage,
			emoji: string,
			operation: "add" | "remove",
		) => {
			saving = (async () => {
				expect(target.id).toBe(message.id);
				actions.push({ messageId: target.id, emoji, operation });
				await putChatMessage({
					...message,
					reactions: applyMessageReaction(message.reactions, emoji, operation),
				});
				const stored = await db.chatDb.messages.get(target.id);
				if (!stored) throw new Error("Message was not saved");
				message = decodeMessageRow(stored);
				render();
			})();
			return saving;
		};
		const render = () =>
			root.render(
				<MessageComponent
					message={message}
					onMessageUpdate={onMessageUpdate}
					onReaction={onReaction}
				/>,
			);
		await act(async () => render());
		const add = container.querySelector<HTMLButtonElement>(
			'button[aria-label="Add reaction"]',
		);
		expect(add).not.toBeNull();
		await act(async () => add?.click());
		const heart = document.querySelector<HTMLButtonElement>(
			'[data-slot="popover-content"] button[aria-label="React with ❤️"]',
		);
		expect(heart).not.toBeNull();
		await act(async () => {
			heart?.click();
			await saving;
		});
		expect(actions).toEqual([
			{ messageId: message.id, emoji: "❤️", operation: "add" },
		]);
		expect(onMessageUpdate).not.toHaveBeenCalled();
		expect(message.reactions).toEqual(["❤️"]);
		expect(message.rating).toBe(1);
		const selected = container.querySelector<HTMLButtonElement>(
			'fieldset button[aria-label="React with ❤️"]',
		);
		expect(selected?.getAttribute("aria-pressed")).toBe("true");
		await act(async () => {
			selected?.click();
			await saving;
		});
		expect(message.reactions).toEqual([]);
		expect(actions.at(-1)).toEqual({
			messageId: message.id,
			emoji: "❤️",
			operation: "remove",
		});
		expect(container.querySelector("fieldset")).toBeNull();
	});

	test("shows saved reactions without offering edits in a read-only chat", async () => {
		const { root, container } = await setup();
		const { MessageComponent, ChatFeedbackEnabledContext } = await import(
			"./message"
		);
		await act(async () => {
			root.render(
				<ChatFeedbackEnabledContext.Provider value={false}>
					<MessageComponent
						onMessageUpdate={vi.fn()}
						message={{
							id: "read-only",
							appId: "app",
							sessionId: "session",
							timestamp: 1,
							inner: { role: IRole.User, content: "Hello" },
							files: [],
							reactions: ["🎉"],
						}}
					/>
				</ChatFeedbackEnabledContext.Provider>,
			);
		});
		expect(
			container.querySelector('button[aria-label="Add reaction"]'),
		).toBeNull();
		expect(container.querySelector("fieldset button")?.textContent).toBe("🎉");
		expect(
			container.querySelector<HTMLButtonElement>("fieldset button")?.disabled,
		).toBe(true);
	});

	test("quotes the selected passage of an assistant message and renders saved context", async () => {
		const { root, container } = await setup();
		const { MessageComponent } = await import("./message");
		const message: IMessage = {
			id: "older-answer",
			appId: "app",
			sessionId: "session",
			timestamp: 1,
			inner: {
				role: IRole.Assistant,
				content: "First paragraph. Second paragraph.",
			},
			files: [],
		};
		const reply = vi.fn();
		await act(async () =>
			root.render(<MessageComponent message={message} onReply={reply} />),
		);
		const textNode = [...container.querySelectorAll("div")].find(
			(element) =>
				element.childNodes.length === 1 &&
				element.firstChild?.nodeType === Node.TEXT_NODE &&
				element.textContent === message.inner.content,
		)?.firstChild;
		expect(textNode).toBeDefined();
		if (!textNode) throw new Error("Message text did not render");
		const range = document.createRange();
		range.setStart(textNode, 17);
		range.setEnd(textNode, 34);
		window.getSelection()?.removeAllRanges();
		window.getSelection()?.addRange(range);
		await act(async () =>
			container
				.querySelector<HTMLButtonElement>(
					'button[aria-label="Reply to message"]',
				)
				?.click(),
		);
		expect(reply).toHaveBeenCalledWith(message, "Second paragraph.");
		window.getSelection()?.removeAllRanges();
		await act(async () =>
			root.render(
				<MessageComponent
					message={{
						...message,
						id: "reply",
						inner: { role: IRole.User, content: "Explain this" },
						interaction: {
							type: "reply",
							target: {
								messageId: message.id,
								role: IRole.Assistant,
								text: "Second paragraph.",
							},
						},
					}}
				/>,
			),
		);
		expect(container.querySelector("blockquote")?.textContent).toContain(
			"Second paragraph.",
		);
		expect(container.textContent).toContain("Explain this");
	});

	test("disables reply and emoji actions while a single-run chat is busy", async () => {
		const { root, container } = await setup();
		const { MessageComponent } = await import("./message");
		const reply = vi.fn();
		const reaction = vi.fn(async () => {});
		await act(async () =>
			root.render(
				<MessageComponent
					message={{
						id: "busy",
						appId: "app",
						sessionId: "session",
						timestamp: 1,
						inner: { role: IRole.Assistant, content: "Earlier answer" },
						files: [],
						reactions: ["❤️"],
					}}
					onReply={reply}
					onReaction={reaction}
					interactionDisabled
				/>,
			),
		);
		for (const label of ["Reply to message", "Add reaction", "React with ❤️"]) {
			const button = container.querySelector<HTMLButtonElement>(
				`button[aria-label="${label}"]`,
			);
			expect(button?.disabled).toBe(true);
			await act(async () => button?.click());
		}
		expect(reply).not.toHaveBeenCalled();
		expect(reaction).not.toHaveBeenCalled();
	});

	test("finds emojis outside the quick choices", async () => {
		const { root, container } = await setup();
		const { MessageReactionPicker } = await import("./message-reactions");
		const selected: string[] = [];
		await act(async () => {
			root.render(
				<MessageReactionPicker onToggle={(emoji) => selected.push(emoji)} />,
			);
		});
		await act(async () => {
			container.querySelector<HTMLButtonElement>("button")?.click();
		});
		await act(async () => {
			await import("@emoji-mart/data");
		});
		const search = document.querySelector<HTMLInputElement>(
			'input[aria-label="Search emojis"]',
		);
		expect(search).not.toBeNull();
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				HTMLInputElement.prototype,
				"value",
			)?.set?.call(search, "rocket");
			search?.dispatchEvent(new Event("input", { bubbles: true }));
		});
		const rocket = document.querySelector<HTMLButtonElement>(
			'[data-slot="popover-content"] button[aria-label="React with 🚀"]',
		);
		expect(rocket).not.toBeNull();
		await act(async () => rocket?.click());
		expect(selected).toEqual(["🚀"]);
	});
});
