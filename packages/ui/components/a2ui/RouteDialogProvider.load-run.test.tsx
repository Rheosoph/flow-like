import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { PageSurfaceIdentity } from "../../lib/page-surface-cache";
import type {
	IPage,
	IPageBootstrap,
} from "../../state/backend-state/page-state";
import type { FrontendStateStore } from "./frontend-state";
import type { Surface } from "./types";

type RunEvents = { event_type: string; payload: unknown }[];
type Run = {
	onStarted?: (runId: string) => void;
	onEvents: (events: RunEvents) => void;
	resolve: () => void;
};

const runs: Run[] = [];
const cancelExecution = mock(async (_runId: string): Promise<void> => {});
let pendingBootstrap: Promise<IPageBootstrap> | null = null;
let renderedOnA2UIMessage: ((message: Record<string, unknown>) => void) | null =
	null;
const readCachedSurface = mock(
	async (): Promise<Surface | null> => null as Surface | null,
);
const writeCachedSurface = mock(
	async (_identity: PageSurfaceIdentity | null, _surface: Surface) => {},
);

// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll.
const actual = {
	frontendState: { ...(await import("./frontend-state")) },
	pageSurfaceCache: { ...(await import("../../lib/page-surface-cache")) },
	locales: { ...(await import("@flow-like/locales")) },
	oidc: { ...(await import("react-oidc-context")) },
	backendState: { ...(await import("../../state/backend-state")) },
	dialog: { ...(await import("../ui/dialog")) },
	renderer: { ...(await import("./A2UIRenderer")) },
	collectRunElements: { ...(await import("./collect-run-elements")) },
	pageLoadingSkeleton: {
		...(await import("../interfaces/page-loading-skeleton")),
	},
};

const persistence = {
	getAll: async () => ({}),
	set: async () => {},
	clearPage: async () => {},
};
const testStores = new Map<string | undefined, FrontendStateStore>();
mock.module("./frontend-state", () => ({
	...actual.frontendState,
	getFrontendStateStore: (appId: string | undefined) => {
		let store = testStores.get(appId);
		if (!store) {
			store = actual.frontendState.createFrontendStateStore(appId, {
				global: persistence,
				page: persistence,
			});
			testStores.set(appId, store);
		}
		return store;
	},
}));
mock.module("../../lib/page-surface-cache", () => ({
	...actual.pageSurfaceCache,
	readPageSurfaceCache: readCachedSurface,
	writePageSurfaceCache: writeCachedSurface,
}));
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => null,
}));
const page: IPage = {
	id: "dialog-page",
	name: "Dialog page",
	components: [
		{
			id: "root",
			component: {
				id: "root",
				type: "column",
				children: { explicitList: ["headline"] },
			},
		},
		{
			id: "headline",
			component: {
				id: "headline",
				type: "text",
				content: { literalString: "Static headline" },
			},
		},
	],
	content: [],
	layoutType: "stack",
	createdAt: "2026-09-23",
	updatedAt: "2026-09-23",
	onLoadEventId: "load-node",
};
const bootstrapFor = (served: IPage) =>
	({
		event: {
			id: "dialog-event",
			board_id: "dialog-board",
			default_page_id: served.id,
		},
		page: served,
		revision: "page-v1",
		executionRevision: "execution-v1",
	}) as unknown as IPageBootstrap;
const bootstrap = bootstrapFor(page);
let servedBootstrap = bootstrap;
const backend = {
	pageState: {
		getPageBootstrap: async () => pendingBootstrap ?? servedBootstrap,
	},
	eventState: {
		executeEvent: (
			_appId: string,
			_eventId: string,
			_payload: unknown,
			_stream: boolean,
			onStarted: Run["onStarted"],
			onEvents: Run["onEvents"],
		) =>
			new Promise<void>((resolve) => {
				runs.push({ onStarted, onEvents, resolve });
			}),
		cancelExecution,
	},
};
mock.module("../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => backend,
}));
const childrenOnly = ({ children }: { children: ReactNode }) => children;
mock.module("../ui/dialog", () => ({
	...actual.dialog,
	Dialog: ({ open, children }: { open: boolean; children: ReactNode }) =>
		open ? children : null,
	DialogContent: ({ children }: { children: ReactNode }) => (
		<div data-dialog-content="">{children}</div>
	),
	DialogHeader: childrenOnly,
	DialogTitle: childrenOnly,
}));
mock.module("./A2UIRenderer", () => ({
	...actual.renderer,
	A2UIRenderer: ({
		surface,
		onA2UIMessage,
	}: {
		surface: Surface;
		onA2UIMessage: (message: Record<string, unknown>) => void;
	}) => {
		renderedOnA2UIMessage = onA2UIMessage;
		return (
			<div
				data-rendered-components={Object.keys(surface.components)
					.sort()
					.join(" ")}
			/>
		);
	},
}));
mock.module("./collect-run-elements", () => ({
	...actual.collectRunElements,
	collectRunElements: async () => ({}),
}));
mock.module("../interfaces/page-loading-skeleton", () => ({
	...actual.pageLoadingSkeleton,
	PageLoadingSkeleton: ({ title }: { title?: string }) => (
		<div data-page-skeleton="">{title}</div>
	),
}));

const { RouteDialogProvider, useRouteDialog } = await import(
	"./RouteDialogProvider"
);

let root: Root | undefined;
let host: HTMLElement | undefined;
let restoreGlobals: (() => void) | undefined;
let dialogApi: ReturnType<typeof useRouteDialog> | undefined;

function DialogApi() {
	dialogApi = useRouteDialog();
	return null;
}

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	host = undefined;
	await flushDisposal();
	restoreGlobals?.();
	restoreGlobals = undefined;
	runs.length = 0;
	pendingBootstrap = null;
	servedBootstrap = bootstrap;
	renderedOnA2UIMessage = null;
	dialogApi = undefined;
	cancelExecution.mockClear();
	readCachedSurface.mockReset();
	readCachedSurface.mockImplementation(async () => null);
	writeCachedSurface.mockClear();
});
afterAll(() => {
	mock.restore();
	mock.module("./frontend-state", () => actual.frontendState);
	mock.module("../../lib/page-surface-cache", () => actual.pageSurfaceCache);
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module("../../state/backend-state", () => actual.backendState);
	mock.module("../ui/dialog", () => actual.dialog);
	mock.module("./A2UIRenderer", () => actual.renderer);
	mock.module("./collect-run-elements", () => actual.collectRunElements);
	mock.module(
		"../interfaces/page-loading-skeleton",
		() => actual.pageLoadingSkeleton,
	);
});

async function flushDisposal() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function mountProvider(appId: string) {
	const window = new Window({ url: "https://example.test/use" });
	Object.assign(window, { SyntaxError });
	const globals = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const previous = Object.fromEntries(
		Object.keys(globals).map((key) => [
			key,
			Object.getOwnPropertyDescriptor(globalThis, key),
		]),
	);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const [key, descriptor] of Object.entries(previous)) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	const container = window.document.createElement("div");
	window.document.body.append(container);
	host = container as unknown as HTMLElement;
	root = createRoot(host);
	await act(async () => {
		root?.render(
			<RouteDialogProvider appId={appId}>
				<DialogApi />
			</RouteDialogProvider>,
		);
	});
}

async function openDetails() {
	await act(async () => dialogApi?.openDialog("/details", "Details", {}, "d1"));
}

const renderedComponents = () =>
	host
		?.querySelector("[data-rendered-components]")
		?.getAttribute("data-rendered-components") ?? null;
const loadIndicator = () =>
	host?.querySelector("[data-page-load-indicator]") ?? null;
const loadStatus = () => host?.querySelector("output") ?? null;
const busyRegion = () => host?.querySelector('[aria-busy="true"]') ?? null;
const skeleton = () => host?.querySelector("[data-page-skeleton]") ?? null;
const freshMessage = {
	type: "upsertElement",
	element_id: "dialog-page/fresh",
	value: {
		type: "createComponent",
		component: {
			id: "fresh",
			type: "text",
			content: { literalString: "Fresh" },
		},
	},
};
const a2ui = (payload: Record<string, unknown>): RunEvents => [
	{ event_type: "a2ui", payload },
];

describe("route dialog pages render their static layout first", () => {
	test("the skeleton only holds while the dialog's page is fetched", async () => {
		let release: (value: IPageBootstrap) => void = () => {};
		pendingBootstrap = new Promise((resolve) => {
			release = resolve;
		});
		await mountProvider("dialog-fetch");
		await openDetails();
		expect(skeleton()?.textContent).toBe("Loading page");
		expect(renderedComponents()).toBeNull();

		await act(async () => release(bootstrap));
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
		expect(runs).toHaveLength(1);
	});

	test("only the load run's own output clears the indicator", async () => {
		await mountProvider("dialog-render-first");
		await openDetails();
		expect(renderedComponents()).toBe("headline root");
		expect(loadIndicator()).not.toBeNull();
		expect(busyRegion()).not.toBeNull();
		const status = loadStatus();
		expect(status?.textContent).toBe("Loading page data…");
		expect(status?.closest("[aria-busy]")).toBeNull();

		await act(async () => renderedOnA2UIMessage?.(freshMessage));
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).not.toBeNull();

		await act(async () => runs[0].onEvents(a2ui({ type: "showScreen" })));
		expect(loadIndicator()).toBeNull();
		expect(busyRegion()).toBeNull();
		expect(loadStatus()).toBe(status);
		expect(status?.textContent).toBe("");
	});

	test("the indicator leaves when the load run ends without output", async () => {
		await mountProvider("dialog-silent");
		await openDetails();
		expect(loadIndicator()).not.toBeNull();
		await act(async () => runs[0].resolve());
		expect(loadIndicator()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
	});
});

describe("route dialog page surface cache", () => {
	const cachedSurface = (): Surface =>
		({
			id: page.id,
			rootComponentId: "root",
			components: {
				root: page.components[0],
				headline: page.components[1],
				cached: {
					id: "cached",
					component: {
						id: "cached",
						type: "text",
						content: { literalString: "Last visit" },
					},
				},
			},
		}) as Surface;

	test("a default dialog page replays its cached surface, refreshes it in place and stores it after success", async () => {
		let releaseCache: (surface: Surface | null) => void = () => {};
		readCachedSurface.mockImplementation(
			() =>
				new Promise<Surface | null>((resolve) => {
					releaseCache = resolve;
				}),
		);
		await mountProvider("dialog-cache");
		await openDetails();
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("headline root");

		await act(async () => releaseCache(cachedSurface()));
		expect(renderedComponents()).toBe("cached headline root");

		await act(async () => runs[0].onEvents(a2ui(freshMessage)));
		expect(renderedComponents()).toBe("cached fresh headline root");
		expect(writeCachedSurface).not.toHaveBeenCalled();

		await act(async () => runs[0].resolve());
		expect(writeCachedSurface).toHaveBeenCalledTimes(1);
		expect(writeCachedSurface.mock.calls[0][0]?.routeKey).toBe("/details");
	});

	test("a noCache dialog page shows the loading screen until its load run renders and never touches the cache", async () => {
		servedBootstrap = bootstrapFor({ ...page, noCache: true });
		await mountProvider("dialog-no-cache");
		await openDetails();
		expect(skeleton()?.textContent).toBe("Running workflow");
		expect(renderedComponents()).toBeNull();
		expect(loadIndicator()).toBeNull();

		await act(async () => runs[0].onEvents(a2ui(freshMessage)));
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("fresh headline root");

		await act(async () => runs[0].resolve());
		expect(readCachedSurface).not.toHaveBeenCalled();
		expect(writeCachedSurface).not.toHaveBeenCalled();
	});
});

describe("route dialog load run ownership", () => {
	test("closing the dialog cancels its load run and drops its later messages", async () => {
		const appId = "dialog-close";
		await mountProvider(appId);
		await openDetails();
		await act(async () => runs[0].onStarted?.("run-dialog"));

		await act(async () => dialogApi?.closeDialog("d1"));
		expect(cancelExecution).not.toHaveBeenCalled();
		await flushDisposal();
		expect(cancelExecution.mock.calls).toEqual([["run-dialog"]]);

		const before = testStores.get(appId)?.getSnapshot();
		expect(before).toBeDefined();
		runs[0].onEvents(a2ui({ type: "setGlobalState", key: "late", value: 1 }));
		expect(testStores.get(appId)?.getSnapshot()).toBe(before);
	});

	test("a run id that arrives after the dialog closed is cancelled on arrival", async () => {
		await mountProvider("dialog-late-id");
		await openDetails();
		await act(async () => dialogApi?.closeDialog("d1"));
		await flushDisposal();
		expect(cancelExecution).not.toHaveBeenCalled();
		runs[0].onStarted?.("run-late");
		expect(cancelExecution.mock.calls).toEqual([["run-late"]]);
	});

	test("a host whose cancel throws synchronously does not break the close", async () => {
		cancelExecution.mockImplementationOnce(() => {
			throw new Error("Method not implemented.");
		});
		await mountProvider("dialog-sync-throw");
		await openDetails();
		await act(async () => runs[0].onStarted?.("run-throw"));
		await act(async () => dialogApi?.closeDialog("d1"));
		await flushDisposal();
		expect(cancelExecution.mock.calls).toEqual([["run-throw"]]);
	});
});
