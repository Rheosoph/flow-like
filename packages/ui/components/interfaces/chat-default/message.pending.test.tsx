import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { IRole } from "../../../lib";
import type { IMessage } from "./chat-db";

const question = {
	id: "q1",
	timestamp: 0,
	files: [],
	inner: { role: IRole.User, content: "Hi Inge, wie gehts" },
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

describe("a question whose answer has shown nothing yet", () => {
	test("says it is waiting, with the time once it is worth a number", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<MessageComponent
					message={question}
					pending={{ phase: "waiting", since: Date.now() - 4_200 }}
				/>,
			);
		});

		const status = container.querySelector("[data-fl-chat-ask-status]");
		expect(status?.getAttribute("data-fl-chat-ask-status")).toBe("waiting");
		expect(status?.textContent).toContain("Waiting");
		expect(status?.textContent).toContain("4s");
		expect(container.querySelector(".animate-ask-scan")).not.toBeNull();
	});

	test("sending does not run the light yet", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(
				<MessageComponent
					message={question}
					pending={{ phase: "sending", since: Date.now() }}
				/>,
			);
		});

		expect(container.textContent).toContain("Sending");
		expect(container.querySelector(".animate-ask-scan")).toBeNull();
	});

	test("an answered question shows neither", async () => {
		const { MessageComponent } = await import("./message");
		const { container, root } = await setup();

		await act(async () => {
			root.render(<MessageComponent message={question} />);
		});

		expect(container.querySelector("[data-fl-chat-ask-status]")).toBeNull();
		expect(container.querySelector(".animate-ask-scan")).toBeNull();
	});
});
