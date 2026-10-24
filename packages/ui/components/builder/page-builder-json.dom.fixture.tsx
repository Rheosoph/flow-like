import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type {
	IPage,
	IWidgetRef,
	PageListItem,
} from "../../state/backend-state/page-state";
import type { SurfaceComponent } from "../a2ui/types";
import type { MonacoCodeEditorProps } from "../ui/monaco-code-editor";
import type { BuilderContextType } from "./BuilderContext";

const window = new Window({ url: "https://builder.local/page-builder" });
const ResizeObserver = class {
	observe() {}
	unobserve() {}
	disconnect() {}
};
Object.assign(window, { SyntaxError, TypeError, Error, ResizeObserver });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLButtonElement: window.HTMLButtonElement,
	SVGElement: window.SVGElement,
	Element: window.Element,
	Node: window.Node,
	NodeFilter: window.NodeFilter,
	DocumentFragment: window.DocumentFragment,
	DOMRect: window.DOMRect,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	FocusEvent: window.FocusEvent,
	KeyboardEvent: window.KeyboardEvent,
	MouseEvent: window.MouseEvent,
	PointerEvent: window.PointerEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: (id: number) => clearTimeout(id),
	ResizeObserver,
	IS_REACT_ACT_ENVIRONMENT: true,
});

const page: IPage = {
	id: "page",
	name: "JSON page",
	layoutType: "freeform",
	content: [],
	components: [
		{
			id: "root",
			component: {
				type: "column",
				children: { explicitList: ["title"] },
			},
		},
		{
			id: "title",
			component: { type: "text", content: { literalString: "Original title" } },
		},
	],
	canvasSettings: { backgroundColor: "#ffffff", padding: "16px" },
	createdAt: "2026-10-01T00:00:00Z",
	updatedAt: "2026-10-01T00:00:00Z",
};
const widgetRef: IWidgetRef = {
	id: "widget",
	name: "Referenced widget",
	rootComponentId: "widget-root",
	components: [
		{
			id: "widget-root",
			component: { type: "text", content: { literalString: "Widget title" } },
		},
	],
	tags: [],
	createdAt: page.createdAt,
	updatedAt: page.updatedAt,
};
const writes: IPage[] = [];
const completedWrites: IPage[] = [];
const pendingSaves: (() => void)[] = [];
const pendingBootstrapReads: (() => void)[] = [];
let holdSaves = false;
let holdBootstrapReads = false;
let saveError: Error | undefined;
let builder: BuilderContextType | undefined;
let editor: MonacoCodeEditorProps | undefined;
let storedPage = structuredClone(page);
const reads: string[] = [];

function pageList(appId: string, source: IPage): PageListItem[] {
	return [
		{
			appId,
			pageId: source.id,
			name: source.name,
			updatedAt: source.updatedAt,
		},
	];
}

function pageBootstrap(source: IPage) {
	return { page: structuredClone(source), revision: source.updatedAt };
}

const backend = {
	pageState: {
		getPages: async (appId: string, _boardId?: string) => {
			reads.push(`getPages:${appId}`);
			return pageList(appId, appId === "app" ? storedPage : page);
		},
		getPage: async (appId: string, _pageId: string, _boardId?: string) => {
			reads.push(`getPage:${appId}`);
			return structuredClone(appId === "app" ? storedPage : page);
		},
		getPageBootstrap: async (
			appId: string,
			_route?: string,
			_eventId?: string,
		) => {
			reads.push(`getPageBootstrap:${appId}`);
			const snapshot = pageBootstrap(appId === "app" ? storedPage : page);
			if (holdBootstrapReads) {
				await new Promise<void>((resolve) =>
					pendingBootstrapReads.push(resolve),
				);
			}
			return snapshot;
		},
		updatePage: async (_appId: string, updated: IPage) => {
			writes.push(structuredClone(updated));
			if (holdSaves) {
				await new Promise<void>((resolve) => pendingSaves.push(resolve));
			}
			if (saveError) throw saveError;
			storedPage = structuredClone(updated);
			completedWrites.push(structuredClone(updated));
		},
	},
	appState: { getApp: async () => ({ frontend: {} }) },
	eventState: { alwaysRemote: false },
	boardState: {
		getBoard: async () => ({ nodes: {} }),
		getCatalog: async () => [],
	},
	storageState: { listStorageItems: async () => [] },
};

const locales = await import("@flow-like/locales");
const backendState = await import("../../state/backend-state");
const runtimeTailwind = await import("../../lib/use-runtime-tailwind");
mock.module("@flow-like/locales", () => ({
	...locales,
	useTranslation: () => ({
		t: (key: string, fallback?: string) => fallback ?? key,
	}),
}));
mock.module("../../state/backend-state", () => ({
	...backendState,
	useBackend: () => backend,
}));
mock.module("./ComponentPalette", () => ({ ComponentPalette: () => null }));
mock.module("./HierarchyTree", () => ({ HierarchyTree: () => null }));
mock.module("./Inspector", () => ({
	Inspector: () => {
		builder = useBuilder();
		return null;
	},
}));
mock.module("./a2ui-copilot", () => ({ A2UICopilot: () => null }));
mock.module("../../lib/use-runtime-tailwind", () => ({
	...runtimeTailwind,
	useRuntimeTailwindStyles: () => {},
}));
mock.module("../ui/monaco-code-editor", () => ({
	MonacoCodeEditor: (props: MonacoCodeEditorProps) => {
		editor = props;
		return <textarea aria-label="Raw JSON" readOnly value={props.value} />;
	},
}));

const { useBuilder } = await import("./BuilderContext");
const { PageBuilderSurface } = await import("./page-builder-surface");
const { useInvoke } = await import("../../hooks/use-invoke");
let root: Root | undefined;
const runtimeRoots: Root[] = [];
let client: QueryClient | undefined;
type DomElement = NonNullable<ReturnType<typeof window.document.querySelector>>;

function find(selector: string, text?: string): DomElement {
	const result = [...window.document.querySelectorAll(selector)].find(
		(element) => text === undefined || element.textContent?.trim() === text,
	);
	if (!result) throw new Error(`Nothing matches ${selector} "${text ?? ""}"`);
	return result;
}

async function settle(check: () => boolean) {
	for (let attempt = 0; attempt < 100; attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 5));
		});
		if (check()) return;
	}
	throw new Error("Page builder did not reach the expected state");
}

async function click(target: DomElement) {
	await act(async () => {
		target.dispatchEvent(
			new window.MouseEvent("click", { bubbles: true, cancelable: true }),
		);
	});
}

async function mount() {
	writes.length = 0;
	completedWrites.length = 0;
	pendingSaves.length = 0;
	holdSaves = false;
	holdBootstrapReads = false;
	saveError = undefined;
	storedPage = structuredClone(page);
	reads.length = 0;
	builder = undefined;
	editor = undefined;
	const mountedClient = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	client = mountedClient;
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
	await act(async () => {
		root?.render(
			<QueryClientProvider client={mountedClient}>
				<PageBuilderSurface appId="app" pageId="page" />
			</QueryClientProvider>,
		);
	});
	await settle(() => !!builder);
	expect(find("[data-canvas-id]").textContent).toContain("Original title");
}

const bootstrapScope = ["viewer", "https://api.local", "profile"];

function seedRuntimeCache(appId = "app") {
	const entries = [
		{ key: ["getPage", appId, "page", "board"], data: structuredClone(page) },
		{ key: ["getPages", appId, "board"], data: pageList(appId, page) },
		{
			key: ["getPageBootstrap", appId, "/reports", null, ...bootstrapScope],
			data: pageBootstrap(page),
		},
	];
	for (const { key, data } of entries) client?.setQueryData(key, data);
	return entries;
}

function RuntimeQueries() {
	const runtimePage = useInvoke(backend.pageState.getPage, backend.pageState, [
		"app",
		"page",
		"board",
	]);
	const runtimePages = useInvoke(
		backend.pageState.getPages,
		backend.pageState,
		["app", "board"],
	);
	const bootstrap = useInvoke(
		backend.pageState.getPageBootstrap,
		backend.pageState,
		["app", "/reports", undefined],
		true,
		bootstrapScope,
		0,
	);
	return (
		<>
			<output data-runtime-page>
				{JSON.stringify(runtimePage.data?.components)}
			</output>
			<output data-runtime-pages>{runtimePages.data?.[0]?.updatedAt}</output>
			<output data-runtime-bootstrap>
				{JSON.stringify(bootstrap.data?.page.components)}
			</output>
		</>
	);
}

async function mountRuntime() {
	const mountedClient = client;
	if (!mountedClient) throw new Error("The page builder did not mount");
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	const runtimeRoot = createRoot(host as unknown as HTMLElement);
	runtimeRoots.push(runtimeRoot);
	await act(async () => {
		runtimeRoot.render(
			<QueryClientProvider client={mountedClient}>
				<RuntimeQueries />
			</QueryClientProvider>,
		);
	});
}

async function draft(title: string) {
	await click(find('button[aria-label="Dev Mode (JSON Editor)"]'));
	await settle(() => !!editor?.value);
	if (!editor) throw new Error("The JSON editor did not mount");
	const data = JSON.parse(editor.value);
	data.components.find(
		(component: SurfaceComponent) => component.id === "title",
	).component.content.literalString = title;
	return data;
}

async function apply(data: unknown) {
	await act(async () => editor?.onChange(JSON.stringify(data)));
	await click(find("button", "Apply Changes"));
}

function hasStatus(status: string) {
	return [...window.document.querySelectorAll("span")].some(
		(element) => element.textContent?.trim() === status,
	);
}

afterEach(async () => {
	holdSaves = false;
	holdBootstrapReads = false;
	saveError = undefined;
	for (const release of pendingSaves.splice(0)) release();
	for (const release of pendingBootstrapReads.splice(0)) release();
	await act(async () => {
		for (const runtimeRoot of runtimeRoots.splice(0)) runtimeRoot.unmount();
	});
	await act(async () => root?.unmount());
	root = undefined;
	client?.clear();
	window.document.body.replaceChildren();
});
afterAll(() => {
	mock.restore();
	window.happyDOM.cancelAsync();
});

test("applying same-id JSON edits updates the edit canvas and saves refs and canvas settings once", async () => {
	await mount();
	const data = await draft("Applied title");
	data.widgetRefs = { instance: widgetRef };
	data.canvasSettings = {
		backgroundColor: "#123456",
		padding: "31px",
		customCss: ".title { color: red; }",
	};
	await apply(data);

	expect(find("[data-canvas-id]").textContent).toContain("Applied title");
	expect(find("[data-canvas-id]").textContent).not.toContain("Original title");
	expect(find("[data-canvas-id]").getAttribute("style")).toContain("31px");
	await settle(() => completedWrites.length === 1);
	expect(writes[0].components).toEqual(data.components);
	expect(writes[0].widgetRefs).toEqual(data.widgetRefs);
	expect(writes[0].canvasSettings).toEqual(data.canvasSettings);
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	expect(hasStatus("Saved")).toBe(true);
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 2200));
	});
	expect(writes).toHaveLength(1);
	expect(hasStatus("Saved")).toBe(true);
	expect(hasStatus("Unsaved changes")).toBe(false);
});

test("opening runtime queries after a JSON save reads the new layout without reloading", async () => {
	await mount();
	seedRuntimeCache();
	const otherAppEntries = seedRuntimeCache("other-app");
	await apply(await draft("Updated runtime title"));
	await settle(() => completedWrites.length === 1 && hasStatus("Saved"));

	await mountRuntime();
	await settle(() => client?.isFetching() === 0);
	expect(find("[data-runtime-page]").textContent).toContain(
		"Updated runtime title",
	);
	expect(find("[data-runtime-pages]").textContent).toBe(
		completedWrites[0].updatedAt,
	);
	expect(find("[data-runtime-bootstrap]").textContent).toContain(
		"Updated runtime title",
	);

	for (const { key, data } of otherAppEntries) {
		expect(client?.getQueryState(key)?.isInvalidated).toBe(false);
		expect(client?.getQueryData(key)).toEqual(data);
	}
	expect(reads.filter((read) => read.endsWith(":other-app"))).toEqual([]);
});

test("a mounted runtime bootstrap refreshes only after the JSON save succeeds", async () => {
	await mount();
	seedRuntimeCache();
	await mountRuntime();
	await settle(() => client?.isFetching() === 0);
	expect(reads.filter((read) => read === "getPageBootstrap:app")).toHaveLength(
		1,
	);
	holdSaves = true;
	await apply(await draft("Live runtime title"));
	await settle(() => pendingSaves.length === 1);
	expect(find("[data-runtime-bootstrap]").textContent).toContain(
		"Original title",
	);
	expect(reads.filter((read) => read === "getPageBootstrap:app")).toHaveLength(
		1,
	);

	await act(async () => pendingSaves.shift()?.());
	await settle(
		() =>
			completedWrites.length === 1 &&
			hasStatus("Saved") &&
			client?.isFetching() === 0,
	);
	expect(find("[data-runtime-bootstrap]").textContent).toContain(
		"Live runtime title",
	);
	expect(reads.filter((read) => read === "getPageBootstrap:app")).toHaveLength(
		2,
	);
});

test("saving replaces an initial runtime bootstrap read already in flight", async () => {
	await mount();
	holdBootstrapReads = true;
	await mountRuntime();
	await settle(() => pendingBootstrapReads.length === 1);
	expect(find("[data-runtime-bootstrap]").textContent).toBe("");

	holdBootstrapReads = false;
	await apply(await draft("New bootstrap title"));
	await settle(
		() =>
			completedWrites.length === 1 &&
			hasStatus("Saved") &&
			client?.isFetching() === 0,
	);
	expect(find("[data-runtime-bootstrap]").textContent).toContain(
		"New bootstrap title",
	);
	await act(async () => pendingBootstrapReads.shift()?.());
	await settle(() => client?.isFetching() === 0);
	expect(find("[data-runtime-bootstrap]").textContent).toContain(
		"New bootstrap title",
	);
	expect(reads.filter((read) => read === "getPageBootstrap:app")).toHaveLength(
		2,
	);
});

test("invalid JSON stays editable and leaves the canvas and saved page untouched", async () => {
	await mount();
	await draft("Not applied");
	await act(async () => editor?.onChange("{ invalid JSON"));
	await click(find("button", "Apply Changes"));
	expect(find('[role="alert"]').textContent).toContain(
		"Could not apply changes",
	);
	expect(window.document.querySelector('[role="dialog"]')).not.toBeNull();
	expect(editor?.value).toBe("{ invalid JSON");
	expect(find("[data-canvas-id]").textContent).toContain("Original title");
	expect(writes).toHaveLength(0);
});

test("canvas-only JSON changes are persisted before either autosave debounce", async () => {
	await mount();
	const data = await draft("Original title");
	data.canvasSettings.padding = "42px";
	await apply(data);
	await settle(() => completedWrites.length === 1);
	expect(writes[0].components).toEqual(data.components);
	expect(writes[0].canvasSettings?.padding).toBe("42px");
	expect(find("[data-canvas-id]").getAttribute("style")).toContain("42px");
	expect(hasStatus("Saved")).toBe(true);
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 2200));
	});
	expect(writes).toHaveLength(1);
	expect(hasStatus("Unsaved changes")).toBe(false);
});

test("applying unchanged JSON closes the editor without leaving the page dirty", async () => {
	await mount();
	const data = await draft("Original title");
	await apply(data);
	expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	await settle(() => !hasStatus("Saving..."));
	expect(hasStatus("Unsaved changes")).toBe(false);
	const immediateWrites = writes.length;
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 2200));
	});
	expect(writes.length).toBeLessThanOrEqual(1);
	expect(writes).toHaveLength(immediateWrites);
	expect(hasStatus("Unsaved changes")).toBe(false);
});

test("a rejected JSON save leaves the applied canvas marked unsaved", async () => {
	await mount();
	const entries = seedRuntimeCache();
	saveError = new Error("Page save failed");
	await apply(await draft("Unsaved applied title"));
	await settle(() => writes.length === 1 && hasStatus("Unsaved changes"));
	expect(completedWrites).toHaveLength(0);
	expect(find("[data-canvas-id]").textContent).toContain(
		"Unsaved applied title",
	);
	expect(hasStatus("Saved")).toBe(false);
	for (const { key, data } of entries) {
		expect(client?.getQueryState(key)?.isInvalidated).toBe(false);
		expect(client?.getQueryData(key)).toEqual(data);
	}
});

test("a failed canvas-only JSON save can be retried from the toolbar", async () => {
	await mount();
	saveError = new Error("Canvas save failed");
	const data = await draft("Original title");
	data.canvasSettings.padding = "52px";
	await apply(data);
	await settle(() => writes.length === 1 && hasStatus("Unsaved changes"));
	expect(completedWrites).toHaveLength(0);
	expect(find("[data-canvas-id]").getAttribute("style")).toContain("52px");

	saveError = undefined;
	await click(find('button[aria-label="Save"]'));
	await settle(() => completedWrites.length === 1);
	expect(writes).toHaveLength(2);
	expect(completedWrites[0].components).toEqual(data.components);
	expect(completedWrites[0].canvasSettings).toEqual(data.canvasSettings);
	expect(hasStatus("Saved")).toBe(true);
	expect(hasStatus("Unsaved changes")).toBe(false);
});

test("canvas edits made during a JSON save become Saved after their metadata autosave finishes", async () => {
	await mount();
	holdSaves = true;
	const data = await draft("Applied title");
	await apply(data);
	await settle(() => pendingSaves.length === 1);
	await act(async () => builder?.setCanvasSettings({ padding: "64px" }));
	await act(async () => pendingSaves.shift()?.());
	await settle(() => completedWrites.length === 1);
	expect(completedWrites[0].canvasSettings).toEqual(data.canvasSettings);
	expect(find("[data-canvas-id]").getAttribute("style")).toContain("64px");
	expect(hasStatus("Unsaved changes")).toBe(true);
	expect(hasStatus("Saved")).toBe(false);

	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 1100));
	});
	await settle(() => pendingSaves.length === 1);
	expect(writes).toHaveLength(2);
	expect(writes[1].canvasSettings?.padding).toBe("64px");
	await act(async () => pendingSaves.shift()?.());
	await settle(() => completedWrites.length === 2);
	expect(completedWrites[1].components).toEqual(data.components);
	expect(completedWrites[1].canvasSettings?.padding).toBe("64px");
	expect(hasStatus("Saved")).toBe(true);
	expect(hasStatus("Unsaved changes")).toBe(false);
});

test("edits made while JSON is saving remain dirty after that save finishes", async () => {
	await mount();
	holdSaves = true;
	const data = await draft("Applied title");
	await apply(data);
	await settle(() => pendingSaves.length === 1);
	await act(async () => {
		builder?.updateComponent("title", {
			component: {
				type: "text",
				content: { literalString: "Later canvas edit" },
			},
		});
	});
	await act(async () => pendingSaves.shift()?.());
	await settle(() => completedWrites.length === 1);
	expect(completedWrites[0].components).toEqual(data.components);
	expect(find("[data-canvas-id]").textContent).toContain("Later canvas edit");
	expect(hasStatus("Unsaved changes")).toBe(true);
	expect(hasStatus("Saved")).toBe(false);
});
