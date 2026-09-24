import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import {
	type ComponentProps,
	type ReactNode,
	useSyncExternalStore,
} from "react";
import type { Root } from "react-dom/client";
import type {
	ExploreSearchQuery,
	ExploreSearchResponse,
} from "../explore-types";

/** The first dynamic import transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;
const ORIGIN = "https://app.flow-like.com";
const SEARCH_PATH = "/store/explore/search";

const browser = new Window({ url: `${ORIGIN}${SEARCH_PATH}` });
const globalKeys = {
	window: browser,
	document: browser.document,
	navigator: browser.navigator,
	localStorage: browser.localStorage,
	HTMLElement: browser.HTMLElement,
	HTMLButtonElement: browser.HTMLButtonElement,
	HTMLInputElement: browser.HTMLInputElement,
	HTMLAnchorElement: browser.HTMLAnchorElement,
	HTMLImageElement: browser.HTMLImageElement,
	SVGElement: browser.SVGElement,
	Element: browser.Element,
	Text: browser.Text,
	DocumentFragment: browser.DocumentFragment,
	Node: browser.Node,
	MutationObserver: browser.MutationObserver,
	IntersectionObserver: browser.IntersectionObserver,
	Event: browser.Event,
	InputEvent: browser.InputEvent,
	CustomEvent: browser.CustomEvent,
	KeyboardEvent: browser.KeyboardEvent,
	FocusEvent: browser.FocusEvent,
	MouseEvent: browser.MouseEvent,
	PointerEvent: browser.PointerEvent,
	NodeFilter: browser.NodeFilter,
	ResizeObserver: browser.ResizeObserver,
	requestAnimationFrame: browser.requestAnimationFrame.bind(browser),
	cancelAnimationFrame: browser.cancelAnimationFrame.bind(browser),
	getComputedStyle: browser.getComputedStyle.bind(browser),
	matchMedia: browser.matchMedia.bind(browser),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const descriptors = Object.keys(globalKeys).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globalKeys);
Object.assign(browser, { SyntaxError, TypeError });

/** A tiny router: `replace`/`push` move the location and re-render every reader, like the real App Router. */
let location = { pathname: SEARCH_PATH, search: new URLSearchParams() };
const listeners = new Set<() => void>();
function navigate(href: string) {
	const url = new URL(href, ORIGIN);
	location = { pathname: url.pathname, search: url.searchParams };
	for (const listener of listeners) listener();
}
function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}
const router = {
	push: mock((href: string) => navigate(href)),
	replace: mock((href: string, _options?: unknown) => navigate(href)),
	prefetch: mock(() => {}),
	back: mock(() => {}),
	forward: mock(() => {}),
	refresh: mock(() => {}),
};

const actual = {
	nextNavigation: { ...(await import("next/navigation")) },
	nextLink: { ...(await import("next/link")) },
	exploreAppsPage: { ...(await import("../../explore-apps-page")) },
};
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	useRouter: () => router,
	usePathname: () => useSyncExternalStore(subscribe, () => location.pathname),
	useSearchParams: () => useSyncExternalStore(subscribe, () => location.search),
}));
mock.module("next/link", () => ({
	...actual.nextLink,
	default: ({
		href,
		children,
		prefetch: _prefetch,
		...props
	}: ComponentProps<"a"> & { href: string; prefetch?: boolean }) => (
		<a href={href} {...props}>
			{children as ReactNode}
		</a>
	),
}));
mock.module("../../explore-apps-page", () => ({
	...actual.exploreAppsPage,
	ExploreAppsPage: () => <div data-legacy-explore>Legacy Explore apps</div>,
}));

afterAll(() => {
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("next/link", () => actual.nextLink);
	mock.module("../../explore-apps-page", () => actual.exploreAppsPage);
	for (const [key, descriptor] of descriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

const { createRoot } = await import("react-dom/client");
const { act } = await import("react");

let root: Root;
let host: HTMLElement;

beforeEach(() => {
	location = { pathname: SEARCH_PATH, search: new URLSearchParams() };
	router.replace.mockClear();
	router.push.mockClear();
	browser.localStorage.clear();
	host = browser.document.createElement("div") as unknown as HTMLElement;
	browser.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
});

async function flush(ms = 20) {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function until(check: () => boolean, label: string) {
	for (let attempt = 0; attempt < 150; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(`Timed out waiting for ${label}`);
}

function query<T extends Element = HTMLElement>(selector: string): T | null {
	return host.querySelector(selector) as T | null;
}

function facet(label: string): HTMLButtonElement | undefined {
	return Array.from(host.querySelectorAll("aside button[aria-pressed]")).find(
		(button) => button.textContent?.trim().startsWith(label),
	) as HTMLButtonElement | undefined;
}

function buttonNamed(name: string): HTMLButtonElement | undefined {
	return Array.from(host.querySelectorAll("button")).find(
		(button) =>
			button.getAttribute("aria-label") === name ||
			button.textContent?.trim() === name,
	) as HTMLButtonElement | undefined;
}

async function click(element: Element | null | undefined, label: string) {
	if (!element) throw new Error(`Missing ${label}`);
	await act(async () => {
		(element as HTMLElement).click();
	});
	await flush();
}

function lastReplace(): string | undefined {
	return router.replace.mock.calls.at(-1)?.[0];
}

function searchInput(): HTMLInputElement {
	const input = query<HTMLInputElement>('input[type="search"]');
	if (!input) throw new Error("Missing the search input");
	return input;
}

async function type(value: string) {
	const input = searchInput();
	const setter = Object.getOwnPropertyDescriptor(
		browser.HTMLInputElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setter?.call(input, value);
		input.dispatchEvent(new InputEvent("input", { bubbles: true }));
	});
}

interface MountOptions {
	url?: string;
	developerMode?: boolean;
	signedIn?: boolean;
	searchExplore?: (
		query: ExploreSearchQuery,
	) => Promise<ExploreSearchResponse> | ExploreSearchResponse;
}

async function mount({
	url = "",
	developerMode = true,
	signedIn = true,
	searchExplore,
}: MountOptions = {}) {
	navigate(`${SEARCH_PATH}${url}`);
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { AuthContext } = await import("react-oidc-context");
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../../../state/backend-state"
	);
	const { exploreSearchFixture } = await import("../explore-fixture");
	const { ExploreBrowsePage } = await import("./explore-browse-page");
	const calls: ExploreSearchQuery[] = [];
	useBackendStore.getState().setBackend({
		appState: {
			searchExplore: async (search: ExploreSearchQuery) => {
				calls.push(search);
				return searchExplore
					? searchExplore(search)
					: exploreSearchFixture({ dev: search.dev });
			},
			getApps: async () => [],
			getStoreGroups: async () => [],
		},
		userState: {
			getInfo: async () => ({ dev_mode: developerMode, permission: 0 }),
			updateUser: async () => {},
		},
		routeState: { getRoutes: async () => [] },
		eventState: { getEvents: async () => [] },
	} as never);
	useAuthStatusStore.setState({ signedIn });
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<AuthContext.Provider value={{ isLoading: false } as never}>
				<QueryClientProvider client={client}>
					<ExploreBrowsePage canScaffold />
				</QueryClientProvider>
			</AuthContext.Provider>,
		),
	);
	await flush();
	return { calls };
}

async function mountResults(options: MountOptions = {}) {
	const mounted = await mount(options);
	await until(
		() =>
			mounted.calls.at(-1)?.dev === (options.developerMode ?? true) &&
			!!query('[data-browse-group="apps"]'),
		"the results",
	);
	return mounted;
}

async function legacyHub() {
	const { toExploreError } = await import("../explore-model");
	return () => {
		throw toExploreError(
			Object.assign(new Error("Not Found"), { status: 404 }),
		);
	};
}

test(
	"a facet writes a canonical query with replace and searches with it",
	async () => {
		const { calls } = await mountResults({ url: "?q=invoice" });
		router.replace.mockClear();

		await click(facet("Paid"), "the Paid facet");

		expect(lastReplace()).toBe(`${SEARCH_PATH}?q=invoice&price=paid`);
		expect(router.replace.mock.calls.at(-1)?.[1]).toEqual({ scroll: false });
		await until(() => calls.at(-1)?.price === "paid", "the paid search");
		expect(calls.at(-1)?.q).toBe("invoice");
		expect(facet("Paid")?.getAttribute("aria-pressed")).toBe("true");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a URL change made elsewhere is adopted without being written back",
	async () => {
		const { calls } = await mountResults({ url: "?q=invoice" });
		router.replace.mockClear();

		await act(async () => navigate(`${SEARCH_PATH}?q=pdf&type=apps`));
		await flush();

		expect(searchInput().value).toBe("pdf");
		await until(
			() => calls.at(-1)?.q === "pdf" && calls.at(-1)?.type === "apps",
			"the adopted search",
		);
		await flush(350);
		expect(router.replace).not.toHaveBeenCalled();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"the category facet takes no new pick past 16",
	async () => {
		const { APP_CATEGORIES } = await import("../explore-types");
		const { exploreSearchFixture } = await import("../explore-fixture");
		const categories = APP_CATEGORIES.slice(0, 17);
		const selected = categories.slice(0, 16).map((name) => `app:${name}`);
		await mountResults({
			url: `?categories=${selected.join(",")}`,
			searchExplore: (search) => {
				const response = exploreSearchFixture({ dev: search.dev });
				response.facets.categories = categories.map((name) => ({
					value: `app:${name}`,
					kind: "app" as const,
					count: 2,
				}));
				return response;
			},
		});
		const buttons = Array.from(
			host.querySelectorAll<HTMLButtonElement>(
				"aside fieldset:nth-of-type(2) button[aria-pressed]",
			),
		);
		expect(buttons).toHaveLength(17);
		const pressed = buttons.filter(
			(button) => button.getAttribute("aria-pressed") === "true",
		);
		const open = buttons.filter(
			(button) => button.getAttribute("aria-pressed") === "false",
		);
		expect(pressed).toHaveLength(16);
		expect(pressed.every((button) => !button.disabled)).toBe(true);
		expect(open).toHaveLength(1);
		expect(open[0]?.disabled).toBe(true);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a collection that is gone says so",
	async () => {
		await mount({
			url: "?collection=gone",
			searchExplore: () => {
				throw Object.assign(
					new Error("Explore collection gone is not available"),
					{ status: 404, code: "not_found" },
				);
			},
		});
		await until(
			() =>
				host.textContent?.includes("This collection is not available") ?? false,
			"the unavailable message",
		);
		expect(query('a[href="/store/explore"]')).not.toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"Clear all inside a collection keeps the collection",
	async () => {
		await mountResults({ url: "?collection=collection-invoices&price=paid" });
		router.replace.mockClear();

		await click(buttonNamed("Clear all"), "Clear all");

		expect(lastReplace()).toBe(`${SEARCH_PATH}?collection=collection-invoices`);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"opening another collection never shows the previous results under its header",
	async () => {
		const { exploreSearchFixture } = await import("../explore-fixture");
		let release: (() => void) | undefined;
		await mountResults({
			url: "?q=invoice",
			searchExplore: (search) => {
				const response = exploreSearchFixture({ dev: search.dev });
				if (search.collection !== "other") return response;
				const [first] = response.collections;
				response.collections = first
					? [{ ...first, placementId: "other", title: "Other picks" }]
					: [];
				return new Promise((resolve) => {
					release = () => resolve(response);
				});
			},
		});
		expect(host.textContent).toContain("Automate your invoices");

		await act(async () => navigate(`${SEARCH_PATH}?collection=other`));
		await until(() => !!release, "the collection request");

		expect(query("h1")?.textContent).toBe("Collection");
		expect(query('[data-browse-group="apps"]')).toBeNull();
		expect(host.textContent).not.toContain("Automate your invoices");

		await act(async () => release?.());
		await until(
			() => query("h1")?.textContent === "Other picks",
			"the opened collection",
		);
		expect(query('[data-browse-collection="other"]')).not.toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a legacy hub gets the old apps page with the filters it understands",
	async () => {
		const { calls } = await mount({
			url: "?type=apps&categories=app:Finance&sort=rating",
			searchExplore: await legacyHub(),
		});
		await until(() => !!query("[data-legacy-explore]"), "the legacy page");

		expect(router.replace.mock.calls.map(([href]) => href)).toContain(
			`${SEARCH_PATH}?type=apps&category=Finance&sort=rated`,
		);
		expect(calls).toHaveLength(1);

		await act(async () => navigate(`${SEARCH_PATH}?category=Health`));
		await flush(350);
		expect(query("[data-legacy-explore]")).not.toBeNull();
		expect(calls).toHaveLength(1);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a legacy hub sends packages to the old list once developer mode is known",
	async () => {
		const failing = await legacyHub();
		await mount({
			url: "?type=packages",
			developerMode: false,
			searchExplore: failing,
		});
		await until(() => !!query("[data-legacy-explore]"), "the legacy page");
		expect(router.replace).not.toHaveBeenCalled();
		expect(location.search.get("type")).toBe("packages");

		await act(() => root.unmount());
		root = createRoot(host);
		router.replace.mockClear();
		// The developer-mode mirror still says false from the mount above, so the hub's true arrives late.
		await mount({ url: "?type=packages", searchExplore: failing });
		await until(
			() =>
				router.replace.mock.calls.some(
					([href]) => href === "/store/packages?tab=explore",
				),
			"the package list redirect",
		);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"without developer mode a packages link shows what the viewer can see",
	async () => {
		const { calls } = await mountResults({
			url: "?type=packages",
			developerMode: false,
		});
		expect(calls.at(-1)?.type).toBeUndefined();
		expect(query('[data-browse-group="packages"]')).toBeNull();
		expect(buttonNamed("Remove filter Packages")).toBeUndefined();
		expect(host.textContent).not.toContain("Nothing matches these filters");
		expect(location.search.get("type")).toBe("packages");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a capped package list still pages within the cap",
	async () => {
		const { exploreSearchFixture } = await import("../explore-fixture");
		await mountResults({
			url: "?q=invoice&permissions=network",
			searchExplore: (search) => {
				const response = exploreSearchFixture({ dev: search.dev });
				response.packages.hasMore = true;
				response.facets.packagesCapped = true;
				return response;
			},
		});
		await until(
			() => !!query('[data-browse-group="packages"]'),
			"the packages group",
		);
		expect(buttonNamed("Show more packages")).toBeDefined();
		expect(host.textContent).not.toContain(
			"Refine your filters to see more packages.",
		);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a members-only hub asks a signed-out viewer to sign in",
	async () => {
		await mount({
			url: "?q=invoice",
			signedIn: false,
			searchExplore: () => {
				throw Object.assign(new Error("Unauthorized"), { status: 401 });
			},
		});
		await until(
			() => host.textContent?.includes("Sign in to explore this hub") ?? false,
			"the sign-in prompt",
		);
		expect(query("[data-legacy-explore]")).toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a server error shows the inline error, and Retry searches again",
	async () => {
		const { calls } = await mount({
			url: "?q=invoice",
			searchExplore: () => {
				throw Object.assign(new Error("Internal Server Error"), {
					status: 500,
				});
			},
		});
		await flush(1_300);
		await until(
			() =>
				host.textContent?.includes(
					"Explore could not be loaded. Please try again.",
				) ?? false,
			"the inline error",
		);
		const before = calls.length;

		await click(buttonNamed("Retry"), "Retry");

		await until(() => calls.length > before, "the retried search");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"only a submitted query becomes a recent search",
	async () => {
		const readRecent = () =>
			JSON.parse(
				browser.localStorage.getItem("flow-like.explore.recent-searches") ??
					"[]",
			);
		await mountResults({ url: "?q=invoice" });

		await type("inv");
		await flush(350);
		expect(searchInput().value).toBe("inv");
		expect(readRecent()).toEqual([]);

		const form = searchInput().closest("form");
		await act(async () => {
			form?.dispatchEvent(
				new Event("submit", { bubbles: true, cancelable: true }),
			);
		});
		await flush();
		expect(readRecent()).toEqual(["inv"]);
	},
	COLD_IMPORT_TIMEOUT_MS,
);
