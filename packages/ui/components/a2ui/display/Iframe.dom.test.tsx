import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { IframeComponent } from "../types";

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;
let restoreBackend: (() => void) | undefined;
const cleanup: (() => void)[] = [];

const HASH = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

beforeEach(async () => {
	restoreBackend = undefined;
	window = new Window({ url: "https://app.flow-like.com/use/page" });
	const globals = {
		document: window.document,
		HTMLElement: window.HTMLElement,
		HTMLIFrameElement: window.HTMLIFrameElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		navigator: window.navigator,
		MutationObserver: window.MutationObserver,
		Event: window.Event,
		getComputedStyle: window.getComputedStyle.bind(window),
		window,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	Object.assign(window, { SyntaxError, TypeError });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	// Frames never navigate in these tests; the attribute is what is asserted.
	const framePrototype = window.HTMLIFrameElement.prototype;
	const src = Object.getOwnPropertyDescriptor(framePrototype, "src");
	if (src) {
		Object.defineProperty(framePrototype, "src", {
			...src,
			get: () => "about:blank",
		});
		cleanup.push(() => Object.defineProperty(framePrototype, "src", src));
	}

	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	for (const restore of cleanup.splice(0).reverse()) restore();
	restoreBackend?.();
	await window.happyDOM.abort();
	restoreGlobals();
});

const literal = (value: string) => ({ literalString: value });

async function installBackend(signedUrl: (path: string) => string) {
	const { useBackendStore } = await import("../../../state/backend-state");
	const previous = useBackendStore.getState().backend;
	restoreBackend = () => useBackendStore.setState({ backend: previous });
	useBackendStore.getState().setBackend({
		eventState: {},
		boardState: {},
		storageState: {
			downloadStorageItems: async (_appId: string, paths: string[]) =>
				paths.map((prefix) => ({ prefix, url: signedUrl(prefix) })),
		},
	} as never);
}

async function inPage(children: ReactNode) {
	const [
		{ createElement },
		{ ActionProvider },
		{ QueryClient, QueryClientProvider },
		{ AppRouterContext },
		{ appGlobalState, pageLocalState },
		uiState,
	] = await Promise.all([
		import("react"),
		import("../ActionHandler"),
		import("@tanstack/react-query"),
		import("next/dist/shared/lib/app-router-context.shared-runtime"),
		import("../../../lib/idb-storage"),
		import("../../../db/ui-state-db"),
	]);
	const spies = [
		spyOn(appGlobalState, "getAll").mockResolvedValue({}),
		spyOn(pageLocalState, "getAll").mockResolvedValue({}),
		spyOn(uiState.uiElementValues, "getAll").mockResolvedValue({}),
		spyOn(uiState, "pruneElementValues").mockResolvedValue(),
	];
	cleanup.push(() => {
		for (const spy of spies) spy.mockRestore();
	});
	const client = new QueryClient();
	cleanup.push(() => client.clear());
	return createElement(
		AppRouterContext.Provider,
		{ value: {} as never },
		createElement(
			QueryClientProvider,
			{ client } as never,
			createElement(
				ActionProvider,
				{
					surfaceId: "page-1",
					appId: "app-1",
					isPreviewMode: true,
					components: {},
				} as never,
				children,
			),
		),
	);
}

async function renderIframe(
	overrides: Partial<IframeComponent>,
	options: { page?: boolean } = {},
) {
	const [{ act, createElement }, { A2UIIframe }, { DataProvider }] =
		await Promise.all([
			import("react"),
			import("./Iframe"),
			import("../DataContext"),
		]);
	const element = createElement(DataProvider, {
		initialData: [],
		children: createElement(A2UIIframe, {
			component: { id: "frame", type: "iframe", ...overrides },
			componentId: "frame",
			surfaceId: "page-1",
		} as never),
	});
	const tree = options.page ? await inPage(element) : element;
	await act(async () => {
		root.render(tree);
	});
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 5));
	});
}

const frame = () => host.querySelector("iframe");
const blockedCard = () => host.querySelector("[data-iframe-blocked]");

describe("widget serving URLs", () => {
	for (const src of [
		`flow-widget://localhost/com.example.maps/${HASH}/frame/live-map/0`,
		`flow-widget://localhost/com.example.maps/${HASH}/widgets/live-map/index.0.html`,
		`http://flow-widget.localhost/com.example.maps/${HASH}/frame/live-map/0`,
		`https://flow-widget.localhost/com.example.maps/${HASH}/shared/x.svg`,
		"https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/frame/live-map/0",
		"https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-asset/1.2.0/widgets/live-map/index.html",
		"/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.0.html",
		"javascript:alert(document.domain)",
		"data:text/html,<script>parent.postMessage(1,'*')</script>",
		"blob:https://app.flow-like.com/5b8a",
		"tauri://localhost/index.html",
		"file:///etc/passwd",
	]) {
		test(`refuses ${src.slice(0, 60)}`, async () => {
			await renderIframe({ src: literal(src) });
			expect(frame()).toBeNull();
			expect(blockedCard()).not.toBeNull();
			expect(host.textContent).toContain("cannot be embedded");
		});
	}

	test("refuses a storage path that resolves to a widget document", async () => {
		await installBackend(
			() =>
				"https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/widgets/live-map/index.0.html",
		);
		await renderIframe({ src: literal("uploads/embed.html") }, { page: true });
		expect(frame()).toBeNull();
		expect(blockedCard()).not.toBeNull();
	});

	test("a storage path that resolves to an ordinary signed URL still renders", async () => {
		await installBackend(
			(path) => `https://storage.example.com/${path}?X-Amz-Expires=3600`,
		);
		await renderIframe({ src: literal("uploads/report.html") }, { page: true });
		expect(blockedCard()).toBeNull();
		expect(frame()?.getAttribute("src")).toBe(
			"https://storage.example.com/uploads/report.html?X-Amz-Expires=3600",
		);
	});
});

describe("ordinary embeds", () => {
	test("https pages render with the default sandbox", async () => {
		await renderIframe({ src: literal("https://www.youtube.com/embed/abc") });
		expect(blockedCard()).toBeNull();
		expect(frame()?.getAttribute("src")).toBe(
			"https://www.youtube.com/embed/abc",
		);
		expect(frame()?.getAttribute("sandbox")).toContain("allow-same-origin");
	});

	test("an author sandbox is kept for src embeds", async () => {
		await renderIframe({
			src: literal("https://example.com/"),
			sandbox: literal("allow-scripts allow-same-origin"),
		});
		expect(frame()?.getAttribute("sandbox")).toBe(
			"allow-scripts allow-same-origin",
		);
	});
});

describe("srcdoc", () => {
	test("never keeps allow-same-origin", async () => {
		await renderIframe({
			srcdoc: literal("<p>hello</p>"),
			sandbox: literal(
				"allow-scripts ALLOW-SAME-ORIGIN allow-forms  allow-same-origin",
			),
		});
		expect(frame()?.getAttribute("sandbox")).toBe("allow-scripts allow-forms");
		expect(frame()?.getAttribute("srcdoc")).toBe("<p>hello</p>");
	});

	test("uses the scripts-only default and ignores a widget src next to it", async () => {
		await renderIframe({
			srcdoc: literal("<p>hello</p>"),
			src: literal(
				`flow-widget://localhost/com.example.maps/${HASH}/frame/live-map/0`,
			),
		});
		expect(blockedCard()).toBeNull();
		expect(frame()?.getAttribute("sandbox")).toBe("allow-scripts");
		expect(frame()?.hasAttribute("src")).toBe(false);
	});

	test("an empty author sandbox stays fully restrictive", async () => {
		await renderIframe({
			srcdoc: literal("<p>hello</p>"),
			sandbox: literal("allow-same-origin"),
		});
		expect(frame()?.getAttribute("sandbox")).toBe("");
	});
});

describe("isBlockedIframeSrc", () => {
	test("checks the raw and the resolved address", async () => {
		const { isBlockedIframeSrc } = await import("./Iframe");
		const base = "https://app.flow-like.com/use";
		expect(
			isBlockedIframeSrc(
				`flow-widget://localhost/p/${HASH}/frame/w/0`,
				undefined,
				base,
			),
		).toBe(true);
		expect(
			isBlockedIframeSrc(
				"uploads/x.html",
				"https://api.flow-like.com/api/v1/registry/package/p/widget-asset/1/frame/w",
				base,
			),
		).toBe(true);
		expect(isBlockedIframeSrc("uploads/x.html", undefined, base)).toBe(false);
		expect(
			isBlockedIframeSrc(
				"uploads/x.html",
				"uploads/x.html",
				"tauri://localhost/",
			),
		).toBe(true);
		expect(
			isBlockedIframeSrc("https://example.com", "https://example.com", base),
		).toBe(false);
	});
});
