import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import type { Challenge, LessonAppRef } from "../../lib/learn/types";
import type { IBlockedPackage } from "../../lib/schema/app/fork";
import type { LessonAppCopy } from "./lesson-app-packages";

/** The first dynamic import transpiles the component graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 60_000;

const PAID: IBlockedPackage = {
	package_id: "chart-kit",
	name: "Chart Kit",
	block: "paid",
	price: 499,
	request_pending: false,
};
const ON_REQUEST: IBlockedPackage = {
	package_id: "team-utils",
	name: "Team Utils",
	block: "request_access",
	price: 0,
	request_pending: false,
};
const PRIVATE: IBlockedPackage = {
	package_id: "internal-nodes",
	name: "internal-nodes",
	block: "private",
	price: 0,
	request_pending: false,
};

const appRef = (alias: string | null): LessonAppRef =>
	({ id: `ref-${alias}`, app_alias: alias }) as LessonAppRef;

const boardChallenge = (payload: Record<string, unknown>): Challenge =>
	({ id: "challenge", kind: "EXECUTE_NODE", payload }) as Challenge;

let window: Window;
let root: Root;
let host: HTMLElement;
let client: QueryClient;
/** Query keys the component invalidated since the backend was set up. */
let invalidated: unknown[] = [];
let restoreGlobals: () => void;
let restoreBackend: (() => void) | undefined;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/learn/lesson" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Node: window.Node,
		MutationObserver: window.MutationObserver,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		MouseEvent: window.MouseEvent,
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		getComputedStyle: window.getComputedStyle.bind(window),
		matchMedia: window.matchMedia.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
		fetch: globalThis.fetch,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	// A browser checkout opens its page with window.open; happy-dom would load it.
	Object.assign(window, { SyntaxError, TypeError, open: () => null });
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};

	const { createRoot } = await import("react-dom/client");
	host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	root = createRoot(host);
});

afterEach(async () => {
	const { act } = await import("react");
	await act(() => root.unmount());
	restoreBackend?.();
	restoreBackend = undefined;
	await window.happyDOM.abort();
	restoreGlobals();
});

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

interface Backend {
	requestAccess?: () => Promise<unknown>;
	purchasePackage?: () => Promise<unknown>;
}

interface Lesson {
	aliases?: (string | null)[];
	challenges?: Challenge[];
	linkedAppIds: Record<string, string>;
	syncCopyPackages: (alias: string) => Promise<LessonAppCopy>;
	/** The workspace layout. A switch rebuilds the lesson body and the notice in it. */
	layout?: "split" | "stacked";
}

/** Stands in for the lesson page, which outlives the notice and keeps its checkouts. */
let LessonPage: ((lesson: Lesson) => ReactNode) | undefined;

/** Installs the hub, the backend and the query client a test renders against. */
async function prepare(backend: Backend = {}) {
	const { QueryClient } = await import("@tanstack/react-query");
	const { useBackendStore } = await import("../../state/backend-state");

	// The hub sells through the browser checkout, not the marketplace dialog.
	globalThis.fetch = (async () =>
		new Response(JSON.stringify({ payments: { marketplace_enabled: false } }), {
			status: 200,
			headers: { "content-type": "application/json" },
		})) as unknown as typeof fetch;
	const previous = useBackendStore.getState().backend;
	restoreBackend = () => useBackendStore.setState({ backend: previous });
	useBackendStore.getState().setBackend({
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
			getProfile: async () => ({ id: "profile", hub: "hub.test" }),
		},
		apiState: { get: async () => null, post: async () => null },
		registryState: backend,
	} as never);
	// The app retries a failed query once by default; without a delay a retry
	// lands inside a test's flush.
	client = new QueryClient({
		defaultOptions: { queries: { retry: 1, retryDelay: 0 } },
	});
	invalidated = [];
	const invalidate = client.invalidateQueries.bind(client);
	client.invalidateQueries = ((filters: { queryKey?: unknown }) => {
		invalidated.push(filters.queryKey);
		return invalidate(filters as never);
	}) as typeof client.invalidateQueries;
}

/** Shows one lesson on the page, which stays mounted from lesson to lesson. */
async function show(lesson: Lesson) {
	const { act } = await import("react");
	const { QueryClientProvider } = await import("@tanstack/react-query");
	const { AuthContext } = await import("react-oidc-context");
	const { LessonAppPackages, useLessonPackageCheckouts } = await import(
		"./lesson-app-packages"
	);
	LessonPage ??= function Page(shown: Lesson) {
		const checkouts = useLessonPackageCheckouts();
		return (
			<LessonAppPackages
				key={shown.layout ?? "split"}
				courseId="course"
				profileId="profile"
				appRefs={(shown.aliases ?? []).map(appRef)}
				challenges={shown.challenges ?? []}
				linkedAppIds={shown.linkedAppIds}
				syncCopyPackages={shown.syncCopyPackages}
				checkouts={checkouts}
			/>
		);
	};
	const Page = LessonPage;
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<AuthContext.Provider
					value={{ isAuthenticated: true, user: null } as never}
				>
					<Page {...lesson} />
				</AuthContext.Provider>
			</QueryClientProvider>,
		),
	);
	await flush(40);
}

async function render(lesson: Lesson, backend?: Backend) {
	await prepare(backend);
	await show(lesson);
}

function button(label: string) {
	return Array.from(window.document.body.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.includes(label),
	);
}

/**
 * Clicks a button once it can be clicked. Buy unlocks only after the rows
 * loaded the profile and then the hub's payment config, which takes a moment
 * of its own after the rows appear.
 */
async function click(label: string) {
	const { act } = await import("react");
	for (let waited = 0; waited < 50; waited++) {
		if (button(label)?.hasAttribute("disabled") === false) break;
		await flush();
	}
	const target = button(label);
	if (!target) throw new Error(`No "${label}" button`);
	if (target.hasAttribute("disabled"))
		throw new Error(`The "${label}" button is disabled`);
	await act(async () => {
		target.dispatchEvent(
			new window.MouseEvent("click", { bubbles: true }) as never,
		);
	});
	await flush();
}

describe("LessonAppPackages", () => {
	test(
		"syncs only the aliases the learner already has an app for",
		async () => {
			const synced: string[] = [];
			await render({
				aliases: ["crm", "billing", null],
				linkedAppIds: { crm: "crm-copy" },
				syncCopyPackages: async (alias) => {
					synced.push(alias);
					return { app_id: "crm-copy", blocked_packages: [ON_REQUEST] };
				},
			});

			expect(synced).toEqual(["crm"]);
			const text = host.textContent ?? "";
			expect(text).toContain("1 package is missing from your copy");
			expect(text).toContain("Request access to add it to your copy.");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test("syncs the copy a board challenge works in", async () => {
		const synced: string[] = [];
		await render({
			challenges: [
				boardChallenge({ appAlias: "starter" }),
				boardChallenge({ app_alias: "crm" }),
				boardChallenge({ boardId: "board" }),
			],
			linkedAppIds: { starter: "starter-copy", crm: "crm-copy" },
			syncCopyPackages: async (alias) => {
				synced.push(alias);
				return { app_id: `${alias}-copy`, blocked_packages: [PRIVATE] };
			},
		});

		expect(synced.sort()).toEqual(["crm", "starter"]);
		expect(host.textContent).toContain("1 package is missing from your copy");
	});

	test("shows nothing while every copy has its template's packages", async () => {
		await render({
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => ({ app_id: "crm-copy" }),
		});

		expect(host.textContent).toBe("");
	});

	test("lists a package once when several copies lack it", async () => {
		const synced: string[] = [];
		await render({
			aliases: ["crm", "billing", "crm"],
			linkedAppIds: { crm: "crm-copy", billing: "billing-copy" },
			syncCopyPackages: async (alias) => {
				synced.push(alias);
				return {
					app_id: `${alias}-copy`,
					blocked_packages:
						alias === "crm" ? [ON_REQUEST] : [ON_REQUEST, PRIVATE],
				};
			},
		});

		expect(synced.sort()).toEqual(["billing", "crm"]);
		expect(host.textContent).toContain("2 packages are missing from your copy");
		expect(host.querySelectorAll("li").length).toBe(2);
	});

	test("granted access syncs the copy again, which then has the package", async () => {
		let granted = false;
		let syncs = 0;
		await render(
			{
				aliases: ["crm"],
				linkedAppIds: { crm: "crm-copy" },
				syncCopyPackages: async () => {
					syncs += 1;
					return granted
						? { app_id: "crm-copy", added_packages: ["team-utils"] }
						: { app_id: "crm-copy", blocked_packages: [ON_REQUEST] };
				},
			},
			{
				requestAccess: async () => {
					granted = true;
					return { granted: true, queued: false, requiresPurchase: false };
				},
			},
		);
		expect(invalidated).toEqual([]);

		await click("Request access");
		await flush(40);

		expect(syncs).toBe(2);
		expect(host.textContent).toBe("");
		expect(invalidated).toContainEqual(["app", "crm-copy", "packages"]);
		expect(invalidated).toContainEqual(["app-catalog-nodes", "crm-copy"]);
	});

	test("a sync that adds nothing leaves the copy's package queries alone", async () => {
		await render({
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => ({
				app_id: "crm-copy",
				blocked_packages: [PRIVATE],
				added_packages: [],
			}),
		});

		expect(host.textContent).toContain("This package is private");
		expect(invalidated).toEqual([]);
	});

	test("a sync that added a package reloads that copy's packages", async () => {
		await render({
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => ({
				app_id: "crm-copy",
				blocked_packages: [],
				added_packages: ["team-utils"],
			}),
		});

		expect(host.textContent).toBe("");
		expect(invalidated).toContainEqual(["app", "crm-copy", "packages"]);
		expect(invalidated).toContainEqual(["getCatalog", "crm-copy"]);
	});

	test("a request that was sent stays requested when the next lesson shows the rows again", async () => {
		let requested = false;
		const lesson: Lesson = {
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => ({
				app_id: "crm-copy",
				blocked_packages: [{ ...ON_REQUEST, request_pending: requested }],
			}),
		};
		await render(lesson, {
			requestAccess: async () => {
				requested = true;
				return { granted: false, queued: true, requiresPurchase: false };
			},
		});
		await click("Request access");
		await flush(40);
		expect(host.textContent).toContain("Requested");

		// The next lesson is still loading: it names no app yet, and the
		// workspace falls back to the stacked layout until it does.
		await show({ ...lesson, aliases: [], layout: "stacked" });
		expect(host.textContent).toBe("");
		await show(lesson);

		expect(button("Request access")).toBeUndefined();
		expect(host.textContent).toContain("Requested");
	});

	test("a checkout that is still open is followed when the next lesson shows the rows again", async () => {
		const lesson: Lesson = {
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => ({
				app_id: "crm-copy",
				blocked_packages: [PAID],
			}),
		};
		await render(lesson, {
			purchasePackage: async () => ({
				alreadyHasAccess: false,
				checkoutUrl: "https://pay.example/session",
				packageId: "chart-kit",
			}),
		});
		await click("Buy");
		expect(button("Waiting for payment")).toBeDefined();

		await show({ ...lesson, aliases: [], layout: "stacked" });
		expect(host.textContent).toBe("");
		await show(lesson);

		expect(button("Waiting for payment")).toBeDefined();
		expect(button("Buy")).toBeUndefined();
	});

	test("a sync the hub refuses is not repeated", async () => {
		const { ApiResponseError } = await import("../../lib/api-error");
		let syncs = 0;
		await render({
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => {
				syncs += 1;
				throw new ApiResponseError({ status: 404, message: "Not Found" });
			},
		});
		await flush(60);

		expect(syncs).toBe(1);
		expect(host.textContent).toBe("");
	});

	test("a sync the hub never answered is repeated", async () => {
		let syncs = 0;
		await render({
			aliases: ["crm"],
			linkedAppIds: { crm: "crm-copy" },
			syncCopyPackages: async () => {
				syncs += 1;
				if (syncs === 1) throw new Error("Network unavailable: /courses");
				return { app_id: "crm-copy", blocked_packages: [PRIVATE] };
			},
		});
		await flush(60);

		expect(syncs).toBe(2);
		expect(host.textContent).toContain("This package is private");
	});
});
