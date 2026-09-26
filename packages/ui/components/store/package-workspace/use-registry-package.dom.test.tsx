import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import { Window } from "happy-dom";
import type { Root } from "react-dom/client";
import { PackagePermissionBits } from "../../../lib/permission/wasm-package-permission";
import type { QueryStorageBackend } from "../../../lib/query-persister";
import { PackageStatus, type RegistryEntry } from "../../../lib/schema/wasm";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import type {
	RegistryPackageAuth,
	RegistryPackageResult,
} from "./use-registry-package";

const COLD_IMPORT_TIMEOUT_MS = 30_000;
const PACKAGE_ID = "simple-math";
const HUB_PROFILE = { id: "hub", hub: "https://api.example.test" };

let window: Window;
let root: Root;
let restoreGlobals: () => void;

beforeEach(async () => {
	window = new Window({ url: "https://app.flow-like.com/store" });
	const globals = {
		window,
		document: window.document,
		navigator: window.navigator,
		HTMLElement: window.HTMLElement,
		Element: window.Element,
		Node: window.Node,
		Event: window.Event,
		MutationObserver: window.MutationObserver,
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	restoreGlobals = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	const { createRoot } = await import("react-dom/client");
	const host = window.document.createElement("div");
	window.document.body.appendChild(host);
	root = createRoot(host as unknown as HTMLElement);
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
		versions: [],
		status: PackageStatus.Active,
		downloadCount: 0,
		createdAt: "",
		updatedAt: "",
		source: { type: "remote" },
		verified: false,
		price: 0,
		visibility: "public",
		currentUserPermission: permission,
	};
}

interface FetchCall {
	path: string;
	token?: string;
}

function recordingFetcher(answer: () => Promise<unknown>) {
	const calls: FetchCall[] = [];
	const fetcher = ((_profile, path, _options, auth?: RegistryPackageAuth) => {
		calls.push({ path, token: auth?.user?.access_token });
		return answer();
	}) as GenericFetcher;
	return { calls, fetcher };
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

function memoryBackend(): QueryStorageBackend {
	const store = new Map<string, string>();
	return {
		get: async (key) => store.get(key),
		set: async (key, value) => {
			store.set(key, value);
		},
		del: async (key) => {
			store.delete(key);
		},
		entries: async () => [...store.entries()],
	};
}

async function flush(ms = 20) {
	const { act } = await import("react");
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, ms));
	});
}

interface MountOptions {
	signedIn: boolean | undefined;
	auth: RegistryPackageAuth;
	fetcher: GenericFetcher;
	profile?: () => Promise<unknown>;
	client?: QueryClient;
}

async function mount(options: MountOptions) {
	const { act } = await import("react");
	const { QueryClient, QueryClientProvider } = await import(
		"@tanstack/react-query"
	);
	const { useAuthStatusStore, useBackendStore } = await import(
		"../../../state/backend-state"
	);
	const { useRegistryPackage } = await import("./use-registry-package");

	useBackendStore.getState().setBackend({
		userState: {
			getSettingsProfile:
				options.profile ?? (async () => ({ hub_profile: HUB_PROFILE })),
		},
		registryState: { getPackage: async () => null },
	} as never);
	useAuthStatusStore.setState({ signedIn: options.signedIn });

	const client =
		options.client ??
		new QueryClient({ defaultOptions: { queries: { retry: false } } });
	const results: RegistryPackageResult[] = [];
	function Probe({ auth }: { auth: RegistryPackageAuth }) {
		results.push(
			useRegistryPackage(PACKAGE_ID, options.fetcher, auth, {
				localFallback: false,
			}),
		);
		return null;
	}
	const draw = async (auth: RegistryPackageAuth) => {
		await act(async () =>
			root.render(
				<QueryClientProvider client={client}>
					<Probe auth={auth} />
				</QueryClientProvider>,
			),
		);
		await flush();
	};
	await draw(options.auth);
	return {
		client,
		results,
		latest: () => results[results.length - 1],
		rerender: draw,
	};
}

const SIGNED_IN_USER = {
	access_token: "fresh-token",
	expired: false,
	profile: { sub: "maintainer-1" },
};
const EXPIRED_USER = {
	access_token: "expired-token",
	expired: true,
	profile: { sub: "maintainer-1" },
};

describe("useRegistryPackage", () => {
	test(
		"a restored expired user the host reports signed out is expired and fetches nothing, not even anonymously",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () => registryEntry());
			const { latest } = await mount({
				signedIn: false,
				auth: { isLoading: false, user: EXPIRED_USER },
				fetcher,
			});
			expect(latest().authState).toBe("expired");
			expect(latest().status).toBe("idle");
			expect(latest().isLoading).toBe(false);
			expect(calls).toEqual([]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a restored expired user being renewed stays unknown",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () => registryEntry());
			const { latest } = await mount({
				signedIn: false,
				auth: {
					isLoading: true,
					activeNavigator: "signinSilent",
					user: EXPIRED_USER,
				},
				fetcher,
			});
			expect(latest().authState).toBe("unknown");
			expect(calls).toEqual([]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a pushed session whose token has expired is not sent",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () => registryEntry());
			const { latest } = await mount({
				signedIn: true,
				auth: { isLoading: false, user: EXPIRED_USER },
				fetcher,
			});
			expect(latest().authState).toBe("expired");
			expect(calls).toEqual([]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a token that expires after a load keeps the confirmed entry and sends nothing more",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Owner),
			);
			const { latest, rerender, results } = await mount({
				signedIn: true,
				auth: { isLoading: false, user: SIGNED_IN_USER },
				fetcher,
			});
			expect(latest().status).toBe("ok");
			const loaded = results.length;

			await rerender({
				isLoading: false,
				user: { ...SIGNED_IN_USER, expired: true },
			});
			await rerender({
				isLoading: true,
				activeNavigator: "signinSilent",
				user: { ...SIGNED_IN_USER, expired: true },
			});

			const after = results.slice(loaded);
			expect(after.map((result) => result.authState)).toContain("expired");
			expect(after.map((result) => result.authState)).toContain("unknown");
			for (const result of after) {
				expect(result.status).toBe("ok");
				expect(result.source).toBe("registry");
				expect(result.entry?.currentUserPermission).toBe(
					PackagePermissionBits.Owner,
				);
			}
			expect(calls).toHaveLength(1);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a failed background refetch keeps the confirmed entry; 403 still takes it away",
		async () => {
			let answer: () => Promise<unknown> = async () =>
				registryEntry(PackagePermissionBits.Maintainer);
			const { calls, fetcher } = recordingFetcher(() => answer());
			const { client, latest } = await mount({
				signedIn: true,
				auth: { isLoading: false, user: SIGNED_IN_USER },
				fetcher,
			});
			expect(latest().status).toBe("ok");
			const { act } = await import("react");

			answer = async () => {
				throw Object.assign(new Error("Bad Gateway"), { status: 502 });
			};
			await act(async () => {
				await client.invalidateQueries({
					queryKey: ["registry-package", PACKAGE_ID],
				});
			});
			await flush();
			expect(calls).toHaveLength(2);
			expect(latest().status).toBe("ok");
			expect(latest().entry?.currentUserPermission).toBe(
				PackagePermissionBits.Maintainer,
			);

			answer = async () => {
				throw Object.assign(new Error("Forbidden"), { status: 403 });
			};
			await act(async () => {
				await client.invalidateQueries({
					queryKey: ["registry-package", PACKAGE_ID],
				});
			});
			await flush();
			expect(calls).toHaveLength(3);
			expect(latest().status).toBe("forbidden");
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"a confirmed session sends its token and caches under its user",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Owner),
			);
			const client = new (await import("@tanstack/react-query")).QueryClient({
				defaultOptions: { queries: { retry: false } },
			});
			const { latest } = await mount({
				signedIn: true,
				auth: { isLoading: false, user: SIGNED_IN_USER },
				fetcher,
				client,
			});
			expect(latest().authState).toBe("signed_in");
			expect(latest().status).toBe("ok");
			expect(calls).toEqual([
				{ path: `registry/package/${PACKAGE_ID}`, token: "fresh-token" },
			]);
			expect(
				client.getQueryData(["registry-package", PACKAGE_ID, "maintainer-1"]),
			).toBeDefined();
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"signed out: an anonymous profile failure is reported as signed_out, not only as an error",
		async () => {
			const { calls, fetcher } = recordingFetcher(async () => registryEntry());
			const { latest } = await mount({
				signedIn: false,
				auth: { isLoading: false, user: null },
				fetcher,
				profile: async () => {
					throw Object.assign(new Error("Unauthorized"), { status: 401 });
				},
			});
			expect(latest().authState).toBe("signed_out");
			expect(calls).toEqual([]);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);

	test(
		"an entry restored from a previous session stays loading until the server confirms it",
		async () => {
			const { QueryClient } = await import("@tanstack/react-query");
			const { createSmartQueryPersister } = await import(
				"../../../lib/query-persister"
			);
			const persister = createSmartQueryPersister({
				backend: memoryBackend(),
			});
			const clientFor = () =>
				new QueryClient({
					defaultOptions: {
						queries: {
							retry: false,
							staleTime: 30_000,
							persister: persister.persisterFn,
						},
					},
				});
			const auth = { isLoading: false, user: SIGNED_IN_USER };

			const first = recordingFetcher(async () =>
				registryEntry(PackagePermissionBits.Maintainer),
			);
			await mount({
				signedIn: true,
				auth,
				fetcher: first.fetcher,
				client: clientFor(),
			});
			expect(first.calls).toHaveLength(1);
			const { act } = await import("react");
			await act(() => root.render(null));
			await flush();

			const revoked = deferred<unknown>();
			const second = recordingFetcher(() => revoked.promise);
			const { results, latest } = await mount({
				signedIn: true,
				auth,
				fetcher: second.fetcher,
				client: clientFor(),
			});

			expect(latest().entry?.currentUserPermission).toBe(
				PackagePermissionBits.Maintainer,
			);
			expect(latest().status).toBe("loading");
			expect(results.some((result) => result.status === "ok")).toBe(false);
			expect(second.calls).toHaveLength(1);

			await act(async () => {
				revoked.reject(Object.assign(new Error("Forbidden"), { status: 403 }));
			});
			await flush();
			expect(latest().status).toBe("forbidden");
			expect(results.some((result) => result.status === "ok")).toBe(false);
		},
		COLD_IMPORT_TIMEOUT_MS,
	);
});
