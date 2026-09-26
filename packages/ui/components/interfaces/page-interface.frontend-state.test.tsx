import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	appRouteUrl,
	parseAppRouteTarget,
	readAppQuery,
} from "../../lib/app-route-url";
import type { IEvent } from "../../lib/schema/flow/event";
import type { IPage } from "../../state/backend-state/page-state";
import type {
	FrontendStatePersistence,
	FrontendStateStore,
} from "../a2ui/frontend-state";

type Run = {
	payload: { id: string; payload: Record<string, unknown> };
	onEvents: (events: { event_type: string; payload: unknown }[]) => void;
};

const runs: Run[] = [];
const intervals = new Set<() => void>();
let hydration: Promise<void> = Promise.resolve();
let failNextRun = false;
const captureReadiness: boolean[] = [];
const globalGetAll = mock(async () => {
	await hydration;
	return { theme: "saved" };
});
const pageGetAll = mock(async (_appId: string, pageId: string) => {
	await hydration;
	return { count: 2, savedPage: pageId };
});
const globalSet = mock(async () => {});
const pageSet = mock(async () => {});

// bun keeps a module mock for every later file in the process, so the modules that later files
// use for real are captured here and restored in afterAll. frontend-state binds its default
// persistence when it is first evaluated, often by an earlier file, so the stores get the test
// persistence injected instead of through a storage module mock.
const actualFrontendState = { ...(await import("../a2ui/frontend-state")) };
const testPersistence: FrontendStatePersistence = {
	global: { getAll: globalGetAll, set: globalSet },
	page: { getAll: pageGetAll, set: pageSet, clearPage: mock(async () => {}) },
};
const testStores = new Map<string | undefined, FrontendStateStore>();
mock.module("../a2ui/frontend-state", () => ({
	...actualFrontendState,
	getFrontendStateStore: (appId: string | undefined) => {
		let store = testStores.get(appId);
		if (!store) {
			store = actualFrontendState.createFrontendStateStore(
				appId,
				testPersistence,
			);
			testStores.set(appId, store);
		}
		return store;
	},
}));
const actual = {
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
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
const router = {
	push: mock((_href: string) => {}),
	replace: mock((_href: string) => {}),
};
let hostSearch = "host=ignored";
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	useRouter: () => router,
	useSearchParams: () => new URLSearchParams(hostSearch),
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
		executeEvent: async (
			_appId: string,
			_eventId: string,
			payload: Run["payload"],
			_stream: boolean,
			_onStarted: unknown,
			onEvents: Run["onEvents"],
		) => {
			if (failNextRun) {
				failNextRun = false;
				throw new Error("Page load failed");
			}
			runs.push({ payload, onEvents });
		},
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
	A2UIRenderer: ({ agentBridge }: { agentBridge: ReactNode }) => agentBridge,
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
	handleElementsRequestMessage: () => false,
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
	PageLoadingSkeleton: () => null,
}));

const { PageInterface } = await import("./page-interface");
const { getFrontendStateStore } = await import("../a2ui/frontend-state");

let root: Root | undefined;
let restoreGlobals: (() => void) | undefined;

afterEach(async () => {
	await act(() => root?.unmount());
	root = undefined;
	restoreGlobals?.();
	restoreGlobals = undefined;
	intervals.clear();
	runs.length = 0;
	hydration = Promise.resolve();
	failNextRun = false;
	captureReadiness.length = 0;
	hostSearch = "host=ignored";
	router.push.mockClear();
	router.replace.mockClear();
});
afterAll(() => {
	mock.restore();
	mock.module("../a2ui/frontend-state", () => actualFrontendState);
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

function createPage(id: string): IPage {
	return {
		id,
		name: "State test",
		components: [
			{
				id: "root",
				component: {
					id: "root",
					type: "column",
					children: { explicitList: [] },
				},
			},
		],
		content: [],
		layoutType: "stack",
		createdAt: "2026-09-09",
		updatedAt: "2026-09-09",
		onLoadEventId: "load-node",
	};
}

async function mount(appId: string, page: IPage, embedded = true) {
	const window = new Window({
		url: `https://example.test/use?${hostSearch.replace(/^\?/, "")}`,
	});
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
	const host = window.document.createElement("div");
	window.document.body.append(host);
	root = createRoot(host as unknown as HTMLElement);
	const event = { id: "page-event", default_page_id: page.id } as IEvent;
	const rerender = async (nextPage: IPage) => {
		await act(() =>
			root?.render(
				<PageInterface
					appId={appId}
					event={event}
					page={nextPage}
					pageExecutionRevision="execution-v1"
					route="/settings"
					queryParams={embedded ? { source: "embedded" } : undefined}
				/>,
			),
		);
	};
	await rerender(page);
	return { rerender };
}

describe("page lifecycle frontend state", () => {
	test("does not mark failed page loads as fresh native widget content", async () => {
		failNextRun = true;
		const mounted = await mount("failed-native-load", createPage("first"));
		expect(captureReadiness.at(-1)).toBe(false);
		await mounted.rerender(createPage("next"));
		expect(captureReadiness.at(-1)).toBe(true);
	});
	test("native app query reaches page load and its updates cannot change the outer app", async () => {
		const url = new URL(
			appRouteUrl(
				"chosen",
				parseAppRouteTarget("/settings?id=order&tag=a&tag=b", [
					{ name: "raw", value: "A+B & 東京 50% #" },
				]),
			),
			"https://example.test",
		);
		hostSearch = url.search;
		await mount("native-query-page", createPage("query-page"), false);
		expect(runs[0].payload.payload).toMatchObject({
			_query_params: { id: "order", tag: "b", raw: "A+B & 東京 50% #" },
			_query_params_format: "app",
			_query_param_values: { tag: ["a", "b"] },
		});
		await act(() =>
			runs[0].onEvents([
				{
					event_type: "a2ui",
					payload: {
						type: "setQueryParam",
						key: "id",
						value: "updated order",
						replace: false,
					},
				},
			]),
		);
		const next = new URL(router.push.mock.calls[0][0], "https://example.test");
		expect(next.searchParams.get("id")).toBe("chosen");
		expect(readAppQuery(next.search).get("id")).toBe("updated order");
		expect(readAppQuery(next.search).getAll("tag")).toEqual(["a", "b"]);
	});

	test("hydrates load input and carries state responses into interval and unload runs", async () => {
		let releaseHydration = () => {};
		hydration = new Promise((resolve) => {
			releaseHydration = resolve;
		});
		const appId = "lifecycle-state-roundtrip";
		const page = {
			...createPage("settings-page"),
			onIntervalEventId: "interval-node",
			onIntervalSeconds: 30,
			onUnloadEventId: "unload-node",
		};
		await mount(appId, page);
		expect(runs).toHaveLength(0);
		await act(async () => releaseHydration());
		expect(runs).toHaveLength(1);
		expect(runs[0].payload.payload).toMatchObject({
			_page_id: "settings-page",
			_route: "/settings",
			_query_params: { source: "embedded" },
			_global_state: { theme: "saved" },
			_page_state: { count: 2, savedPage: "settings-page" },
		});

		await act(() =>
			runs[0].onEvents([
				{
					event_type: "a2ui",
					payload: { type: "setGlobalState", key: "theme", value: "dark" },
				},
				{
					event_type: "a2ui",
					payload: {
						type: "setPageState",
						page_id: "settings-page",
						key: "count",
						value: 3,
					},
				},
			]),
		);
		expect(getFrontendStateStore(appId).getSnapshot()).toMatchObject({
			globalState: { theme: "dark" },
			pageStates: { "settings-page": { count: 3 } },
		});
		await act(async () => {
			for (const tick of intervals) tick();
		});
		expect(runs[1].payload.payload).toMatchObject({
			_event_type: "onInterval",
			_global_state: { theme: "dark" },
			_page_state: { count: 3, savedPage: "settings-page" },
		});

		await act(() =>
			runs[1].onEvents([
				{
					event_type: "a2ui",
					payload: { type: "clearPageState", page_id: "settings-page" },
				},
			]),
		);
		await act(() => root?.unmount());
		root = undefined;
		// onUnload waits one task to confirm the unmount is not a StrictMode replay.
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
		expect(runs[2].payload.payload).toMatchObject({
			_event_type: "onUnload",
			_global_state: { theme: "dark" },
			_page_state: {},
		});
		expect(pageGetAll).toHaveBeenCalledWith(appId, "settings-page");
	});

	test("does not start a superseded page while state hydration is pending", async () => {
		let releaseHydration = () => {};
		hydration = new Promise((resolve) => {
			releaseHydration = resolve;
		});
		const { rerender } = await mount(
			"lifecycle-state-superseded",
			createPage("old-page"),
		);
		await rerender(createPage("new-page"));
		await act(async () => releaseHydration());
		expect(runs).toHaveLength(1);
		expect(runs[0].payload.payload).toMatchObject({
			_page_id: "new-page",
			_page_state: { savedPage: "new-page" },
		});
	});
});
