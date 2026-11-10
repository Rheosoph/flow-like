import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { IRole } from "../../../lib";
import type { IMessage } from "./chat-db";

const own = {
	id: "own",
	timestamp: 0,
	files: [],
	inner: { role: IRole.User, content: "Sounds good" },
} as unknown as IMessage;

const other = {
	id: "other",
	timestamp: 1,
	files: [],
	inner: { role: IRole.Assistant, content: "Here is the table" },
} as unknown as IMessage;

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		HTMLElement: window.HTMLElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		Range: window.Range,
		Selection: window.Selection,
		Text: window.Text,
		Comment: window.Comment,
		DOMParser: window.DOMParser,
		getSelection: window.getSelection?.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = document.createElement("div");
	document.body.append(container);
	return { container, root: createRoot(container) };
}

const copyIn = (scope: Element | null) =>
	scope?.querySelector('button[title="Copy message"]') ?? null;

describe("the room layout", () => {
	test("your message is a muted bubble with no Asked label", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<MessageComponent
					message={own}
					layout="room"
					pending={{ phase: "waiting", since: Date.now() - 5_000 }}
				/>,
			);
		});

		const bubble = container.querySelector('[data-fl-chat-message="user"]');
		expect(bubble?.className).toContain("bg-muted");
		expect(bubble?.className).toContain("max-w-[85%]");
		expect(bubble?.className.split(" ")).toContain("w-fit");
		expect(bubble?.className.split(" ")).not.toContain("flex-1");
		expect(bubble?.className).toContain("rounded-2xl");
		expect(bubble?.className).not.toContain("border-l-2");
		expect(container.textContent).not.toContain("Asked");
		expect(container.querySelector("[data-fl-chat-ask-status]")).toBeNull();
		expect(container.querySelector(".animate-ask-scan")).toBeNull();
	});

	test("actions float in an overlay and reserve no row", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<div>
					<MessageComponent
						message={other}
						layout="room"
						onReply={() => {}}
						renderMenu={() => <button type="button">More</button>}
					/>
					<MessageComponent message={own} layout="room" />
				</div>,
			);
		});

		const overlays = container.querySelectorAll("[data-fl-chat-room-actions]");
		expect(overlays).toHaveLength(2);
		for (const overlay of overlays) {
			expect(overlay.className).toContain("absolute");
			expect(copyIn(overlay)).not.toBeNull();
		}
		expect(copyIn(container.querySelector(".h-6.w-full"))).toBeNull();
		expect(overlays[0]?.textContent).toContain("More");
		expect(
			overlays[0]?.querySelector('button[title="Reply to message"]'),
		).not.toBeNull();
		expect(
			overlays[1]?.querySelector('button[title="Reply to message"]'),
		).toBeNull();
	});

	test("others stay flat and aligned past the avatar gutter", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent message={other} layout="room" />);
		});

		const body = container.querySelector('[data-fl-chat-message="assistant"]');
		expect(body?.className).toContain("pl-[calc(1rem+30px)]");
		expect(body?.className).not.toContain("bg-muted");
	});
});

describe("without a layout", () => {
	test("your message keeps its Asked card and the inline copy row", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent message={own} />);
		});

		const card = container.querySelector('[data-fl-chat-message="user"]');
		expect(card?.className).toContain("border-l-2");
		expect(card?.className).not.toContain("bg-muted");
		expect(container.textContent).toContain("Asked");
		expect(container.querySelector("[data-fl-chat-room-actions]")).toBeNull();
		expect(copyIn(container)).not.toBeNull();
	});

	test("an answer keeps its padding and the persistent copy row", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent message={other} />);
		});

		const body = container.querySelector('[data-fl-chat-message="assistant"]');
		expect(body?.className).toContain("p-4");
		expect(container.querySelector("[data-fl-chat-room-actions]")).toBeNull();
		expect(copyIn(container.querySelector(".h-6.w-full"))).not.toBeNull();
	});
});
