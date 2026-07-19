import { Window } from "happy-dom";
import { ThemeProvider } from "next-themes";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, test } from "vitest";

const THEME_SCRIPT_ID = "next-themes-bootstrap-test";
const THEME_CHILD_ID = "next-themes-child-test";

const browserGlobalKeys = [
	"window",
	"document",
	"HTMLElement",
	"Node",
	"navigator",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"IS_REACT_ACT_ENVIRONMENT",
] as const;

function installBrowserGlobals(window: Window): () => void {
	const previous = new Map(
		browserGlobalKeys.map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	const values: Record<(typeof browserGlobalKeys)[number], unknown> = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};

	for (const key of browserGlobalKeys) {
		Object.defineProperty(globalThis, key, {
			configurable: true,
			writable: true,
			value: values[key],
		});
	}

	return () => {
		for (const key of browserGlobalKeys) {
			const descriptor = previous.get(key);
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
}

function themeTree() {
	return createElement(
		ThemeProvider,
		{
			attribute: "class",
			defaultTheme: "system",
			enableSystem: true,
			storageKey: "theme",
			scriptProps: { id: THEME_SCRIPT_ID },
		},
		createElement("div", { id: THEME_CHILD_ID }, "Theme content"),
	);
}

describe("patched next-themes bootstrap script", () => {
	test("renders during SSR but never during a client mount or remount", async () => {
		const serverMarkup = renderToStaticMarkup(themeTree());

		expect(serverMarkup).toContain(`<script id="${THEME_SCRIPT_ID}"`);
		expect(serverMarkup).toContain("localStorage.getItem");
		expect(serverMarkup).toContain(`<div id="${THEME_CHILD_ID}">`);

		const browserWindow = new Window({ url: "http://localhost" });
		// Bun's Happy DOM window does not currently mirror the host SyntaxError
		// constructor, but its selector parser expects one on the window object.
		Object.defineProperty(browserWindow, "SyntaxError", {
			configurable: true,
			value: SyntaxError,
		});
		const restoreGlobals = installBrowserGlobals(browserWindow);
		const container = browserWindow.document.createElement("div");
		browserWindow.document.body.append(container);

		try {
			for (let mount = 0; mount < 2; mount += 1) {
				const root = createRoot(container as unknown as Element);
				await act(async () => root.render(themeTree()));

				expect(container.querySelector(`#${THEME_CHILD_ID}`)).not.toBeNull();
				expect(container.querySelector(`#${THEME_SCRIPT_ID}`)).toBeNull();

				await act(async () => root.unmount());
			}
		} finally {
			container.remove();
			browserWindow.close();
			restoreGlobals();
		}
	});
});
