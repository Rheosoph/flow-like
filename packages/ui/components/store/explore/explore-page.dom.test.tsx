import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import type { ComponentProps, ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { ExploreQuery, ResolvedExplore } from "./explore-types";

/** The first dynamic import transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;

const browser = new Window({ url: "https://app.flow-like.com/store/explore" });
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

let pathname = "/store/explore";
let searchParams = new URLSearchParams();
const router = {
	push: mock((_href: string) => {}),
	replace: mock((_href: string, _options?: unknown) => {}),
	prefetch: mock(() => {}),
	back: mock(() => {}),
	forward: mock(() => {}),
	refresh: mock(() => {}),
};

// bun keeps a module mock for every later file in the process, so each mocked module is captured first and put
// back in afterAll.
const actual = {
	nextNavigation: { ...(await import("next/navigation")) },
	nextLink: { ...(await import("next/link")) },
	exploreAppsPage: { ...(await import("../explore-apps-page")) },
};
mock.module("next/navigation", () => ({
	...actual.nextNavigation,
	useRouter: () => router,
	usePathname: () => pathname,
	useSearchParams: () => searchParams,
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
mock.module("../explore-apps-page", () => ({
	...actual.exploreAppsPage,
	ExploreAppsPage: () => <div data-legacy-explore>Legacy Explore apps</div>,
}));

afterAll(() => {
	mock.module("next/navigation", () => actual.nextNavigation);
	mock.module("next/link", () => actual.nextLink);
	mock.module("../explore-apps-page", () => actual.exploreAppsPage);
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
	pathname = "/store/explore";
	searchParams = new URLSearchParams();
	router.replace.mockClear();
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
	for (let attempt = 0; attempt < 100; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(`Timed out waiting for ${label}`);
}

function query<T extends Element = HTMLElement>(selector: string): T | null {
	return host.querySelector(selector) as T | null;
}

function buttonNamed(name: string): HTMLButtonElement | undefined {
	return Array.from(host.querySelectorAll("button")).find(
		(button) =>
			button.getAttribute("aria-label") === name ||
			button.textContent?.trim().startsWith(name),
	) as HTMLButtonElement | undefined;
}

async function click(element: Element | null | undefined, label: string) {
	if (!element) throw new Error(`Missing ${label}`);
	await act(async () => {
		(element as HTMLElement).click();
	});
	await flush();
}

interface MountOptions {
	developerMode: boolean;
	getExplore?: (query: ExploreQuery) => Promise<ResolvedExplore>;
	/** What the host pushed; `undefined` is a host that never pushes its auth state. */
	signedIn?: boolean;
}

async function mount(options: MountOptions) {
	const { developerMode, getExplore } = options;
	const signedIn = "signedIn" in options ? options.signedIn : true;
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { AuthContext } = await import("react-oidc-context");
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../../state/backend-state"
	);
	const { exploreFixture } = await import("./explore-fixture");
	const { ExplorePage } = await import("./explore-page");
	const calls: ExploreQuery[] = [];
	useBackendStore.getState().setBackend({
		appState: {
			getExplore: async (explore: ExploreQuery) => {
				calls.push(explore);
				return getExplore
					? getExplore(explore)
					: exploreFixture({ dev: explore.dev });
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
					<ExplorePage />
				</QueryClientProvider>
			</AuthContext.Provider>,
		),
	);
	await flush();
	return { calls };
}

async function mountDev() {
	const mounted = await mount({ developerMode: true });
	await until(
		() => !!query("[data-explore-type-filter]"),
		"the dev landing with its type filter",
	);
	return mounted;
}

function slot(name: string): HTMLElement | null {
	return query(`[data-slot="${name}"]`);
}

test(
	"dismissing the announcement grows the feature tile to two rows and offers Restore",
	async () => {
		await mountDev();
		expect(slot("notice")).not.toBeNull();
		expect(slot("feature")?.dataset.rowSpan).toBe("1");
		expect(slot("feature")?.getAttribute("style")).toContain(
			"--xl-row: 2 / span 1",
		);

		await click(buttonNamed("Dismiss announcement"), "dismiss button");

		expect(slot("notice")).toBeNull();
		expect(slot("feature")?.dataset.rowSpan).toBe("2");
		expect(slot("feature")?.getAttribute("style")).toContain(
			"--xl-row: 1 / span 2",
		);
		const restore = buttonNamed("Restore announcement");
		expect(restore).toBeDefined();
		expect(browser.document.activeElement).toBe(restore as never);

		await click(restore, "restore button");
		expect(slot("notice")).not.toBeNull();
		expect(buttonNamed("Restore announcement")).toBeUndefined();
		expect(browser.document.activeElement).toBe(
			buttonNamed("Dismiss announcement") as never,
		);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"an announcement dismissed earlier is never painted",
	async () => {
		const { dismissStorage } = await import("./explore-model");
		const { exploreFixture } = await import("./explore-fixture");
		const key = exploreFixture({ dev: true }).views.all.grid.notice?.dismissKey;
		expect(key).toBeTruthy();
		dismissStorage.dismiss(key ?? "");
		let painted = false;
		const observer = new browser.MutationObserver((records) => {
			for (const record of records) {
				for (const node of Array.from(record.addedNodes)) {
					const element = node as unknown as Element;
					painted ||=
						element.matches?.('[data-slot="notice"]') ||
						!!element.querySelector?.('[data-slot="notice"]');
				}
			}
		});
		observer.observe(host as never, { childList: true, subtree: true });

		await mountDev();
		observer.disconnect();

		expect(painted).toBe(false);
		expect(slot("feature")?.dataset.rowSpan).toBe("2");
		expect(buttonNamed("Restore announcement")).toBeDefined();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"without developer mode there is no type filter and no package card",
	async () => {
		const { calls } = await mount({ developerMode: false });
		await until(() => !!query("[data-explore-bento]"), "the landing");
		await until(
			() => calls.at(-1)?.dev === false && !!query("[data-explore-app]"),
			"the non-dev page",
		);
		expect(query("[data-explore-type-filter]")).toBeNull();
		expect(query("[data-package-card]")).toBeNull();
		expect(host.textContent).not.toContain("Code Interpreter");
		expect(query("[data-explore-app]")).not.toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a dev page still gets no type filter while developer mode is off",
	async () => {
		const { exploreFixture } = await import("./explore-fixture");
		searchParams = new URLSearchParams("type=packages");
		const { calls } = await mount({
			developerMode: false,
			getExplore: async () => exploreFixture({ dev: true }),
		});
		await until(
			() => calls.length > 0 && !!query("[data-explore-bento]"),
			"the landing",
		);
		expect(calls.at(-1)?.dev).toBe(false);
		expect(query("[data-explore-type-filter]")).toBeNull();
		expect(query("[data-explore-view]")?.dataset.exploreView).toBe("all");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a host that never pushes its auth state still reaches the legacy page",
	async () => {
		const { EXPLORE_AUTH_STATE_GRACE_MS } = await import("./use-explore");
		const { toExploreError } = await import("./explore-model");
		const { calls } = await mount({
			developerMode: true,
			signedIn: undefined,
			getExplore: async () => {
				throw toExploreError(
					Object.assign(new Error("Not Found"), { status: 404 }),
				);
			},
		});
		expect(calls).toEqual([]);
		expect(query("[data-legacy-explore]")).toBeNull();

		await flush(EXPLORE_AUTH_STATE_GRACE_MS + 100);

		await until(() => !!query("[data-legacy-explore]"), "the legacy page");
		expect(calls).toHaveLength(1);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"the Apps filter switches to the apps view and its fallbacks",
	async () => {
		await mountDev();
		expect(slot("feature")?.textContent).toContain("Typst Documents");

		await click(buttonNamed("Apps"), "Apps filter");

		expect(buttonNamed("Apps")?.getAttribute("aria-pressed")).toBe("true");
		expect(query("[data-explore-view]")?.dataset.exploreView).toBe("apps");
		expect(slot("feature")?.textContent).toContain("Invoice Autopilot");
		expect(slot("collection")?.textContent).toContain(
			"Agents that stay in bounds",
		);
		const rowTitles = Array.from(
			host.querySelectorAll("[data-explore-row] h2"),
		).map((heading) => heading.textContent);
		expect(rowTitles).toContain("New this week");
		expect(rowTitles).not.toContain("For builders");
		expect(router.replace.mock.calls.at(-1)?.[0]).toBe(
			"/store/explore?type=apps",
		);
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"the Packages filter shows no app cards",
	async () => {
		await mountDev();
		await click(buttonNamed("Packages"), "Packages filter");

		expect(query("[data-explore-view]")?.dataset.exploreView).toBe("packages");
		expect(query("[data-explore-app]")).toBeNull();
		expect(query("[data-package-card]")).not.toBeNull();
		expect(host.textContent).not.toContain("Customer Support Copilot");
		expect(host.textContent).not.toContain("Invoice Autopilot");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a hub without Explore (codeless 404) renders the legacy apps page",
	async () => {
		const { toExploreError } = await import("./explore-model");
		await mount({
			developerMode: true,
			getExplore: async () => {
				throw toExploreError(
					Object.assign(new Error("Not Found"), { status: 404 }),
				);
			},
		});
		await until(() => !!query("[data-legacy-explore]"), "the legacy page");
		expect(query("[data-explore-bento]")).toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a hub with nothing but the suites row shows the empty state",
	async () => {
		const { exploreFixture } = await import("./explore-fixture");
		await mount({
			developerMode: false,
			getExplore: async () => {
				const page = exploreFixture({ dev: false });
				return {
					...page,
					typeCounts: { apps: 0, packages: 0 },
					views: {
						all: {
							grid: {},
							rows: [
								{
									kind: "rail",
									rail: {
										placementId: "row:suites",
										rail: "suites",
										items: [],
									},
								},
							],
						},
						apps: null,
						packages: null,
					},
				};
			},
		});
		await until(
			() => host.textContent?.includes("Nothing to explore yet") ?? false,
			"the empty state",
		);
		expect(query("[data-explore-bento]")).toBeNull();
		expect(buttonNamed("Browse everything")).toBeDefined();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a members-only hub asks a signed-out viewer to sign in",
	async () => {
		const { calls } = await mount({
			developerMode: false,
			signedIn: false,
			getExplore: async () => {
				throw Object.assign(new Error("Unauthorized"), { status: 401 });
			},
		});
		await until(
			() => host.textContent?.includes("Sign in to explore this hub") ?? false,
			"the sign-in prompt",
		);
		expect(calls).toHaveLength(1);
		expect(query("[data-legacy-explore]")).toBeNull();
	},
	COLD_IMPORT_TIMEOUT_MS,
);

/** Explore retries a transport or server failure once, after about a second, before it shows the failure. */
const RETRY_SETTLE_MS = 1_300;

test(
	"an unreachable hub shows the offline state, and Retry asks again",
	async () => {
		const { calls } = await mount({
			developerMode: false,
			getExplore: async () => {
				throw new TypeError("Failed to fetch");
			},
		});
		await flush(RETRY_SETTLE_MS);
		await until(
			() => host.textContent?.includes("You're offline") ?? false,
			"the offline state",
		);
		const before = calls.length;

		await click(buttonNamed("Retry"), "Retry");

		await until(() => calls.length > before, "the retried request");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"a server error shows the inline error, and Retry asks again",
	async () => {
		const { calls } = await mount({
			developerMode: false,
			getExplore: async () => {
				throw Object.assign(new Error("Internal Server Error"), {
					status: 500,
				});
			},
		});
		await flush(RETRY_SETTLE_MS);
		await until(
			() =>
				host.textContent?.includes(
					"Explore could not be loaded. Please try again.",
				) ?? false,
			"the inline error",
		);
		expect(query('a[href="/store/explore/search"]')?.textContent).toBe(
			"Browse everything",
		);
		const before = calls.length;

		await click(buttonNamed("Retry"), "Retry");

		await until(() => calls.length > before, "the retried request");
	},
	COLD_IMPORT_TIMEOUT_MS,
);

test(
	"package cards link to the store with the Explore state as from=",
	async () => {
		searchParams = new URLSearchParams("type=packages");
		await mountDev();
		expect(query("[data-explore-view]")?.dataset.exploreView).toBe("packages");
		const card = query<HTMLAnchorElement>("a[data-package-card]");
		expect(card).not.toBeNull();
		const href = new URL(card?.getAttribute("href") ?? "", "https://x.test");
		expect(href.pathname).toBe("/store/packages");
		expect(href.searchParams.get("from")).toBe("/store/explore?type=packages");
		expect(href.searchParams.get("id")).toBeTruthy();
	},
	COLD_IMPORT_TIMEOUT_MS,
);
