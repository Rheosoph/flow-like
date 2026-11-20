// @vitest-environment happy-dom

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

const other = {
	id: "other",
	timestamp: 1,
	files: [],
	inner: { role: IRole.Assistant, content: "Here is the table" },
} as unknown as IMessage;

let root: Root | undefined;

async function setup(coarse: boolean) {
	Object.assign(globalThis, {
		IS_REACT_ACT_ENVIRONMENT: true,
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	});
	window.matchMedia = (() => ({
		matches: !coarse,
		addEventListener() {},
		removeEventListener() {},
	})) as unknown as typeof window.matchMedia;
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	return { container, root };
}

afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	root = undefined;
});

const touch = (type: string, x: number, y: number) => {
	const event = new Event(type, { bubbles: true });
	Object.assign(event, {
		touches: type === "touchend" ? [] : [{ clientX: x, clientY: y }],
	});
	return event;
};

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function mount(coarse = true) {
	const { MessageComponent } = await import("./message");
	const { container, root } = await setup(coarse);
	const toggled: string[] = [];
	const replied: string[] = [];
	await act(async () => {
		root.render(
			<MessageComponent
				message={other}
				layout="room"
				onReply={(message) => replied.push(message.id)}
				onReaction={async (_message, emoji) => {
					toggled.push(emoji);
				}}
				renderMenu={() => <button type="button">Pin it</button>}
			/>,
		);
	});
	const bubble = container.querySelector(
		'[data-fl-chat-message="assistant"]',
	) as HTMLElement;
	return { bubble, toggled, replied };
}

const menu = () => document.body.querySelector("[data-fl-chat-touch-menu]");

describe("the touch menu", () => {
	test("a long press opens reactions, actions and the room items", async () => {
		const { bubble, toggled } = await mount();
		await act(async () => {
			bubble.dispatchEvent(touch("touchstart", 10, 10));
			expect(bubble.hasAttribute("data-fl-pressing")).toBe(true);
			await wait(500);
		});
		expect(bubble.hasAttribute("data-fl-pressing")).toBe(false);
		expect(menu()).not.toBeNull();
		expect(menu()?.textContent).toContain("Reply");
		expect(menu()?.textContent).toContain("Copy");
		expect(menu()?.textContent).toContain("Pin it");
		const thumbs = menu()?.querySelector(
			'button[aria-label="React with \u{1F44D}"]',
		) as HTMLElement;
		await act(async () => {
			thumbs.click();
			await wait(100);
		});
		expect(toggled).toEqual(["\u{1F44D}"]);
		expect(menu()).toBeNull();
	});

	test("moving the finger or lifting early cancels the press", async () => {
		const { bubble } = await mount();
		await act(async () => {
			bubble.dispatchEvent(touch("touchstart", 10, 10));
			bubble.dispatchEvent(touch("touchmove", 10, 30));
			await wait(500);
		});
		expect(menu()).toBeNull();
		await act(async () => {
			bubble.dispatchEvent(touch("touchstart", 10, 10));
			bubble.dispatchEvent(touch("touchend", 10, 10));
			await wait(500);
		});
		expect(menu()).toBeNull();
	});

	test("a fine pointer never opens it", async () => {
		const { bubble } = await mount(false);
		await act(async () => {
			bubble.dispatchEvent(touch("touchstart", 10, 10));
			await wait(500);
		});
		expect(menu()).toBeNull();
	});
});
