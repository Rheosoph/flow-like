import { afterAll, beforeAll, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement } from "react";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });

// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = [
	"window",
	"document",
	"navigator",
	"HTMLElement",
	"Element",
	"Node",
	"MutationObserver",
	"SVGElement",
	"Event",
	"getComputedStyle",
	"requestAnimationFrame",
	"cancelAnimationFrame",
	"ResizeObserver",
	"IS_REACT_ACT_ENVIRONMENT",
].map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
const actualLocales = { ...(await import("@flow-like/locales")) };

function installDomGlobals() {
	Object.assign(globalThis, {
		window,
		document: window.document,
		navigator: window.navigator,
		HTMLElement: window.HTMLElement,
		Element: window.Element,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		SVGElement: window.SVGElement,
		Event: window.Event,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: (cb: FrameRequestCallback) =>
			setTimeout(() => cb(0), 0),
		cancelAnimationFrame: (id: number) => clearTimeout(id),
		ResizeObserver: class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	});
}
installDomGlobals();

// @ts-expect-error — react-dom checks this flag before touching the DOM.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

const { createRoot } = await import("react-dom/client");
const { AppearancePreview } = await import("./appearance-preview");
const { DEFAULT_APPEARANCE_STATE, buildAppearanceBlock } = await import(
	"../../../lib/appearance/appearance-theme"
);

beforeAll(() => installDomGlobals());
const roots: ReturnType<typeof createRoot>[] = [];
afterAll(async () => {
	await act(async () => {
		for (const root of roots) root.unmount();
	});
	mock.module("@flow-like/locales", () => actualLocales);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

test("the preview carries the scoped sheet and the app surfaces", () => {
	const container = window.document.createElement(
		"div",
	) as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	const root = createRoot(container);
	roots.push(root);
	const sheet = buildAppearanceBlock(DEFAULT_APPEARANCE_STATE, {
		mode: "dark",
		boost: true,
	});
	act(() =>
		root.render(
			createElement(AppearancePreview, {
				view: "dashboard",
				mode: "dark",
				sheet,
			}),
		),
	);

	const style = container.querySelector("style")?.textContent ?? "";
	expect(style).toContain('[data-appearance-preview="1"]');
	expect(style).toContain("--primary");
	expect(
		container.querySelectorAll('[data-slot="card"]').length,
	).toBeGreaterThan(3);
	expect(container.querySelector('[data-slot="skeleton"]')).not.toBeNull();
	expect(container.textContent).toContain("Runs per hour");
});
