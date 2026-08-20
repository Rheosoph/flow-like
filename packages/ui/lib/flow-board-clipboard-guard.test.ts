import { beforeEach, describe, expect, test } from "bun:test";
import { Window } from "happy-dom";

function installDom() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLSelectElement: window.HTMLSelectElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		navigator: window.navigator,
		window,
	});
	return window;
}

installDom();

const { shouldIgnoreBoardClipboardEvent } = await import("./flow-board-utils");

let window = globalThis.window as unknown as Window;

// happy-dom's element classes are structurally distinct from lib.dom's, so the DOM this test
// builds only satisfies the guard's `instanceof` checks through the globals installed above.
function focus(element: unknown) {
	(element as HTMLElement).focus();
	Object.defineProperty(window.document, "activeElement", {
		configurable: true,
		get: () => element,
	});
}

beforeEach(() => {
	window = installDom() as unknown as Window;
});

describe("shouldIgnoreBoardClipboardEvent", () => {
	test("ignores clipboard events raised inside a Monaco EditContext host", () => {
		const editor = window.document.createElement("div");
		editor.className = "monaco-editor";
		const host = window.document.createElement("div");
		host.className = "native-edit-context";
		host.setAttribute("tabindex", "0");
		editor.appendChild(host);
		window.document.body.appendChild(editor);
		focus(host);

		expect(
			shouldIgnoreBoardClipboardEvent({
				target: host,
			} as unknown as ClipboardEvent),
		).toBe(true);
	});

	test("ignores clipboard events on an element that owns an EditContext", () => {
		const host = window.document.createElement("div");
		Object.defineProperty(host, "editContext", {
			configurable: true,
			value: {},
		});
		window.document.body.appendChild(host);
		focus(host);

		expect(
			shouldIgnoreBoardClipboardEvent({
				target: host,
			} as unknown as ClipboardEvent),
		).toBe(true);
	});

	test("still handles clipboard events raised on the board canvas", () => {
		const canvas = window.document.createElement("div");
		canvas.className = "react-flow__pane";
		window.document.body.appendChild(canvas);
		focus(canvas);

		expect(
			shouldIgnoreBoardClipboardEvent({
				target: canvas,
			} as unknown as ClipboardEvent),
		).toBe(false);
	});
});
