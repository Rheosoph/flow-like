import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, StrictMode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { PageSurfaceIdentity } from "../../lib/page-surface-cache";
import type { IRunTimingReport } from "../../lib/run-timing";
import type { IEvent } from "../../lib/schema/flow/event";
import type { IPage } from "../../state/backend-state/page-state";
import type { FrontendStateStore } from "../a2ui/frontend-state";
import type { Surface } from "../a2ui/types";

type RunEvents = { event_type: string; payload: unknown }[];
type Run = {
	payload: { id: string; payload: Record<string, unknown> };
	onStarted?: (runId: string) => void;
	onEvents: (events: RunEvents) => void;
	resolve: () => void;
	reject: (error: Error) => void;
};

const runs: Run[] = [];
const intervals = new Set<() => void>();
const captureReadiness: boolean[] = [];
const cancelExecution = mock(async (_runId: string): Promise<void> => {});
let renderedOnA2UIMessage: ((message: Record<string, unknown>) => void) | null =
	null;
const handleElementsRequest = mock(
	(message: { type?: string }) => message.type === "requestElements",
);
const readCachedSurface = mock(
	async (): Promise<Surface | null> => null as Surface | null,
);
const writeCachedSurface = mock(
	async (_identity: PageSurfaceIdentity | null, _surface: Surface) => {},
);

// bun keeps a module mock for every later file in the process, so each mocked module is
// captured first and put back in afterAll. frontend-state binds its default persistence when it
// is first evaluated, often by an earlier file, so the stores get theirs injected.
const actual = {
	frontendState: { ...(await import("../a2ui/frontend-state")) },
	pageSurfaceCache: { ...(await import("../../lib/page-surface-cache")) },
	locales: { ...(await import("@flow-like/locales")) },
	nextNavigation: { ...(await import("next/navigation")) },
	oidc: { ...(await import("react-oidc-context")) },
	assetSource: { ...(await import("../../hooks/use-asset-source")) },
	backendState: { ...(await import("../../state/backend-state")) },
	executionService: {
		...(await import("../../state/execution-service-context")),
	},
	renderer: { ...(await import("../a2ui/A2UIRenderer")) },
	dataContext: { ...(await import("../a2ui/DataContext")) },
	livePageAgentBridge: { ...(await import("../a2ui/LivePageAgentBridge")) },
	routeDialog: { ...(await import("../a2ui/RouteDialogProvider")) },
	collectRunElements: { ...(await import("../a2ui/collect-run-elements")) },
	elementsRequestHandler: {
		...(await import("../a2ui/elements-request-handler")),
	},
	widgetQueryHandler: { ...(await import("../a2ui/widget-query-handler")) },
	scopedCustomCss: { ...(await import("../scoped-custom-css")) },
	nativeWidgetPageCapture: {
		...(await import("./native-widget-page-capture")),
	},
	pageLoadingSkeleton: { ...(await import("./page-loading-skeleton")) },
};

const persistence = {
	getAll: async () => ({}),
	set: async () => {},
	clearPage: async () => {},
};
const testStores = new Map<string | undefined, FrontendStateStore>();
mock.module("../a2ui/frontend-state", () => ({
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
const router = { push: () => {}, replace: () => {} };
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	useRouter: () => router,
	useSearchParams: () => new URLSearchParams(),
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => null,
}));
mock.module("../../hooks/use-asset-source", () => ({
	...actual.assetSource,
	useAssetSource: () => ({ src: undefined }),
}));
const backend = {
	eventState: {
		executeEvent: (
			_appId: string,
			_eventId: string,
			payload: Run["payload"],
			_stream: boolean,
			onStarted: Run["onStarted"],
			onEvents: Run["onEvents"],
		) =>
			new Promise<void>((resolve, reject) => {
				runs.push({ payload, onStarted, onEvents, resolve, reject });
			}),
		cancelExecution,
	},
};
mock.module("../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => backend,
}));
mock.module("../../state/execution-service-context", () => ({
	...actual.executionService,
	useExecutionServiceOptional: () => null,
}));
const childrenOnly = ({ children }: { children: ReactNode }) => children;
mock.module("../a2ui/A2UIRenderer", () => ({
	...actual.renderer,
	A2UIRenderer: ({
		surface,
		agentBridge,
		onA2UIMessage,
	}: {
		surface: Surface;
		agentBridge: ReactNode;
		onA2UIMessage: (message: Record<string, unknown>) => void;
	}) => {
		renderedOnA2UIMessage = onA2UIMessage;
		return (
			<div
				data-rendered-components={Object.keys(surface.components)
					.sort()
					.join(" ")}
			>
				{agentBridge}
			</div>
		);
	},
}));
mock.module("../a2ui/DataContext", () => ({
	...actual.dataContext,
	DataProvider: childrenOnly,
}));
mock.module("../a2ui/LivePageAgentBridge", () => ({
	...actual.livePageAgentBridge,
	LivePageAgentBridge: () => null,
}));
mock.module("./native-widget-page-capture", () => ({
	...actual.nativeWidgetPageCapture,
	NativeWidgetPageCaptureBridge: ({ ready }: { ready: boolean }) => {
		captureReadiness.push(ready);
		return null;
	},
}));
const dialogs = { openDialog: () => {}, closeDialog: () => {} };
mock.module("../a2ui/RouteDialogProvider", () => ({
	...actual.routeDialog,
	RouteDialogProvider: childrenOnly,
	useRouteDialog: () => dialogs,
}));
mock.module("../a2ui/collect-run-elements", () => ({
	...actual.collectRunElements,
	collectRunElements: async () => ({}),
}));
mock.module("../a2ui/elements-request-handler", () => ({
	...actual.elementsRequestHandler,
	handleElementsRequestMessage: handleElementsRequest,
}));
mock.module("../a2ui/widget-query-handler", () => ({
	...actual.widgetQueryHandler,
	handleWidgetQueryMessage: () => false,
}));
mock.module("../scoped-custom-css", () => ({
	...actual.scopedCustomCss,
	ScopedCustomCss: () => null,
}));
mock.module("./page-loading-skeleton", () => ({
	...actual.pageLoadingSkeleton,
	PageLoadingSkeleton: ({ title }: { title?: string }) => (
		<div data-page-skeleton="">{title}</div>
	),
}));

const { PageInterface } = await import("./page-interface");
const { STALE_SURFACE_SETTLE_MS } = await import("./use-page-surface-cache");
const { getFrontendStateStore } = await import("../a2ui/frontend-state");
const { notifyLivePageRun } = await import("../a2ui/live-page-registry");
const { resetRunTiming } = await import("../../lib/run-timing");

let root: Root | undefined;
let host: HTMLElement | undefined;
let restoreGlobals: (() => void) | undefined;
let silenceInfo: ReturnType<typeof spyOn> | undefined;

beforeEach(() => {
	resetRunTiming();
	Reflect.deleteProperty(globalThis, "__flowLikeRunTimings");
	silenceInfo = spyOn(console, "info").mockImplementation(() => {});
});
afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	host = undefined;
	await flushDisposal();
	restoreGlobals?.();
	restoreGlobals = undefined;
	silenceInfo?.mockRestore();
	resetRunTiming();
	runs.length = 0;
	intervals.clear();
	captureReadiness.length = 0;
	renderedOnA2UIMessage = null;
	cancelExecution.mockReset();
	cancelExecution.mockImplementation(async () => {});
	handleElementsRequest.mockClear();
	readCachedSurface.mockReset();
	readCachedSurface.mockImplementation(async () => null);
	writeCachedSurface.mockClear();
});
afterAll(() => {
	mock.restore();
	mock.module("../a2ui/frontend-state", () => actual.frontendState);
	mock.module("../../lib/page-surface-cache", () => actual.pageSurfaceCache);
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module("../../hooks/use-asset-source", () => actual.assetSource);
	mock.module("../../state/backend-state", () => actual.backendState);
	mock.module(
		"../../state/execution-service-context",
		() => actual.executionService,
	);
	mock.module("../a2ui/A2UIRenderer", () => actual.renderer);
	mock.module("../a2ui/DataContext", () => actual.dataContext);
	mock.module("../a2ui/LivePageAgentBridge", () => actual.livePageAgentBridge);
	mock.module("../a2ui/RouteDialogProvider", () => actual.routeDialog);
	mock.module("../a2ui/collect-run-elements", () => actual.collectRunElements);
	mock.module(
		"../a2ui/elements-request-handler",
		() => actual.elementsRequestHandler,
	);
	mock.module("../a2ui/widget-query-handler", () => actual.widgetQueryHandler);
	mock.module("../scoped-custom-css", () => actual.scopedCustomCss);
	mock.module(
		"./native-widget-page-capture",
		() => actual.nativeWidgetPageCapture,
	);
	mock.module("./page-loading-skeleton", () => actual.pageLoadingSkeleton);
});

function createPage(overrides: Partial<IPage> = {}): IPage {
	return {
		id: "load-page",
		name: "Load run",
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
		...overrides,
	};
}

async function flushDisposal() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

async function mount(appId: string, page: IPage, { strict = false } = {}) {
	const window = new Window({ url: "https://example.test/use" });
	// Bun does not populate this Happy DOM realm constructor used by selector parsing.
	Object.assign(window, { SyntaxError });
	const globals = {
		window,
		document: window.document,
		HTMLElement: window.HTMLElement,
		Node: window.Node,
		navigator: window.navigator,
		IS_REACT_ACT_ENVIRONMENT: true,
		setInterval: (callback: () => void) => {
			intervals.add(callback);
			return callback;
		},
		clearInterval: (callback: () => void) => intervals.delete(callback),
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
	const event = { id: "page-event", default_page_id: page.id } as IEvent;
	const rerender = async (nextPage: IPage) => {
		const element = (
			<PageInterface
				appId={appId}
				event={event}
				page={nextPage}
				pageExecutionRevision="execution-v1"
				route="/load"
				queryParams={{}}
			/>
		);
		await act(async () => {
			root?.render(strict ? <StrictMode>{element}</StrictMode> : element);
		});
	};
	await rerender(page);
	return { rerender };
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
const pageLoadingFlag = () =>
	host
		?.querySelector("[data-flowpilot-page-loading]")
		?.getAttribute("data-flowpilot-page-loading") ?? null;
const loadReports = () =>
	(
		(globalThis as { __flowLikeRunTimings?: IRunTimingReport[] })
			.__flowLikeRunTimings ?? []
	).filter((report) => report.label === "page onLoad");
const markNames = (report: IRunTimingReport | undefined) =>
	report?.marks.map((mark) => mark.name) ?? [];

const a2ui = (payload: Record<string, unknown>): RunEvents => [
	{ event_type: "a2ui", payload },
];
const freshMessage = {
	type: "upsertElement",
	element_id: "load-page/fresh",
	value: {
		type: "createComponent",
		component: {
			id: "fresh",
			type: "text",
			content: { literalString: "Fresh" },
		},
	},
};
const freshElement = a2ui(freshMessage);

const cachedSurface = (): Surface => {
	const [root, headline] = createPage().components;
	return {
		id: "load-page",
		rootComponentId: "root",
		components: {
			root,
			headline,
			cached: {
				id: "cached",
				component: {
					id: "cached",
					type: "text",
					content: { literalString: "Last visit" },
				},
			},
		},
	} as Surface;
};

function holdCacheRead() {
	let release: (surface: Surface | null) => void = () => {};
	readCachedSurface.mockImplementation(
		() =>
			new Promise<Surface | null>((resolve) => {
				release = resolve;
			}),
	);
	return (surface: Surface | null) => release(surface);
}

async function deliver(run: Run, events: RunEvents) {
	await act(async () => run.onEvents(events));
}

async function finishRun(run: Run) {
	await act(async () => run.resolve());
}

describe("page onLoad renders the static layout first", () => {
	test("static components render while onLoad runs, behind an indicator that leaves on the first renderable message", async () => {
		await mount("render-first", createPage());
		expect(runs).toHaveLength(1);
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
		expect(loadIndicator()).not.toBeNull();
		expect(busyRegion()).not.toBeNull();
		expect(pageLoadingFlag()).toBe("true");

		await act(async () => runs[0].onStarted?.("run-1"));
		expect(loadIndicator()).not.toBeNull();

		await deliver(runs[0], freshElement);
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).toBeNull();
		expect(busyRegion()).toBeNull();
		expect(pageLoadingFlag()).toBe("true");
		expect(captureReadiness.at(-1)).toBe(false);

		await finishRun(runs[0]);
		expect(pageLoadingFlag()).toBe("false");
		expect(captureReadiness.at(-1)).toBe(true);
	});

	test("one status region outside the busy page announces the load and clears when output arrives", async () => {
		await mount("render-first-status", createPage());
		const status = loadStatus();
		expect(status?.textContent).toBe("Loading page data…");
		expect(status?.closest("[aria-busy]")).toBeNull();

		await deliver(runs[0], freshElement);
		expect(loadStatus()).toBe(status);
		expect(status?.textContent).toBe("");
	});

	test("an empty page keeps its status region from the empty state into the load", async () => {
		await mount("render-first-empty-status", createPage({ components: [] }));
		const status = loadStatus();
		expect(status?.textContent).toBe("Loading page data…");
		expect(status?.closest("[aria-busy]")).toBeNull();
		await finishRun(runs[0]);
		expect(loadStatus()).toBe(status);
		expect(status?.textContent).toBe("");
	});

	test("output from an action or interval run does not stand in for the load run's", async () => {
		await mount(
			"render-first-other-runs",
			createPage({ onIntervalEventId: "interval-node", onIntervalSeconds: 5 }),
		);
		await act(async () => runs[0].onStarted?.("run-load"));
		await act(async () => renderedOnA2UIMessage?.(freshMessage));
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).not.toBeNull();
		expect(busyRegion()).not.toBeNull();

		await act(async () => {
			for (const tick of intervals) tick();
		});
		await deliver(runs[1], a2ui({ type: "showScreen" }));
		await deliver(runs[1], freshElement);
		expect(loadIndicator()).not.toBeNull();
		expect(loadReports()).toHaveLength(0);

		await deliver(runs[0], a2ui({ type: "showScreen" }));
		expect(loadIndicator()).toBeNull();
		expect(markNames(loadReports()[0])).toEqual([
			"run_initiated",
			"interval_dispatch",
			"revealed",
		]);
	});

	test("the indicator leaves when the load run ends without output", async () => {
		await mount("render-first-silent", createPage());
		expect(loadIndicator()).not.toBeNull();
		await finishRun(runs[0]);
		expect(loadIndicator()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
	});

	test("an empty page shows the indicator while onLoad runs and its empty state after", async () => {
		await mount("render-first-empty", createPage({ components: [] }));
		expect(loadIndicator()).not.toBeNull();
		expect(busyRegion()).not.toBeNull();
		expect(host?.textContent).not.toContain("No content to display");
		await finishRun(runs[0]);
		expect(loadIndicator()).toBeNull();
		expect(host?.textContent).toContain("No content to display");
	});
});

describe("page surface cache", () => {
	test("a default page renders its static layout without waiting for the cache read, then shows the cached surface behind the indicator while the load run rebuilds from the static layout", async () => {
		const releaseCache = holdCacheRead();
		await mount("cache-replay", createPage());
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
		expect(readCachedSurface).toHaveBeenCalledTimes(1);

		await act(async () => releaseCache(cachedSurface()));
		expect(renderedComponents()).toBe("cached headline root");
		expect(loadIndicator()).not.toBeNull();

		await deliver(runs[0], freshElement);
		expect(renderedComponents()).toBe("cached headline root");
		expect(loadIndicator()).not.toBeNull();
		expect(busyRegion()).not.toBeNull();

		await finishRun(runs[0]);
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).toBeNull();
	});

	test("the cached surface yields once the load run's output settles", async () => {
		const settleTimers: (() => void)[] = [];
		const realSetTimeout = globalThis.setTimeout;
		const timers = spyOn(globalThis, "setTimeout").mockImplementation(((
			callback: () => void,
			delay?: number,
		) => {
			if (delay !== STALE_SURFACE_SETTLE_MS)
				return realSetTimeout(callback, delay);
			settleTimers.push(callback);
			return 0;
		}) as unknown as typeof setTimeout);
		try {
			const releaseCache = holdCacheRead();
			await mount("cache-settle", createPage());
			await act(async () => releaseCache(cachedSurface()));
			await deliver(runs[0], freshElement);
			expect(renderedComponents()).toBe("cached headline root");

			await act(async () => settleTimers.at(-1)?.());
			expect(renderedComponents()).toBe("fresh headline root");
			expect(loadIndicator()).toBeNull();
			expect(pageLoadingFlag()).toBe("true");
		} finally {
			timers.mockRestore();
		}
	});

	test("showScreen hands the page to the load run's output at once", async () => {
		const releaseCache = holdCacheRead();
		await mount("cache-show-screen", createPage());
		await act(async () => releaseCache(cachedSurface()));
		await deliver(runs[0], freshElement);
		expect(renderedComponents()).toBe("cached headline root");

		await deliver(runs[0], a2ui({ type: "showScreen" }));
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).toBeNull();
	});

	test("output from an action on the cached surface shows at once and survives the switch", async () => {
		const releaseCache = holdCacheRead();
		await mount("cache-action", createPage());
		await act(async () => releaseCache(cachedSurface()));

		await act(async () => renderedOnA2UIMessage?.(freshMessage));
		expect(renderedComponents()).toBe("cached fresh headline root");

		await finishRun(runs[0]);
		expect(renderedComponents()).toBe("fresh headline root");
	});

	test("a cached surface that lands after the load run's output does not replace it", async () => {
		const releaseCache = holdCacheRead();
		await mount("cache-late", createPage());
		await deliver(runs[0], freshElement);
		await act(async () => releaseCache(cachedSurface()));
		expect(renderedComponents()).toBe("fresh headline root");
	});

	test("a default page stores its surface once the load run succeeded", async () => {
		await mount("cache-write", createPage());
		await deliver(runs[0], freshElement);
		expect(writeCachedSurface).not.toHaveBeenCalled();

		await finishRun(runs[0]);
		expect(writeCachedSurface).toHaveBeenCalledTimes(1);
		const [identity, surface] = writeCachedSurface.mock.calls[0];
		expect(identity?.pageId).toBe("load-page");
		expect(Object.keys(surface.components).sort()).toEqual([
			"fresh",
			"headline",
			"root",
		]);
	});

	test("a failed load run stores nothing", async () => {
		const silenceError = spyOn(console, "error").mockImplementation(() => {});
		try {
			await mount("cache-failed", createPage());
			await deliver(runs[0], freshElement);
			await act(async () => runs[0].reject(new Error("stream dropped")));
			expect(loadIndicator()).toBeNull();
			expect(writeCachedSurface).not.toHaveBeenCalled();
		} finally {
			silenceError.mockRestore();
		}
	});

	test("a failed load run still replaces the cached surface", async () => {
		const silenceError = spyOn(console, "error").mockImplementation(() => {});
		try {
			const releaseCache = holdCacheRead();
			await mount("cache-failed-release", createPage());
			await act(async () => releaseCache(cachedSurface()));
			await act(async () => runs[0].reject(new Error("stream dropped")));
			expect(renderedComponents()).toBe("headline root");
			expect(loadIndicator()).toBeNull();
		} finally {
			silenceError.mockRestore();
		}
	});

	test("only a successful action or interval run stores the page again", async () => {
		await mount(
			"cache-later-runs",
			createPage({ onIntervalEventId: "interval-node", onIntervalSeconds: 5 }),
		);
		await finishRun(runs[0]);
		expect(writeCachedSurface).toHaveBeenCalledTimes(1);

		await act(async () => renderedOnA2UIMessage?.(freshMessage));
		await act(async () =>
			notifyLivePageRun("load-page", { status: "failed", endedAtMs: 1 }),
		);
		expect(writeCachedSurface).toHaveBeenCalledTimes(1);

		await act(async () =>
			notifyLivePageRun("load-page", { status: "ok", endedAtMs: 2 }),
		);
		expect(writeCachedSurface).toHaveBeenCalledTimes(2);
		expect(
			Object.keys(writeCachedSurface.mock.calls[1][1].components).sort(),
		).toEqual(["fresh", "headline", "root"]);

		await act(async () => {
			for (const tick of intervals) tick();
		});
		await finishRun(runs[1]);
		expect(writeCachedSurface).toHaveBeenCalledTimes(3);
	});
});

describe("noCache pages", () => {
	test("show the loading screen instead of the static layout until the load run renders", async () => {
		await mount("no-cache-reveal", createPage({ noCache: true }));
		expect(skeleton()?.textContent).toBe("Running workflow");
		expect(renderedComponents()).toBeNull();

		await deliver(
			runs[0],
			a2ui({ type: "setGlobalState", key: "ready", value: false }),
		);
		expect(skeleton()).not.toBeNull();

		await deliver(runs[0], freshElement);
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("fresh headline root");
		expect(loadIndicator()).toBeNull();
	});

	test("leave the loading screen when the load run ends without output", async () => {
		await mount("no-cache-silent", createPage({ noCache: true }));
		expect(skeleton()).not.toBeNull();
		await finishRun(runs[0]);
		expect(skeleton()).toBeNull();
		expect(renderedComponents()).toBe("headline root");
	});

	test("never read or write the surface cache", async () => {
		await mount("no-cache-store", createPage({ noCache: true }));
		await deliver(runs[0], freshElement);
		await finishRun(runs[0]);
		expect(readCachedSurface).not.toHaveBeenCalled();
		expect(writeCachedSurface).not.toHaveBeenCalled();
	});
});

describe("page onUnload", () => {
	const unloadRuns = () =>
		runs.filter((run) => run.payload.id === "page_unload");
	const unloadingPage = (overrides: Partial<IPage> = {}) =>
		createPage({ onUnloadEventId: "unload-node", ...overrides });

	test("fires once on a real unmount and not when a re-render renews its dispatcher", async () => {
		const { rerender } = await mount("unload-once", unloadingPage());
		await rerender(unloadingPage({ name: "Renamed" }));
		await rerender(unloadingPage({ name: "Renamed again" }));
		await flushDisposal();
		expect(unloadRuns()).toHaveLength(0);

		await act(() => root?.unmount());
		root = undefined;
		await flushDisposal();
		expect(unloadRuns()).toHaveLength(1);
		expect(unloadRuns()[0].payload.payload).toMatchObject({
			_page_id: "load-page",
			_event_type: "onUnload",
		});
	});

	test("a state-preserving effect replay does not fire it", async () => {
		await mount("unload-strict", unloadingPage(), { strict: true });
		await flushDisposal();
		expect(unloadRuns()).toHaveLength(0);
	});

	test("switching to another page in place fires the leaving page's onUnload", async () => {
		const { rerender } = await mount(
			"unload-switch",
			unloadingPage({ id: "old-page" }),
		);
		await rerender(createPage({ id: "new-page" }));
		await flushDisposal();
		expect(unloadRuns().map((run) => run.payload.payload._page_id)).toEqual([
			"old-page",
		]);
	});
});

describe("page onLoad run ownership", () => {
	test("unmounting mid-run cancels the run and drops its later messages", async () => {
		const appId = "orphan-unmount";
		await mount(
			appId,
			createPage({ onIntervalEventId: "interval-node", onIntervalSeconds: 5 }),
		);
		await act(async () => runs[0].onStarted?.("run-load"));
		await act(async () => {
			for (const tick of intervals) tick();
		});
		expect(runs).toHaveLength(2);
		expect(runs[1].payload.payload._event_type).toBe("onInterval");

		await act(() => root?.unmount());
		root = undefined;
		expect(cancelExecution).not.toHaveBeenCalled();
		await flushDisposal();
		expect(cancelExecution.mock.calls).toEqual([["run-load"]]);
		expect(markNames(loadReports().at(-1))).toContain("unmounted");

		const store = getFrontendStateStore(appId);
		const before = store.getSnapshot();
		for (const run of runs) {
			run.onEvents(a2ui({ type: "setGlobalState", key: "late", value: 1 }));
			run.onEvents(
				a2ui({
					type: "requestElements",
					request_id: "late-request",
					selectors: ["headline"],
				}),
			);
		}
		expect(store.getSnapshot()).toBe(before);
		expect(handleElementsRequest).not.toHaveBeenCalled();

		await finishRun(runs[0]);
		expect(cancelExecution).toHaveBeenCalledTimes(1);
		expect(runs).toHaveLength(2);
	});

	test("a run id that arrives after the page is gone is cancelled on arrival", async () => {
		await mount("orphan-late-id", createPage());
		await act(() => root?.unmount());
		root = undefined;
		await flushDisposal();
		expect(cancelExecution).not.toHaveBeenCalled();
		runs[0].onStarted?.("run-late");
		expect(cancelExecution.mock.calls).toEqual([["run-late"]]);
		runs[0].onStarted?.("run-late");
		expect(cancelExecution).toHaveBeenCalledTimes(1);
	});

	test("a load run superseded in place is cancelled and its trace closed at once", async () => {
		const appId = "orphan-supersede";
		const { rerender } = await mount(appId, createPage({ id: "old-page" }));
		await act(async () => runs[0].onStarted?.("run-old"));
		expect(loadReports()).toHaveLength(0);

		await rerender(createPage({ id: "new-page" }));
		expect(runs).toHaveLength(2);
		expect(runs[1].payload.payload._page_id).toBe("new-page");
		expect(cancelExecution.mock.calls).toEqual([["run-old"]]);
		expect(markNames(loadReports().at(-1))).toEqual([
			"run_initiated",
			"superseded",
		]);

		const before = getFrontendStateStore(appId).getSnapshot();
		await deliver(
			runs[0],
			a2ui({ type: "setGlobalState", key: "x", value: 1 }),
		);
		expect(getFrontendStateStore(appId).getSnapshot()).toBe(before);

		await act(async () => runs[1].onStarted?.("run-new"));
		await finishRun(runs[0]);
		expect(loadIndicator()).not.toBeNull();
		await finishRun(runs[1]);
		expect(loadIndicator()).toBeNull();
		expect(cancelExecution).toHaveBeenCalledTimes(1);
	});

	test("a host whose cancel throws synchronously still hands the page to the next run", async () => {
		cancelExecution.mockImplementation(() => {
			throw new Error("Method not implemented.");
		});
		const { rerender } = await mount(
			"orphan-sync-throw",
			createPage({ id: "old-page" }),
		);
		await act(async () => runs[0].onStarted?.("run-old"));

		await rerender(createPage({ id: "new-page" }));
		expect(cancelExecution.mock.calls).toEqual([["run-old"]]);
		expect(runs).toHaveLength(2);
		expect(runs[1].payload.payload._page_id).toBe("new-page");
		expect(pageLoadingFlag()).toBe("true");

		await act(async () => runs[1].onStarted?.("run-new"));
		await rerender(createPage({ id: "new-page", onLoadEventId: undefined }));
		expect(cancelExecution.mock.calls).toEqual([["run-old"], ["run-new"]]);
		expect(loadIndicator()).toBeNull();
		expect(pageLoadingFlag()).toBe("false");
	});

	test("a host whose cancel throws synchronously does not break the deferred dispose", async () => {
		cancelExecution.mockImplementation(() => {
			throw new Error("Method not implemented.");
		});
		await mount("orphan-sync-throw-unmount", createPage());
		await act(async () => runs[0].onStarted?.("run-gone"));
		await act(() => root?.unmount());
		root = undefined;
		await flushDisposal();
		expect(cancelExecution.mock.calls).toEqual([["run-gone"]]);
		expect(markNames(loadReports().at(-1))).toEqual([
			"run_initiated",
			"unmounted",
		]);
	});

	test("a state-preserving effect replay does not cancel the load run", async () => {
		const appId = "orphan-strict-replay";
		await mount(appId, createPage(), { strict: true });
		await flushDisposal();
		expect(runs).toHaveLength(1);
		await act(async () => runs[0].onStarted?.("run-strict"));
		await flushDisposal();
		expect(cancelExecution).not.toHaveBeenCalled();

		await deliver(
			runs[0],
			a2ui({ type: "setGlobalState", key: "kept", value: true }),
		);
		expect(getFrontendStateStore(appId).getSnapshot().globalState).toEqual({
			kept: true,
		});
		expect(loadReports()).toHaveLength(0);
	});
});

describe("page onLoad trace", () => {
	test("finishes at reveal and marks an interval dispatched during the load", async () => {
		await mount(
			"trace-reveal",
			createPage({ onIntervalEventId: "interval-node", onIntervalSeconds: 5 }),
		);
		await act(async () => runs[0].onStarted?.("run-trace"));
		await act(async () => {
			for (const tick of intervals) tick();
		});
		expect(loadReports()).toHaveLength(0);

		await deliver(runs[0], freshElement);
		expect(loadReports()).toHaveLength(1);
		expect(markNames(loadReports()[0])).toEqual([
			"run_initiated",
			"interval_dispatch",
			"revealed",
		]);

		await finishRun(runs[0]);
		expect(loadReports()).toHaveLength(1);
	});

	test("finishes at run end when the load never reveals", async () => {
		await mount("trace-run-end", createPage());
		await act(async () => runs[0].onStarted?.("run-quiet"));
		expect(loadReports()).toHaveLength(0);
		await finishRun(runs[0]);
		expect(markNames(loadReports()[0])).toEqual(["run_initiated"]);
	});
});
