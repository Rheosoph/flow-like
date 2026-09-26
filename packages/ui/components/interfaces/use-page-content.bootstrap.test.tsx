import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { IEvent } from "../../lib/schema/flow/event";
import type { IPageBootstrap } from "../../state/backend-state/page-state";
import type { IUseEventMapping, IUseInterfaceProps } from "./interfaces";
import type { UsePageContentProps } from "./use-page-content";

const COLD_OPEN_URL = "/use?id=app-1";
const window = new Window({ url: `https://example.test${COLD_OPEN_URL}` });
Object.assign(window, { SyntaxError, TypeError, Error });
let online = true;
Object.defineProperty(window.navigator, "onLine", {
	configurable: true,
	get: () => online,
});
const domGlobals = {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	Event: window.Event,
	CustomEvent: window.CustomEvent,
	MutationObserver: window.MutationObserver,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: clearTimeout,
	IS_REACT_ACT_ENVIRONMENT: true,
};
const previousGlobals = Object.fromEntries(
	Object.keys(domGlobals).map((key) => [
		key,
		Object.getOwnPropertyDescriptor(globalThis, key),
	]),
);
Object.assign(globalThis, domGlobals);

const { act, useEffect, useMemo, useSyncExternalStore } = await import("react");
const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { ApiResponseError } = await import("../../lib/api-error");
const { createSmartQueryPersister } = await import("../../lib/query-persister");
const { MobileHeaderProvider } = await import("../ui/mobile-header");

interface Mount {
	readonly revision?: string;
	readonly executionRevision?: string;
}

const mounts: Mount[] = [];
function CountingPageInterface({
	page,
	pageRevision,
	pageExecutionRevision,
}: Readonly<{
	page: { id: string };
	pageRevision?: string;
	pageExecutionRevision?: string;
}>) {
	useEffect(() => {
		mounts.push({
			revision: pageRevision,
			executionRevision: pageExecutionRevision,
		});
	}, []);
	return <div data-page-revision={pageRevision} data-page-id={page.id} />;
}

let headerSwitchEvent: ((eventId: string) => void) | undefined;
function CapturingHeader({
	switchEvent,
}: Readonly<{ switchEvent: (eventId: string) => void }>) {
	useEffect(() => {
		headerSwitchEvent = switchEvent;
	}, [switchEvent]);
	return null;
}

function ChatInterface({ event }: IUseInterfaceProps) {
	return <div data-event-interface={event.id} />;
}
const CHAT_EVENT_CONFIG: IUseEventMapping = {
	chat: {
		eventTypes: ["simple_chat"],
		useInterfaces: { simple_chat: ChatInterface },
	},
};

const locationListeners = new Set<() => void>();
function setLocation(href: string) {
	window.history.replaceState(null, "", href);
	for (const listener of locationListeners) listener();
}
function subscribeToLocation(listener: () => void) {
	locationListeners.add(listener);
	return () => {
		locationListeners.delete(listener);
	};
}
function useLocationSearchParams() {
	const search = useSyncExternalStore(
		subscribeToLocation,
		() => window.location.search,
	);
	return useMemo(() => new URLSearchParams(search), [search]);
}
/** Next commits a search-param navigation after the render that requested it. */
function followQueryNavigation(href: string) {
	if (href.startsWith("?")) realSetTimeout(() => setLocation(href), 0);
}

const router = {
	push: mock(followQueryNavigation),
	replace: mock(followQueryNavigation),
	prefetch: () => {},
	back: () => {},
	forward: () => {},
	refresh: () => {},
};
const auth = {
	isLoading: false,
	isAuthenticated: true,
	user: { access_token: "token", profile: { sub: "user-1" } },
};

const EVENT = {
	id: "event-1",
	name: "Home",
	description: "",
	active: true,
	priority: 0,
	route: "/",
	is_default: true,
	default_page_id: "page-1",
	event_type: "page",
	config: [],
} as unknown as IEvent;
const REPORTS_EVENT = {
	...EVENT,
	id: "event-reports",
	name: "Reports",
	priority: 1,
	route: "/reports",
	is_default: false,
	default_page_id: "page-reports",
} as IEvent;
const DETAIL_EVENT = {
	...EVENT,
	id: "event-detail",
	name: "Detail",
	priority: 2,
	route: null,
	is_default: false,
	default_page_id: "page-detail",
} as IEvent;
function chatEvent(id: string, priority: number): IEvent {
	return {
		...EVENT,
		id,
		name: id,
		priority,
		route: null,
		is_default: false,
		default_page_id: null,
		event_type: "simple_chat",
	};
}
const CHAT_A = chatEvent("chat-a", 0);
const CHAT_B = chatEvent("chat-b", 1);

function bootstrapAt(revision: string, event = EVENT): IPageBootstrap {
	return {
		event,
		page: {
			id: event.default_page_id ?? "page-1",
			name: event.name,
			content: [],
			layoutType: "freeform",
			components: [],
			createdAt: "2026-09-01T00:00:00.000Z",
			updatedAt: "2026-09-01T00:00:00.000Z",
		},
		revision,
		executionRevision: `execution-${revision}`,
	};
}

const transportFailure = () => new TypeError("Failed to fetch");
const denied = () =>
	new ApiResponseError({ status: 403, message: "No access to this app" });
const lapsed = () =>
	new ApiResponseError({ status: 401, message: "Unauthorized" });
const notFound = () =>
	new ApiResponseError({
		status: 404,
		message: "No active route or Event was found",
	});

let network: (
	appId: string,
	route?: string,
	eventId?: string,
) => Promise<IPageBootstrap> = async () => bootstrapAt("B");
let accessChecks: () => Promise<never> = async () => {
	throw transportFailure();
};
let catalog: () => Promise<IEvent[]> = () => accessChecks();
const bootstrapCalls: unknown[][] = [];
let metadataReads = 0;
const backend = {
	capabilities: () => ({
		needsSignIn: true,
		canHostLlamaCPP: false,
		canHostMLX: false,
		canHostEmbeddings: false,
		canExecuteLocally: false,
	}),
	pageState: {
		async getPageBootstrap(appId: string, route?: string, eventId?: string) {
			bootstrapCalls.push([appId, route, eventId]);
			return network(appId, route, eventId);
		},
	},
	eventState: {
		getEvents: () => catalog(),
		getEvent: () => accessChecks(),
	},
	appState: {
		getApp: () => accessChecks(),
		getAppMeta: async () => {
			metadataReads++;
			return {};
		},
	},
	userState: { getAllSettingsProfiles: async () => [] },
	boardState: { getBoard: async () => ({}) },
};

// bun keeps a module mock for every later file in the process, so each module replaced here
// is captured first and put back in afterAll.
const [
	actualNavigation,
	actualOidc,
	actualBackendState,
	actualPageInterface,
	actualHeader,
] = await Promise.all([
	import("next/navigation").then((module) => ({ ...module })),
	import("react-oidc-context").then((module) => ({ ...module })),
	import("../../state/backend-state").then((module) => ({ ...module })),
	import("./page-interface").then((module) => ({ ...module })),
	import("./header").then((module) => ({ ...module })),
]);
mock.module("next/navigation", () => ({
	...actualNavigation,
	useRouter: () => router,
	useSearchParams: useLocationSearchParams,
}));
mock.module("react-oidc-context", () => ({
	...actualOidc,
	useAuth: () => auth,
}));
mock.module("../../state/backend-state", () => ({
	...actualBackendState,
	useBackend: () => backend,
}));
mock.module("./page-interface", () => ({
	...actualPageInterface,
	PageInterface: CountingPageInterface,
}));
mock.module("./header", () => ({
	...actualHeader,
	Header: CapturingHeader,
}));

const { RESTORED_BOOTSTRAP_GRACE_MS, UsePageContent } = await import(
	"./use-page-content"
);
const { resetQueryParamRequests } = await import("../../lib/set-query-params");

const BOOTSTRAP_QUERY_KEY = ["getPageBootstrap", "app-1", "/", null, "user-1"];
const AUTHORIZATION_ERROR = "could not load its execution authorization";
const EVENTS_ERROR = "events could not be loaded";
const realSetTimeout = globalThis.setTimeout;

function memoryStorage() {
	const map = new Map<string, string>();
	return {
		get: async (key: string) => map.get(key),
		set: async (key: string, value: string) => {
			map.set(key, value);
		},
		del: async (key: string) => {
			map.delete(key);
		},
		entries: async () => [...map.entries()],
	};
}

/** A previous session's answer, persisted exactly as the host persister writes it. */
async function persistedSession(restored?: IPageBootstrap) {
	const persister = createSmartQueryPersister({ backend: memoryStorage() });
	if (restored) {
		const previous = new QueryClient();
		previous.setQueryData(BOOTSTRAP_QUERY_KEY, restored);
		await persister.persistQueryByKey(BOOTSTRAP_QUERY_KEY, previous);
		previous.clear();
	}
	return new QueryClient({
		defaultOptions: {
			queries: {
				networkMode: "always",
				staleTime: 30_000,
				refetchOnWindowFocus: false,
				refetchOnReconnect: false,
				refetchOnMount: true,
				retry: 1,
				retryDelay: 0,
				persister: persister.persisterFn,
			},
		},
	});
}

let root: ReturnType<typeof createRoot> | undefined;
let client: InstanceType<typeof QueryClient> | undefined;
let host: HTMLElement | undefined;

type ContentProps = Omit<UsePageContentProps, "eventConfig"> &
	Partial<Pick<UsePageContentProps, "eventConfig">>;
const ROOT_ROUTE: ContentProps = { appId: "app-1", routePath: "/" };
/** Plain `/use`: the app, route and Event all come from the address bar. */
const FROM_URL: ContentProps = {};
let contentProps = ROOT_ROUTE;

function tree(queryClient: InstanceType<typeof QueryClient>) {
	return (
		<QueryClientProvider client={queryClient}>
			<MobileHeaderProvider>
				<UsePageContent eventConfig={{}} {...contentProps} />
			</MobileHeaderProvider>
		</QueryClientProvider>
	);
}

async function render(
	queryClient: InstanceType<typeof QueryClient>,
	props = ROOT_ROUTE,
) {
	contentProps = props;
	client = queryClient;
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.append(host as never);
	root = createRoot(host);
	await act(async () => {
		root?.render(tree(queryClient));
	});
}

/** Leaves the page and opens it again over the same query cache, as navigating back does. */
async function reopen() {
	const queryClient = client;
	if (!queryClient) return;
	await act(async () => root?.unmount());
	host?.remove();
	mounts.length = 0;
	await render(queryClient, contentProps);
}

/** The auth context changed: a new render reads the mutated `auth` mock. */
async function rerender() {
	const queryClient = client;
	if (!queryClient) return;
	await act(async () => {
		root?.render(tree(queryClient));
	});
}

/** Holds every timer of `delay` ms for the test to fire by hand. */
function holdTimers(delay: number): (() => void)[] {
	const held: (() => void)[] = [];
	globalThis.setTimeout = ((
		callback: () => void,
		ms?: number,
		...args: unknown[]
	) => {
		if (ms !== delay) return realSetTimeout(callback, ms, ...args);
		held.push(callback);
		return 0;
	}) as typeof setTimeout;
	return held;
}

async function settle(rounds = 10) {
	for (let round = 0; round < rounds; round++) {
		await act(async () => {
			await new Promise((resolve) => realSetTimeout(resolve, 0));
		});
	}
}

async function waitFor(condition: () => boolean, label: string) {
	for (let round = 0; round < 100; round++) {
		if (condition()) return;
		await settle(1);
	}
	throw new Error(`Timed out waiting for ${label}`);
}

function deferred<T>() {
	let resolve: (value: T) => void = () => {};
	const promise = new Promise<T>((onResolve) => {
		resolve = onResolve;
	});
	return { promise, resolve };
}

const text = () => host?.textContent ?? "";
const onScreen = () =>
	host
		?.querySelector("[data-page-revision]")
		?.getAttribute("data-page-revision");
const storeRedirects = () =>
	router.replace.mock.calls.filter(([href]) => href.startsWith("/store"));
const pageOnScreen = () =>
	host?.querySelector("[data-page-id]")?.getAttribute("data-page-id");
const eventInterface = () =>
	host
		?.querySelector("[data-event-interface]")
		?.getAttribute("data-event-interface");
const navigations = () => [
	...router.push.mock.calls.map(([href]) => ["push", href]),
	...router.replace.mock.calls.map(([href]) => ["replace", href]),
];

afterEach(async () => {
	await act(async () => root?.unmount());
	// A navigation the test left in flight lands before the address is reset.
	await new Promise((resolve) => realSetTimeout(resolve, 0));
	setLocation(COLD_OPEN_URL);
	// A query-param write layers onto the previous one while that is still in flight.
	resetQueryParamRequests();
	window.localStorage.clear();
	headerSwitchEvent = undefined;
	root = undefined;
	client?.clear();
	client = undefined;
	host?.remove();
	host = undefined;
	mounts.length = 0;
	bootstrapCalls.length = 0;
	metadataReads = 0;
	online = true;
	network = async () => bootstrapAt("B");
	accessChecks = async () => {
		throw transportFailure();
	};
	catalog = () => accessChecks();
	auth.isLoading = false;
	auth.user.access_token = "token";
	router.push.mockClear();
	router.replace.mockClear();
	globalThis.setTimeout = realSetTimeout;
});

afterAll(() => {
	mock.module("next/navigation", () => actualNavigation);
	mock.module("react-oidc-context", () => actualOidc);
	mock.module("../../state/backend-state", () => actualBackendState);
	mock.module("./page-interface", () => actualPageInterface);
	mock.module("./header", () => actualHeader);
	for (const [key, descriptor] of Object.entries(previousGlobals)) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

describe("page bootstrap validation", () => {
	test("an online cold start mounts the server's bootstrap once, never the restored copy", async () => {
		const response = deferred<IPageBootstrap>();
		network = () => response.promise;
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => bootstrapCalls.length === 1, "the network request");
		await settle();
		expect(mounts).toEqual([]);

		response.resolve(bootstrapAt("B"));
		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();

		expect(mounts).toEqual([
			{ revision: "B", executionRevision: "execution-B" },
		]);
		expect(onScreen()).toBe("B");
	});

	test("a server confirming the restored copy mounts it once", async () => {
		network = async () => bootstrapAt("A");
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();

		expect(bootstrapCalls).toHaveLength(1);
		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
	});

	test("an offline cold start renders the restored copy and keeps it when the refetch fails", async () => {
		online = false;
		network = async () => {
			throw transportFailure();
		};
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => mounts.length > 0, "the page to mount");
		await waitFor(() => bootstrapCalls.length === 2, "the refetch to fail");
		await settle();

		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
		expect(onScreen()).toBe("A");
	});

	test("an unreachable server falls back to the restored copy instead of an error", async () => {
		network = async () => {
			throw transportFailure();
		};
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();

		expect(bootstrapCalls).toHaveLength(2);
		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
		expect(onScreen()).toBe("A");
		expect(text()).not.toContain(AUTHORIZATION_ERROR);
		expect(storeRedirects()).toEqual([]);
	});

	test("a request that never settles releases the restored copy after the grace period", async () => {
		const graceTimers: (() => void)[] = [];
		globalThis.setTimeout = ((
			callback: () => void,
			delay?: number,
			...args: unknown[]
		) => {
			if (delay !== RESTORED_BOOTSTRAP_GRACE_MS) {
				return realSetTimeout(callback, delay, ...args);
			}
			graceTimers.push(callback);
			return 0;
		}) as typeof setTimeout;
		network = () => new Promise<never>(() => {});
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => bootstrapCalls.length === 1, "the network request");
		await settle();
		expect(mounts).toEqual([]);
		expect(graceTimers.length).toBeGreaterThan(0);

		await act(async () => {
			for (const expire of graceTimers) expire();
		});
		await waitFor(() => mounts.length > 0, "the page to mount");

		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
		expect(onScreen()).toBe("A");
	});

	for (const [label, failure] of [
		["an unreachable server", transportFailure],
		[
			"a server error",
			() => new ApiResponseError({ status: 503, message: "Unavailable" }),
		],
		["a session that lapsed in the background", lapsed],
	] as const) {
		test(`a validated page survives a background refetch that meets ${label}`, async () => {
			await render(await persistedSession());
			await waitFor(() => mounts.length > 0, "the page to mount");

			network = async () => {
				throw failure();
			};
			await act(async () => {
				window.dispatchEvent(new window.Event("focus"));
			});
			await waitFor(
				() => Boolean(client?.getQueryState(BOOTSTRAP_QUERY_KEY)?.error),
				"the refetch to fail",
			);
			await settle();

			expect(mounts).toEqual([
				{ revision: "B", executionRevision: "execution-B" },
			]);
			expect(onScreen()).toBe("B");
			expect(text()).not.toContain(AUTHORIZATION_ERROR);
		});
	}

	/** A validated page whose focus revalidation could not reach the server. */
	async function pageOverFailedRefetch() {
		await render(await persistedSession());
		await waitFor(() => mounts.length > 0, "the page to mount");
		network = async () => {
			throw transportFailure();
		};
		await act(async () => {
			window.dispatchEvent(new window.Event("focus"));
		});
		await waitFor(
			() => Boolean(client?.getQueryState(BOOTSTRAP_QUERY_KEY)?.error),
			"the refetch to fail",
		);
		await settle();
	}

	test("a page reopened over a failed refetch waits for the server, not the kept copy", async () => {
		await pageOverFailedRefetch();
		const response = deferred<IPageBootstrap>();
		network = () => response.promise;
		const callsBeforeReopen = bootstrapCalls.length;

		await reopen();
		await waitFor(
			() => bootstrapCalls.length > callsBeforeReopen,
			"the network request",
		);
		await settle();
		expect(mounts).toEqual([]);

		response.resolve(bootstrapAt("C"));
		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();

		expect(mounts).toEqual([
			{ revision: "C", executionRevision: "execution-C" },
		]);
		expect(onScreen()).toBe("C");
	});

	test("a page reopened over a failed refetch keeps its copy while the server stays unreachable", async () => {
		await pageOverFailedRefetch();
		const callsBeforeReopen = bootstrapCalls.length;

		await reopen();
		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();

		expect(bootstrapCalls.length).toBeGreaterThan(callsBeforeReopen);
		expect(mounts).toEqual([
			{ revision: "B", executionRevision: "execution-B" },
		]);
		expect(text()).not.toContain(AUTHORIZATION_ERROR);
	});

	test("a validated page still reports a background refetch the server refused", async () => {
		await render(await persistedSession());
		await waitFor(() => mounts.length > 0, "the page to mount");

		network = async () => {
			throw denied();
		};
		await act(async () => {
			window.dispatchEvent(new window.Event("focus"));
		});
		await waitFor(
			() => text().includes(AUTHORIZATION_ERROR),
			"the authorization error",
		);

		expect(mounts).toHaveLength(1);
		expect(onScreen()).toBeUndefined();
		expect(storeRedirects()).toEqual([]);
	});

	test("a first load the server refuses still sends the user to the store", async () => {
		network = async () => {
			throw denied();
		};
		accessChecks = async () => {
			throw denied();
		};
		await render(await persistedSession());

		await waitFor(() => storeRedirects().length > 0, "the store redirect");

		expect(storeRedirects()).toEqual([["/store?id=app-1"]]);
		expect(mounts).toEqual([]);
	});

	test("a restored copy does not vouch for access the server now refuses", async () => {
		network = async () => {
			throw denied();
		};
		accessChecks = async () => {
			throw denied();
		};
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => storeRedirects().length > 0, "the store redirect");

		expect(storeRedirects()).toEqual([["/store?id=app-1"]]);
		expect(mounts).toEqual([]);
	});

	test("a first load that cannot reach the server retries on its own", async () => {
		const ladder = holdTimers(1_000);
		catalog = async () => [EVENT];
		network = async () => {
			throw transportFailure();
		};
		await render(await persistedSession());

		await waitFor(() => text().includes(AUTHORIZATION_ERROR), "the error card");
		expect(text()).toContain("Failed to fetch");
		await waitFor(() => ladder.length > 0, "the scheduled retry");

		network = async () => bootstrapAt("B");
		await act(async () => {
			for (const retry of ladder.splice(0)) retry();
		});
		await waitFor(() => mounts.length > 0, "the page to mount");

		expect(onScreen()).toBe("B");
		expect(text()).not.toContain(AUTHORIZATION_ERROR);
	});

	test("a first load refused for a lapsed session recovers once the session renews", async () => {
		const ladder = holdTimers(1_000);
		network = async () => {
			throw lapsed();
		};
		accessChecks = async () => {
			throw lapsed();
		};
		await render(await persistedSession());

		await waitFor(
			() => text().includes(EVENTS_ERROR) || storeRedirects().length > 0,
			"the access checks to settle",
		);
		expect(storeRedirects()).toEqual([]);
		const callsBeforeRenewal = bootstrapCalls.length;

		network = async () => bootstrapAt("B");
		auth.user.access_token = "renewed-token";
		await rerender();
		await waitFor(() => mounts.length > 0, "the page to mount");

		expect(bootstrapCalls).toHaveLength(callsBeforeRenewal + 1);
		expect(ladder.length).toBeGreaterThan(0);
		expect(onScreen()).toBe("B");
		expect(storeRedirects()).toEqual([]);
	});

	test("a restored copy stands in while a lapsed session renews", async () => {
		network = async () => {
			throw lapsed();
		};
		accessChecks = async () => {
			throw lapsed();
		};
		await render(await persistedSession(bootstrapAt("A")));

		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();
		expect(text()).not.toContain(AUTHORIZATION_ERROR);
		expect(storeRedirects()).toEqual([]);
		const callsBeforeRenewal = bootstrapCalls.length;

		network = async () => bootstrapAt("A");
		auth.user.access_token = "renewed-token";
		await rerender();
		await waitFor(
			() => bootstrapCalls.length > callsBeforeRenewal,
			"the renewed request",
		);
		await settle();

		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
		expect(onScreen()).toBe("A");
	});
});

describe("runtime URL targets", () => {
	async function openAt(href: string, props = FROM_URL) {
		setLocation(href);
		await render(await persistedSession(), props);
	}

	async function settledPage() {
		await waitFor(() => mounts.length > 0, "the page to mount");
		await settle();
	}

	const eventById = (events: readonly IEvent[], eventId?: string) =>
		events.find((event) => event.id === eventId) ?? events[0];

	/** The root page, mounted once, without ever retargeting the address to its Event. */
	async function expectRootPageOnce() {
		await settledPage();
		expect(
			bootstrapCalls.filter(([, , eventId]) => eventId !== undefined),
		).toEqual([]);
		expect(mounts).toHaveLength(1);
		expect(pageOnScreen()).toBe("page-1");
		expect(window.location.search).toBe("?id=app-1");
		expect(navigations()).toEqual([]);
	}

	test("a cold open of the app root fetches its bootstrap once and mounts the page once", async () => {
		await openAt(COLD_OPEN_URL);
		await settledPage();

		expect({ bootstrapCalls, mounts }).toEqual({
			bootstrapCalls: [["app-1", "/", undefined]],
			mounts: [{ revision: "B", executionRevision: "execution-B" }],
		});
		expect(window.location.search).toBe("?id=app-1");
		expect(navigations()).toEqual([]);
	});

	test("a page target never reads the app metadata only the header shows", async () => {
		await openAt(COLD_OPEN_URL);
		await settledPage();

		expect(metadataReads).toBe(0);
	});

	test("a root open recovering through the retry ladder keeps the root target", async () => {
		const ladder = holdTimers(1_000);
		network = async () => {
			throw transportFailure();
		};
		await openAt(COLD_OPEN_URL);
		await waitFor(() => ladder.length > 0, "the scheduled retry");
		await settle();

		network = async () => bootstrapAt("B");
		await act(async () => {
			for (const retry of ladder.splice(0)) retry();
		});
		await expectRootPageOnce();
		expect(metadataReads).toBe(0);
	});

	test("a root open recovering from a lapsed session keeps the root target", async () => {
		holdTimers(1_000);
		network = async () => {
			throw lapsed();
		};
		accessChecks = async () => {
			throw lapsed();
		};
		await openAt(COLD_OPEN_URL);
		await waitFor(() => text().includes(EVENTS_ERROR), "the events error");

		network = async () => bootstrapAt("B");
		auth.user.access_token = "renewed-token";
		await rerender();
		await expectRootPageOnce();
	});

	test("an offline root open without a restored copy keeps the root target on reconnect", async () => {
		online = false;
		network = async () => {
			throw transportFailure();
		};
		await openAt(COLD_OPEN_URL);
		await waitFor(() => bootstrapCalls.length === 2, "the request to fail");
		await settle();

		network = async () => bootstrapAt("B");
		online = true;
		await act(async () => {
			window.dispatchEvent(new window.Event("online"));
		});
		await expectRootPageOnce();
	});

	for (const [label, answer] of [
		[
			"fails",
			async () => {
				throw transportFailure();
			},
		],
		["is empty", async () => []],
	] as const) {
		test(`a root open whose catalog ${label} while auth loads keeps the root target`, async () => {
			auth.isLoading = true;
			catalog = answer;
			await openAt(COLD_OPEN_URL);
			await settle();

			auth.isLoading = false;
			await rerender();
			await expectRootPageOnce();
		});
	}

	test("a root page resolved from the catalog mounts once over the bootstrap's copy of its Event", async () => {
		auth.isLoading = true;
		catalog = async () => [{ ...EVENT, board_id: "board-1" }];
		await openAt(COLD_OPEN_URL);
		await settle();

		auth.isLoading = false;
		await rerender();
		await expectRootPageOnce();
	});

	test("a route deep link opens that route's page without naming its Event", async () => {
		network = async (_appId, route) =>
			bootstrapAt("B", route === "/reports" ? REPORTS_EVENT : EVENT);
		await openAt("/use?id=app-1&route=/reports");
		await settledPage();

		expect(bootstrapCalls).toEqual([["app-1", "/reports", undefined]]);
		expect(mounts).toHaveLength(1);
		expect(pageOnScreen()).toBe("page-reports");
		expect(navigations()).toEqual([]);
	});

	test("an Event deep link bootstraps that Event directly", async () => {
		network = async (_appId, _route, eventId) =>
			bootstrapAt("B", eventById([EVENT, DETAIL_EVENT], eventId));
		await openAt(`/use?id=app-1&eventId=${DETAIL_EVENT.id}`);
		await settledPage();

		expect(bootstrapCalls).toEqual([["app-1", undefined, DETAIL_EVENT.id]]);
		expect(mounts).toHaveLength(1);
		expect(pageOnScreen()).toBe("page-detail");
		expect(navigations()).toEqual([]);
	});

	test("a removed route falls back to the default page without another request", async () => {
		network = async () => ({ ...bootstrapAt("B"), routeMiss: true });
		await openAt("/use?id=app-1&route=/removed");
		await settledPage();

		expect(bootstrapCalls).toEqual([["app-1", "/removed", undefined]]);
		expect(mounts).toHaveLength(1);
		expect(pageOnScreen()).toBe("page-1");
		expect(navigations()).toEqual([]);
	});

	test("an Event id beside a mapped route is dropped from the address", async () => {
		network = async () => bootstrapAt("B", REPORTS_EVENT);
		await openAt("/use?id=app-1&route=/reports&eventId=event-stale");
		await settledPage();

		expect(navigations()).toEqual([["replace", "?id=app-1&route=%2Freports"]]);
		expect(window.location.search).toBe("?id=app-1&route=%2Freports");
		expect(bootstrapCalls).toEqual([["app-1", "/reports", undefined]]);
		expect(mounts).toHaveLength(1);
	});

	test("an app without a root route opens its first usable Event by id", async () => {
		network = async (_appId, _route, eventId) => {
			if (eventId === CHAT_A.id) return { event: CHAT_A };
			throw notFound();
		};
		catalog = async () => [CHAT_B, CHAT_A];
		await openAt(COLD_OPEN_URL, { eventConfig: CHAT_EVENT_CONFIG });
		await waitFor(() => eventInterface() === CHAT_A.id, "the first Event");

		expect(navigations()).toEqual([
			["replace", `?id=app-1&eventId=${CHAT_A.id}`],
		]);
		expect(storeRedirects()).toEqual([]);
	});

	test("a link to a removed Event reroutes to the first usable Event", async () => {
		network = async (_appId, _route, eventId) => {
			if (eventId === CHAT_A.id) return { event: CHAT_A };
			throw notFound();
		};
		catalog = async () => [CHAT_B, CHAT_A];
		await openAt("/use?id=app-1&eventId=chat-removed", {
			eventConfig: CHAT_EVENT_CONFIG,
		});
		await waitFor(() => eventInterface() === CHAT_A.id, "the first Event");

		expect(navigations()).toEqual([
			["replace", `?id=app-1&eventId=${CHAT_A.id}`],
		]);
		expect(storeRedirects()).toEqual([]);
	});

	test("switching Events from the header opens the chosen Event", async () => {
		network = async (_appId, _route, eventId) => ({
			event: eventById([CHAT_A, CHAT_B], eventId),
		});
		catalog = async () => [CHAT_A, CHAT_B];
		await openAt(`/use?id=app-1&eventId=${CHAT_A.id}`, {
			eventConfig: CHAT_EVENT_CONFIG,
		});
		await waitFor(
			() => eventInterface() === CHAT_A.id && Boolean(headerSwitchEvent),
			"the first Event",
		);

		await act(async () => headerSwitchEvent?.(CHAT_B.id));
		await waitFor(() => eventInterface() === CHAT_B.id, "the chosen Event");

		expect(navigations()).toEqual([["push", `?id=app-1&eventId=${CHAT_B.id}`]]);
		expect(bootstrapCalls).toEqual([
			["app-1", undefined, CHAT_A.id],
			["app-1", undefined, CHAT_B.id],
		]);
	});

	test("an offline cold open renders the restored root page without retargeting", async () => {
		online = false;
		network = async () => {
			throw transportFailure();
		};
		await render(await persistedSession(bootstrapAt("A")), FROM_URL);
		await waitFor(() => bootstrapCalls.length === 2, "the refetch to fail");
		await settledPage();

		expect(mounts).toEqual([
			{ revision: "A", executionRevision: "execution-A" },
		]);
		expect(bootstrapCalls.filter(([, route]) => route !== "/")).toEqual([]);
		expect(navigations()).toEqual([]);
	});

	test("an embedded runtime opening the app root keeps its target", async () => {
		const onNavigate = mock((_next: { eventId?: string | null }) => {});
		await render(await persistedSession(), {
			appId: "app-1",
			routePath: "/",
			eventId: null,
			embedded: true,
			eventIdTakesPrecedence: true,
			onNavigate,
		});
		await settledPage();

		expect(onNavigate).not.toHaveBeenCalled();
		expect(bootstrapCalls).toEqual([["app-1", "/", undefined]]);
		expect(mounts).toHaveLength(1);
	});

	test("an embedded runtime drops an Event id its route already maps", async () => {
		const onNavigate = mock((_next: { eventId?: string | null }) => {});
		network = async () => bootstrapAt("B", REPORTS_EVENT);
		await render(await persistedSession(), {
			appId: "app-1",
			routePath: "/reports",
			eventId: "event-stale",
			embedded: true,
			onNavigate,
		});
		await settledPage();

		expect(onNavigate.mock.calls).toEqual([[{ eventId: null }]]);
		expect(bootstrapCalls).toEqual([["app-1", "/reports", undefined]]);
		expect(mounts).toHaveLength(1);
		expect(navigations()).toEqual([]);
	});
});
