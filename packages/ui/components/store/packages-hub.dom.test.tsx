import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { PackageStatus, type PackageSummary } from "../../lib/schema/wasm";
import type { LibraryAuth } from "./package-library/use-library-packages";

/** The first dynamic import transpiles the backend-state graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 30_000;
const SKELETON_SELECTOR = '[data-packages-hub-skeleton][aria-busy="true"]';

// bun keeps a module mock for every later file in the process, so the real module is put back in afterAll.
const actualNavigation = { ...(await import("next/navigation")) };
const navigation = {
	params: new URLSearchParams(),
	replace: mock((_href: string) => {}),
	push: mock((_href: string) => {}),
};
const router = {
	replace: (href: string) => navigation.replace(href),
	push: (href: string) => navigation.push(href),
	back: () => {},
	forward: () => {},
	refresh: () => {},
	prefetch: () => {},
};
mock.module("next/navigation", () => ({
	...actualNavigation,
	useRouter: () => router,
	useSearchParams: () => navigation.params,
}));
afterAll(() => {
	mock.module("next/navigation", () => actualNavigation);
});

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/store/packages" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		Element: window.Element,
		Text: window.Text,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		IntersectionObserver: window.IntersectionObserver,
		Event: window.Event,
		InputEvent: window.InputEvent,
		CustomEvent: window.CustomEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		MouseEvent: window.MouseEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		ResizeObserver: window.ResizeObserver,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
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

	navigation.params = new URLSearchParams();
	navigation.replace.mockClear();
	navigation.push.mockClear();
	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	await window.happyDOM.abort();
	restoreGlobals();
});

type Mocked = Record<string, (...args: never[]) => Promise<unknown>>;

interface Backend {
	appState?: Mocked;
	registryState?: Mocked;
}

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function until(check: () => boolean, label: string, attempts = 50) {
	for (let attempt = 0; attempt < attempts; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(
		`Timed out waiting for ${label}: ${host.textContent?.slice(0, 300)}`,
	);
}

function authFor(
	signedIn: boolean | undefined,
	isLoading = false,
): LibraryAuth {
	return {
		isLoading,
		isAuthenticated: signedIn === true,
		user: signedIn
			? { access_token: "token", profile: { sub: "user-1" } }
			: null,
		signinRedirect: async () => {},
	};
}

async function render(
	node: (auth: LibraryAuth) => ReactNode,
	{
		backend,
		signedIn,
		authLoading = false,
		search = "",
	}: {
		backend: Backend;
		signedIn: boolean | undefined;
		authLoading?: boolean;
		search?: string;
	},
) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { AuthContext } = await import("react-oidc-context");
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../state/backend-state"
	);
	navigation.params = new URLSearchParams(search);
	useBackendStore.getState().setBackend({
		profile: undefined,
		userState: {
			getInfo: async () => ({}),
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
		},
		appState: {},
		registryState: {},
		...backend,
	} as never);
	useAuthStatusStore.setState({ signedIn });
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const content = () => (
		<QueryClientProvider client={client}>
			{node(authFor(signedIn, authLoading))}
		</QueryClientProvider>
	);
	// Another file's react-oidc-context mock may lack AuthContext; useAuth() then warns and returns undefined.
	const draw = async () => {
		await act(async () =>
			root.render(
				AuthContext ? (
					<AuthContext.Provider
						value={{ isLoading: authLoading } as AuthContextProps}
					>
						{content()}
					</AuthContext.Provider>
				) : (
					content()
				),
			),
		);
		await flush();
	};
	await draw();
	return { rerender: draw, client };
}

function summary(
	id: string,
	status: PackageStatus,
	overrides: Partial<PackageSummary> = {},
): PackageSummary {
	return {
		id,
		name: id,
		description: `${id} description`,
		latestVersion: "1.2.0",
		downloadCount: 7,
		status,
		keywords: [],
		verified: false,
		price: 0,
		visibility: "public",
		viewerPermission: 1,
		...overrides,
	};
}

async function renderHub(
	backend: Backend,
	options: { signedIn: boolean | undefined; search?: string },
) {
	const { PackagesHubPage } = await import("./packages-hub");
	const fetcher = mock(async () => ({
		packages: [],
		totalCount: 0,
		offset: 0,
		limit: 12,
	}));
	const { rerender } = await render(
		(auth) => (
			<PackagesHubPage
				fetcher={fetcher as never}
				auth={auth}
				mine={(nav) => <section data-mine>{nav}</section>}
				library={(nav) => <section data-library>{nav}</section>}
			/>
		),
		{ backend, ...options },
	);
	return { fetcher, rerender };
}

function hubTab(name: string) {
	return Array.from(
		host.querySelectorAll<HTMLElement>('[data-packages-hub-nav] [role="tab"]'),
	).find((tab) => tab.textContent?.includes(name));
}

async function press(target: HTMLElement | undefined, key: string) {
	const { act } = await import("react");
	await act(async () => {
		target?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
	});
	await flush();
}

async function renderMine(
	registryState: Mocked,
	options: { signedIn: boolean | undefined; authLoading?: boolean },
) {
	const { RegistryMinePackages } = await import("./registry-mine-packages");
	const { client } = await render(
		(auth) => (
			<RegistryMinePackages
				auth={auth}
				navigation={<nav data-nav>Mine · Library</nav>}
			/>
		),
		{ backend: { registryState }, ...options },
	);
	return client;
}

/** A settled push clears use-explore's app-wide "waited too long" flag, so the next test waits again. */
async function settleAuth(signedIn: boolean) {
	const { act } = await import("react");
	const { useAuthStatusStore } = await import("../../state/backend-state");
	await act(async () => useAuthStatusStore.setState({ signedIn }));
	await flush();
}

/** `until` polls every ≥10 ms; this many attempts outlast Explore's wait for the host's auth state. */
async function graceAttempts() {
	const { EXPLORE_AUTH_STATE_GRACE_MS } = await import("./explore/use-explore");
	return EXPLORE_AUTH_STATE_GRACE_MS / 5;
}

function exploreBackend(getExplore: () => Promise<unknown>) {
	const calls: string[] = [];
	return {
		calls,
		backend: {
			appState: {
				getExplore: async () => {
					calls.push("explore");
					return getExplore();
				},
			},
		},
	};
}

describe("Explore tab", () => {
	test(
		"waits on a skeleton while the auth state is unknown, without asking the hub",
		async () => {
			const explore = exploreBackend(async () => ({}));
			await renderHub(explore.backend, { signedIn: undefined });
			expect(explore.calls).toEqual([]);
			expect(host.querySelector(SKELETON_SELECTOR)).not.toBe(null);
			expect(host.querySelector("main")).toBe(null);
			expect(navigation.replace).not.toHaveBeenCalled();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a host that never pushes its auth state asks once as signed out after the grace period, then goes to Browse",
		async () => {
			const explore = exploreBackend(async () => ({}));
			await renderHub(explore.backend, { signedIn: undefined });
			expect(host.querySelector(SKELETON_SELECTOR)).not.toBe(null);
			expect(explore.calls).toEqual([]);
			await until(
				() => navigation.replace.mock.calls.length > 0,
				"the Browse redirect after the grace period",
				await graceAttempts(),
			);
			expect(explore.calls).toEqual(["explore"]);
			expect(navigation.replace.mock.calls).toEqual([
				["/store/explore/search?type=packages"],
			]);
			await settleAuth(false);
			expect(explore.calls).toEqual(["explore"]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a host that never pushes its auth state on a hub without Explore gets the legacy list after the grace period",
		async () => {
			const { ExploreUnsupportedError } = await import(
				"./explore/explore-types"
			);
			const explore = exploreBackend(async () => {
				throw new ExploreUnsupportedError();
			});
			const { fetcher } = await renderHub(explore.backend, {
				signedIn: undefined,
			});
			expect(host.querySelector(SKELETON_SELECTOR)).not.toBe(null);
			expect(explore.calls).toEqual([]);
			await until(
				() => host.textContent?.includes("Discover and install") ?? false,
				"the legacy package list after the grace period",
				await graceAttempts(),
			);
			expect(explore.calls).toEqual(["explore"]);
			await until(() => fetcher.mock.calls.length > 0, "the legacy search");
			expect(navigation.replace).not.toHaveBeenCalled();
			await settleAuth(false);
			expect(host.textContent).toContain("Discover and install");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("redirects to Browse packages when the hub has Explore", async () => {
		const explore = exploreBackend(async () => ({}));
		await renderHub(explore.backend, { signedIn: false });
		await until(
			() => navigation.replace.mock.calls.length > 0,
			"the Browse redirect",
		);
		expect(navigation.replace.mock.calls[0]?.[0]).toBe(
			"/store/explore/search?type=packages",
		);
	});

	test("?tab=explore redirects too", async () => {
		const explore = exploreBackend(async () => ({}));
		await renderHub(explore.backend, {
			signedIn: true,
			search: "tab=explore",
		});
		await until(
			() => navigation.replace.mock.calls.length > 0,
			"the Browse redirect",
		);
		expect(navigation.replace.mock.calls[0]?.[0]).toBe(
			"/store/explore/search?type=packages",
		);
	});

	test("a 401 still redirects, so Browse shows its own sign-in state", async () => {
		const explore = exploreBackend(async () => {
			throw Object.assign(new Error("Unauthorized"), { status: 401 });
		});
		await renderHub(explore.backend, { signedIn: false });
		await until(
			() => navigation.replace.mock.calls.length > 0,
			"the Browse redirect",
		);
		expect(host.textContent).not.toContain("Discover and install");
	});

	test("a 500 ends the wait with a redirect after its one retry", async () => {
		const explore = exploreBackend(async () => {
			throw Object.assign(new Error("Internal"), { status: 500 });
		});
		await renderHub(explore.backend, { signedIn: true });
		await until(
			() => navigation.replace.mock.calls.length > 0,
			"the Browse redirect",
			300,
		);
		expect(explore.calls).toEqual(["explore", "explore"]);
	});

	test("a hub without Explore (codeless 404) keeps the legacy package list", async () => {
		const { ExploreUnsupportedError } = await import("./explore/explore-types");
		const explore = exploreBackend(async () => {
			throw new ExploreUnsupportedError();
		});
		const { fetcher } = await renderHub(explore.backend, { signedIn: true });
		await until(
			() => host.textContent?.includes("Discover and install") ?? false,
			"the legacy package list",
		);
		expect(navigation.replace).not.toHaveBeenCalled();
		await until(() => fetcher.mock.calls.length > 0, "the legacy search");
		const nav = host.querySelector("[data-packages-hub-nav]");
		expect(nav?.textContent).toContain("Mine");
		expect(nav?.textContent).toContain("Library");
		expect(
			nav?.querySelector('a[aria-current="page"]')?.getAttribute("href"),
		).toBe("/store/packages");
	});

	test("signing in on a hub without Explore keeps the legacy list mounted while support is re-checked", async () => {
		const { ExploreUnsupportedError } = await import("./explore/explore-types");
		const explore = exploreBackend(() =>
			explore.calls.length === 1
				? Promise.reject(new ExploreUnsupportedError())
				: new Promise(() => {}),
		);
		await renderHub(explore.backend, { signedIn: false });
		await until(
			() => host.textContent?.includes("Discover and install") ?? false,
			"the legacy package list",
		);
		const search = host.querySelector("input");
		expect(search).not.toBe(null);

		await settleAuth(true);
		expect(explore.calls).toEqual(["explore", "explore"]);
		expect(host.querySelector(SKELETON_SELECTOR)).toBe(null);
		expect(host.querySelector("input")).toBe(search);
		expect(navigation.replace).not.toHaveBeenCalled();
	});
});

describe("Mine · Library tabs", () => {
	test("?tab=library renders Library with the tabs and the Explore packages link", async () => {
		const explore = exploreBackend(async () => ({}));
		await renderHub(explore.backend, {
			signedIn: true,
			search: "tab=library",
		});
		const library = host.querySelector("[data-library]");
		expect(library).not.toBe(null);
		expect(host.querySelector("[data-mine]")).toBe(null);
		expect(library?.textContent).toContain("Mine");
		expect(library?.textContent).not.toContain("Apps");
		expect(
			library?.querySelector("[data-packages-hub-nav] a")?.getAttribute("href"),
		).toBe("/store/explore/search?type=packages");
		expect(explore.calls).toEqual([]);
		expect(navigation.replace).not.toHaveBeenCalled();
	});

	test("the legacy aliases render their tab and canonicalise the URL", async () => {
		await renderHub({}, { signedIn: true, search: "tab=installed" });
		expect(host.querySelector("[data-library]")).not.toBe(null);
		expect(navigation.replace.mock.calls[0]?.[0]).toBe(
			"/store/packages?tab=library",
		);

		await renderHub({}, { signedIn: true, search: "tab=projects" });
		expect(host.querySelector("[data-mine]")).not.toBe(null);
		expect(navigation.replace.mock.calls.at(-1)?.[0]).toBe(
			"/store/packages?tab=mine",
		);
	});

	test("switching tabs pushes the new tab", async () => {
		const { act } = await import("react");
		await renderHub({}, { signedIn: true, search: "tab=mine" });
		const trigger = Array.from(host.querySelectorAll('[role="tab"]')).find(
			(tab) => tab.textContent?.includes("Library"),
		);
		expect(trigger).toBeDefined();
		await act(async () => {
			trigger?.dispatchEvent(
				new MouseEvent("mousedown", { bubbles: true, button: 0 }),
			);
		});
		expect(navigation.push.mock.calls[0]?.[0]).toBe(
			"/store/packages?tab=library",
		);
	});

	test("arrows only move focus; Enter switches and focus lands on the new panel's active tab", async () => {
		const { act } = await import("react");
		const { rerender } = await renderHub(
			{},
			{ signedIn: true, search: "tab=mine" },
		);
		await act(async () => hubTab("Mine")?.focus());
		await press(hubTab("Mine"), "ArrowRight");
		expect(hubTab("Library")).toBeDefined();
		expect(host.ownerDocument.activeElement).toBe(hubTab("Library") ?? null);
		expect(navigation.push).not.toHaveBeenCalled();

		await press(hubTab("Library"), "Enter");
		expect(navigation.push.mock.calls[0]?.[0]).toBe(
			"/store/packages?tab=library",
		);
		const previous = hubTab("Library");

		navigation.params = new URLSearchParams("tab=library");
		await rerender();
		expect(host.querySelector("[data-library]")).not.toBe(null);
		const active = hubTab("Library");
		expect(active).toBeDefined();
		expect(active).not.toBe(previous);
		expect(active?.getAttribute("aria-selected")).toBe("true");
		expect(host.ownerDocument.activeElement).toBe(active ?? null);
	});
});

describe("web Mine", () => {
	const PACKAGES = [
		summary("live-pkg", PackageStatus.Active),
		summary("team-pkg", PackageStatus.Active, { viewerPermission: 2 }),
		summary("review-pkg", PackageStatus.PendingReview),
		summary("off-pkg", PackageStatus.Disabled),
		summary("old-pkg", PackageStatus.Deprecated),
	];

	function registry(log: unknown[], result: () => Promise<unknown>) {
		return {
			getOwnedPackages: async (filters: unknown) => {
				log.push(filters);
				return result();
			},
		} as unknown as Mocked;
	}

	test("while auth is unknown: skeleton, no registry request", async () => {
		const log: unknown[] = [];
		await renderMine(
			registry(log, async () => ({ packages: PACKAGES })),
			{ signedIn: undefined, authLoading: true },
		);
		expect(log).toEqual([]);
		expect(host.querySelector("h1")?.textContent).toBe("Packages");
		expect(host.querySelector("[data-nav]")).not.toBe(null);
		expect(host.querySelector("[data-registry-mine-sign-in]")).toBe(null);
		expect(
			host.querySelector("#registry-mine-results")?.getAttribute("aria-busy"),
		).toBe("true");
	});

	test("a signed-out push while OIDC is still loading keeps the skeleton", async () => {
		const log: unknown[] = [];
		await renderMine(
			registry(log, async () => ({ packages: PACKAGES })),
			{ signedIn: false, authLoading: true },
		);
		expect(log).toEqual([]);
		expect(host.querySelector("[data-registry-mine-sign-in]")).toBe(null);
	});

	test("signed out: sign-in call to action, not the empty state", async () => {
		const log: unknown[] = [];
		await renderMine(
			registry(log, async () => ({ packages: [] })),
			{ signedIn: false },
		);
		expect(log).toEqual([]);
		expect(host.querySelector("[data-registry-mine-sign-in]")).not.toBe(null);
		expect(host.querySelector("[data-registry-mine-empty]")).toBe(null);
	});

	const cardIds = () =>
		Array.from(host.querySelectorAll("[data-registry-mine-package]")).map(
			(card) => card.getAttribute("data-registry-mine-package"),
		);

	test("signed in: maintained packages incl. disabled and deprecated, with state and workspace links", async () => {
		const log: unknown[] = [];
		const client = await renderMine(
			registry(log, async () => ({ packages: PACKAGES })),
			{ signedIn: true },
		);
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-package]")),
			"the Mine cards",
		);
		expect(log).toEqual([
			{
				access: "maintainer",
				includeDisabled: true,
				includeDeprecated: true,
				limit: 100,
			},
		]);
		expect(
			client.getQueryCache().find({
				queryKey: ["mine-registry-maintained", "user-1"],
				exact: true,
			}),
		).toBeDefined();
		expect(cardIds()).toEqual([
			"off-pkg",
			"review-pkg",
			"live-pkg",
			"team-pkg",
			"old-pkg",
		]);
		const card = (id: string) =>
			host.querySelector(`[data-registry-mine-package="${id}"]`);
		expect(card("live-pkg")?.textContent).toContain("Live");
		expect(card("live-pkg")?.textContent).toContain("You own this package");
		expect(card("team-pkg")?.textContent).toContain(
			"You maintain this package",
		);
		expect(card("review-pkg")?.textContent).toContain("In review");
		expect(card("review-pkg")?.textContent).toContain("Waiting for review");
		expect(card("off-pkg")?.textContent).toContain("Disabled");
		expect(card("old-pkg")?.textContent).toContain("Deprecated");
		expect(card("old-pkg")?.textContent).toContain(
			"no longer offered to new users",
		);

		const manage = (id: string) =>
			Array.from(card(id)?.querySelectorAll("a") ?? [])
				.find((link) => link.textContent?.includes("Manage"))
				?.getAttribute("href");
		expect(manage("live-pkg")).toBe("/store/package-workspace?id=live-pkg");
		expect(manage("off-pkg")).toBe(
			"/store/package-workspace?id=off-pkg&tab=releases",
		);
		expect(card("live-pkg")?.querySelector("h3 a")?.getAttribute("href")).toBe(
			"/store/package-workspace?id=live-pkg",
		);
		const stretched = card("off-pkg")?.querySelector(
			"[data-registry-mine-card-link]",
		);
		expect(stretched?.getAttribute("href")).toBe(
			"/store/package-workspace?id=off-pkg",
		);
		expect(stretched?.getAttribute("tabindex")).toBe("-1");
		expect(stretched?.getAttribute("aria-hidden")).toBe("true");

		const chips = host.querySelector('fieldset[aria-label="Filter by state"]');
		expect(chips?.textContent).toContain("All5");
		expect(chips?.textContent).toContain("Live2");
		expect(chips?.textContent).toContain("Disabled1");
		expect(chips?.textContent).toContain("Deprecated1");
	});

	test("the ⋯ menu links the store page with tab=mine", async () => {
		const { act } = await import("react");
		await renderMine(
			registry([], async () => ({ packages: PACKAGES.slice(0, 1) })),
			{ signedIn: true },
		);
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-package]")),
			"the Mine card",
		);
		const more = host.querySelector<HTMLButtonElement>(
			'button[aria-label="More actions for live-pkg"]',
		);
		expect(more).not.toBe(null);
		await act(async () => {
			more?.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					button: 0,
					pointerType: "mouse",
				}),
			);
		});
		await until(
			() =>
				host.ownerDocument.body.querySelectorAll('[role="menuitem"]').length >
				0,
			"the menu",
		);
		const hrefs = Array.from(
			host.ownerDocument.body.querySelectorAll('[role="menuitem"]'),
		).map((item) => item.getAttribute("href"));
		expect(hrefs).toEqual([
			"/store/packages?id=live-pkg&tab=mine",
			"/store/package-workspace?id=live-pkg&tab=access",
			"/store/package-workspace?id=live-pkg&tab=releases",
		]);
	});

	test("a registry failure shows an error with Retry that refetches", async () => {
		const log: unknown[] = [];
		let fail = true;
		await renderMine(
			registry(log, async () => {
				if (fail) throw new Error("500");
				return { packages: PACKAGES };
			}),
			{ signedIn: true },
		);
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-error]")),
			"the registry error",
		);
		expect(host.querySelector("[data-registry-mine-empty]")).toBe(null);
		fail = false;
		const retry = Array.from(
			host.querySelectorAll<HTMLButtonElement>(
				"[data-registry-mine-error] button",
			),
		).find((button) => button.textContent?.includes("Retry"));
		const { act } = await import("react");
		await act(async () => retry?.click());
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-package]")),
			"the cards after Retry",
		);
		expect(log).toHaveLength(2);
		expect(host.querySelector("[data-registry-mine-error]")).toBe(null);
	});

	test("a failed refresh keeps the loaded list and shows Retry above it", async () => {
		const log: unknown[] = [];
		let fail = false;
		await renderMine(
			registry(log, async () => {
				if (fail) throw new Error("503");
				return { packages: PACKAGES };
			}),
			{ signedIn: true },
		);
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-package]")),
			"the Mine cards",
		);
		fail = true;
		const refresh = Array.from(
			host.querySelectorAll<HTMLButtonElement>("[data-packages-header] button"),
		).find((button) => button.textContent?.includes("Refresh"));
		expect(refresh).toBeDefined();
		const { act } = await import("react");
		await act(async () => refresh?.click());
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-error]")),
			"the refresh error",
		);
		expect(log).toHaveLength(2);
		expect(cardIds()).toHaveLength(PACKAGES.length);
		expect(host.querySelector("[data-packages-toolbar] input")).not.toBe(null);
		expect(
			host.querySelector('fieldset[aria-label="Filter by state"]'),
		).not.toBe(null);
	});

	test("no packages: points to the desktop app for local checkouts", async () => {
		await renderMine(
			registry([], async () => ({ packages: [] })),
			{ signedIn: true },
		);
		await until(
			() => Boolean(host.querySelector("[data-registry-mine-empty]")),
			"the empty state",
		);
		expect(host.textContent).toContain("desktop app");
		expect(
			host
				.querySelector('[data-registry-mine-empty] a[target="_blank"]')
				?.getAttribute("href"),
		).toBe("https://flow-like.com/download");
	});
});
