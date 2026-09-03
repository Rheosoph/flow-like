import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { IRole } from "../../../lib";
import type { IMessage } from "./chat-db";

function liveMessage(content: string): IMessage {
	return {
		id: "a1",
		timestamp: 0,
		inner: { role: IRole.Assistant, content },
	} as unknown as IMessage;
}

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
		// Slate/Plate reach for these during their DOM lookups.
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
	const container = window.document.createElement("div");
	window.document.body.append(container);
	return { container, root: createRoot(container) };
}

describe("assistant streaming", () => {
	test("each content update reaches the DOM while loading", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent loading message={liveMessage("Hel")} />);
		});
		expect(container.textContent).toContain("Hel");

		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hello wor")} />,
			);
		});
		expect(container.textContent).toContain("Hello wor");

		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hello world")} />,
			);
		});
		expect(container.textContent).toContain("Hello world");
	});

	test("an empty live turn shows the thinking indicator", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent loading message={liveMessage("")} />);
		});
		expect(container.textContent).toContain("Thinking");
	});

	test("the settled turn keeps the final text", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<MessageComponent loading message={liveMessage("Hello world")} />,
			);
		});
		await act(async () => {
			root.render(<MessageComponent message={liveMessage("Hello world")} />);
		});
		expect(container.textContent).toContain("Hello world");
	});
});

describe("assistant streaming with inline step anchors", () => {
	// The real FlowPilot stream anchors plan steps into the text via
	// content_offset, which routes rendering through buildInlineSegments.
	function anchored(content: string): IMessage {
		return {
			id: "a2",
			timestamp: 0,
			inner: { role: IRole.Assistant, content },
			plan_steps: [
				{
					id: "s1",
					title: "Searched the web",
					status: "done",
					content_offset: 12,
				},
			],
			current_step_id: "s1",
		} as unknown as IMessage;
	}

	test("the tail segment keeps growing in the DOM", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<MessageComponent loading message={anchored("Looking it up. Alpha")} />,
			);
		});
		expect(container.textContent).toContain("Alpha");

		await act(async () => {
			root.render(
				<MessageComponent
					loading
					message={anchored("Looking it up. Alpha bravo")}
				/>,
			);
		});
		expect(container.textContent).toContain("Alpha bravo");

		await act(async () => {
			root.render(
				<MessageComponent
					loading
					message={anchored("Looking it up. Alpha bravo charlie")}
				/>,
			);
		});
		expect(container.textContent).toContain("Alpha bravo charlie");
	});

	test("anchored steps render inline when content arrives as parts", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();
		const message = {
			id: "a3",
			timestamp: 0,
			inner: {
				role: IRole.Assistant,
				content: [
					{ type: "text", text: "Hello " },
					{ type: "text", text: "world" },
				],
			},
			plan_steps: [
				{ id: "step-1", title: "Searching", status: "done", content_offset: 6 },
			],
		} as unknown as IMessage;

		await act(async () => {
			root.render(<MessageComponent message={message} />);
		});

		const order = Array.from(
			container.querySelectorAll("[data-fl-chat-prose], [data-fl-plan-group]"),
		).map((el) =>
			el.hasAttribute("data-fl-plan-group") ? "steps" : el.textContent?.trim(),
		);
		expect(order).toEqual(["Hello", "steps", "world"]);
	});
});
