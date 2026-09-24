import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { ReactNode } from "react";
import type { Root } from "react-dom/client";
import { PackagePermissionBits } from "../../../lib/permission/wasm-package-permission";
import {
	type PackageMeta,
	PackageStatus,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import type { RegistryPackageAuth } from "./use-registry-package";

/** The first dynamic import transpiles the workspace graph, which can take seconds on a cold cache. */
const COLD_IMPORT_TIMEOUT_MS = 60_000;
const PACKAGE_ID = "simple-math";
const ENTRY_PATH = `registry/package/${PACKAGE_ID}`;
const STORE_HREF = `/store/packages?id=${PACKAGE_ID}`;
const OWNER_ONLY = /\/(users|access|meta|publication-reviews|price)\b/;

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
	usePathname: () => "/store/package-workspace",
}));
afterAll(() => {
	mock.module("next/navigation", () => actualNavigation);
});

let window: Window;
let root: Root;
let host: HTMLElement;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({
		url: "https://app.flow-like.com/store/package-workspace",
	});
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		localStorage: window.localStorage,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLAnchorElement: window.HTMLAnchorElement,
		HTMLImageElement: window.HTMLImageElement,
		Element: window.Element,
		Text: window.Text,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		ShadowRoot: window.ShadowRoot,
		DOMRect: window.DOMRect,
		Range: window.Range,
		Selection: window.Selection,
		HTMLCollection: window.HTMLCollection,
		NodeList: window.NodeList,
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

function registryEntry(permission?: number): RegistryEntry {
	return {
		id: PACKAGE_ID,
		manifest: {
			manifestVersion: 1,
			id: PACKAGE_ID,
			name: "Simple Math",
			version: "0.3.0",
			description: "",
			authors: [],
			keywords: [],
			primaryCategory: "DATA_TRANSFORMATION",
			permissions: {} as RegistryEntry["manifest"]["permissions"],
			metadata: {},
		},
		nodes: [],
		versions: [
			{
				version: "0.3.0",
				wasmHash: "",
				wasmSize: 0,
				publishedAt: "2026-09-01T00:00:00Z",
				yanked: false,
				status: PackageStatus.Active,
			},
		],
		status: PackageStatus.Active,
		downloadCount: 0,
		createdAt: "2026-09-01T00:00:00Z",
		updatedAt: "2026-09-01T00:00:00Z",
		source: { type: "remote" },
		verified: false,
		price: 0,
		visibility: "public",
		currentUserPermission: permission,
	};
}

const EMPTY_META: PackageMeta = {
	id: PACKAGE_ID,
	lang: "en",
	name: "Simple Math",
	tags: [],
};

interface FetchCall {
	path: string;
	token?: string;
}

function recordingFetcher(entry: () => Promise<unknown>) {
	const calls: FetchCall[] = [];
	const fetcher = (async (_profile, path, _options, auth?: unknown) => {
		calls.push({
			path,
			token: (auth as RegistryPackageAuth)?.user?.access_token,
		});
		if (path === ENTRY_PATH) return entry();
		if (path.endsWith("/meta")) return EMPTY_META;
		return [];
	}) as GenericFetcher;
	return { calls, fetcher };
}

const SIGNED_IN = {
	isLoading: false,
	user: {
		access_token: "fresh-token",
		expired: false,
		profile: { sub: "user-1" },
	},
	signinRedirect: async () => {},
};
const SIGNED_OUT = { isLoading: false, user: null };

function expiredCopy(auth: typeof SIGNED_IN) {
	return { ...auth, user: { ...auth.user, expired: true } };
}

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

async function until(check: () => boolean, label: string, attempts = 100) {
	for (let attempt = 0; attempt < attempts; attempt += 1) {
		if (check()) return;
		await flush(10);
	}
	throw new Error(
		`Timed out waiting for ${label}: ${host.textContent?.slice(0, 400)}`,
	);
}

async function prepare(
	signedIn: boolean | undefined,
	search = `id=${PACKAGE_ID}`,
) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../../state/backend-state"
	);
	const { PackageWorkspace } = await import("./package-workspace");

	navigation.params = new URLSearchParams(search);
	useBackendStore.getState().setBackend({
		profile: undefined,
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "hub" } }),
		},
		registryState: { getPackage: async () => null },
	} as never);
	useAuthStatusStore.setState({ signedIn });
	const client: QueryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const render = async (node: ReactNode) => {
		await act(async () =>
			root.render(
				<QueryClientProvider client={client}>{node}</QueryClientProvider>,
			),
		);
		await flush();
	};
	return { client, render, PackageWorkspace };
}

async function mount({
	signedIn,
	auth,
	fetcher,
	search,
}: {
	signedIn: boolean | undefined;
	auth: RegistryPackageAuth;
	fetcher: GenericFetcher;
	search?: string;
}) {
	const { client, render, PackageWorkspace } = await prepare(signedIn, search);
	const draw = (nextAuth: RegistryPackageAuth) =>
		render(
			<PackageWorkspace
				packageId={PACKAGE_ID}
				fetcher={fetcher}
				auth={nextAuth}
			/>,
		);
	await draw(auth);
	return { client, rerender: draw };
}

/** Radix trigger ids end in `-trigger-<value>`. */
function tabs(): string[] {
	return Array.from(host.querySelectorAll('[role="tab"]')).map(
		(tab) => tab.id.split("-trigger-")[1] ?? "",
	);
}

function button(label: string): HTMLButtonElement | undefined {
	return Array.from(host.querySelectorAll("button")).find(
		(candidate) => candidate.textContent?.trim() === label,
	) as HTMLButtonElement | undefined;
}

function ownerOnlyCalls(calls: FetchCall[]): FetchCall[] {
	return calls.filter((call) => OWNER_ONLY.test(call.path));
}

describe("PackageWorkspace gate", () => {
	test(
		"a signed-in stranger goes to the store page after one entry request, and no owner request fires",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(undefined),
			);
			await mount({ signedIn: true, auth: SIGNED_IN, fetcher });
			await until(
				() => navigation.replace.mock.calls.length > 0,
				"the redirect",
			);

			expect(navigation.replace.mock.calls).toEqual([[STORE_HREF]]);
			expect(calls).toEqual([{ path: ENTRY_PATH, token: "fresh-token" }]);
			expect(host.querySelector('[role="tablist"]')).toBeNull();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a signed-out visitor goes to the store page without any owner request",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(undefined),
			);
			await mount({ signedIn: false, auth: SIGNED_OUT, fetcher });
			await until(
				() => navigation.replace.mock.calls.length > 0,
				"the redirect",
			);

			expect(navigation.replace.mock.calls).toEqual([[STORE_HREF]]);
			expect(ownerOnlyCalls(calls)).toEqual([]);
			expect(calls.every((call) => call.token === undefined)).toBe(true);
			expect(host.querySelector('[role="tablist"]')).toBeNull();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a maintainer is never redirected and ?tab=listing opens Listing",
		async () => {
			const { fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Maintainer),
			);
			await mount({
				signedIn: true,
				auth: SIGNED_IN,
				fetcher,
				search: `id=${PACKAGE_ID}&tab=listing`,
			});
			await until(() => tabs().length > 0, "the owner tabs");

			expect(tabs()).toEqual([
				"overview",
				"nodes",
				"listing",
				"access",
				"releases",
			]);
			const active = host.querySelector('[role="tab"][aria-selected="true"]');
			expect(active?.id.endsWith("-trigger-listing")).toBe(true);
			await until(
				() => !!host.querySelector('[aria-label="Store preview"]'),
				"the Listing panel",
			);
			expect(navigation.replace).not.toHaveBeenCalled();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a registry failure offers Retry and never redirects",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () => {
				throw Object.assign(new Error("Internal Server Error"), {
					status: 500,
				});
			});
			await mount({ signedIn: true, auth: SIGNED_IN, fetcher });
			await until(() => !!button("Retry"), "the Retry action");

			expect(navigation.replace).not.toHaveBeenCalled();
			expect(ownerOnlyCalls(calls)).toEqual([]);
			expect(host.querySelector('[role="tablist"]')).toBeNull();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});

describe("PackageWorkspace across an expired session", () => {
	test(
		"the owner shell stays mounted when the token expires after a load, and the expired token is never sent",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Owner),
			);
			const { rerender } = await mount({
				signedIn: true,
				auth: SIGNED_IN,
				fetcher,
			});
			await until(() => tabs().length > 0, "the owner tabs");
			const shell = host.querySelector('[role="tablist"]');
			const entryCalls = () =>
				calls.filter((call) => call.path === ENTRY_PATH).length;
			const before = entryCalls();

			await rerender(expiredCopy(SIGNED_IN));
			await flush(50);

			expect(host.querySelector('[role="tablist"]')).toBe(shell);
			expect(host.textContent).toContain("Your session expired");
			expect(button("Sign in")).toBeDefined();
			expect(entryCalls()).toBe(before);
			expect(calls.some((call) => call.token === undefined)).toBe(false);
			expect(navigation.replace).not.toHaveBeenCalled();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"an expired session with nothing confirmed asks for a sign-in instead of waiting or redirecting",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Owner),
			);
			let signIns = 0;
			await mount({
				signedIn: false,
				auth: {
					...expiredCopy(SIGNED_IN),
					signinRedirect: async () => {
						signIns += 1;
					},
				},
				fetcher,
			});
			await until(() => !!button("Sign in"), "the sign-in action");

			expect(calls).toEqual([]);
			expect(navigation.replace).not.toHaveBeenCalled();
			const { act } = await import("react");
			await act(async () => button("Sign in")?.click());
			expect(signIns).toBe(1);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});

describe("PackageWorkspace while a stored session is renewed", () => {
	test(
		"a restored expired user the host renews right away never flashes the sign-in page",
		async () => {
			const { act, useEffect, useState } = await import("react");
			const { render, PackageWorkspace } = await prepare(false);
			const { fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Owner),
			);

			let restore: (() => void) | undefined;
			const committed: string[] = [];
			/**
			 * Like the hosts' AuthInner: an ancestor that renews an expired stored
			 * user in an effect. It re-renders right after each commit, so it sees
			 * every frame the workspace committed.
			 */
			function RenewingHost() {
				const [auth, setAuth] = useState<RegistryPackageAuth>({
					isLoading: true,
					user: null,
				});
				restore = () =>
					setAuth({ ...expiredCopy(SIGNED_IN), isLoading: false });
				committed.push(host.textContent ?? "");
				useEffect(() => {
					if (auth?.user?.expired && !auth.activeNavigator) {
						setAuth({
							...auth,
							isLoading: true,
							activeNavigator: "signinSilent",
						});
					}
				}, [auth]);
				return (
					<PackageWorkspace
						packageId={PACKAGE_ID}
						fetcher={fetcher}
						auth={auth}
					/>
				);
			}

			await render(<RenewingHost />);
			await act(async () => restore?.());
			await flush();

			expect(committed.length).toBeGreaterThan(2);
			expect(
				committed.some((text) => text.includes("Your session expired")),
			).toBe(false);
			expect(button("Sign in")).toBeUndefined();
			expect(navigation.replace).not.toHaveBeenCalled();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});

describe("Listing after publish", () => {
	test(
		"the fix button targets a field the form renders and moves focus to it",
		async () => {
			const { LISTING_FIELD_ANCHOR } = await import("./listing-tab");
			const { fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Maintainer),
			);
			await mount({
				signedIn: true,
				auth: SIGNED_IN,
				fetcher,
				search: `id=${PACKAGE_ID}&tab=listing&published=1`,
			});
			const banner = () =>
				host.querySelector<HTMLElement>('[aria-label="Publish result"]');
			await until(() => !!banner(), "the publish banner");
			const fix = () =>
				Array.from(banner()?.querySelectorAll("button") ?? []).find(
					(candidate) => candidate.getAttribute("aria-label") !== "Dismiss",
				);
			await until(() => !!fix(), "a fix button");

			const { act } = await import("react");
			await act(async () => fix()?.click());
			const anchors = new Set(Object.values(LISTING_FIELD_ANCHOR));
			expect(anchors.has(window.document.activeElement?.id ?? "")).toBe(true);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});
