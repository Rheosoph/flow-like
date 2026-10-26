// @vitest-environment happy-dom
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { createSmartQueryPersister } from "@flow-like/flow-like-ui/lib/query-persister";
import {
	QueryClient,
	QueryClientProvider,
	QueryObserver,
	useQuery,
} from "@tanstack/react-query";
import { act, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { accountQueryDeps } from "../lib/account-query";
import { reportUnauthorized } from "../lib/auth-session";

const harness = vi.hoisted(() => {
	let state = {} as AuthContextProps;
	const listeners = new Set<() => void>();
	class WebBackend {
		pushAuthContext = vi.fn();
		pushProfile = vi.fn();
		userState = {
			getInfo: vi.fn(async () => ({ id: "user-1" })),
			getProfile: vi.fn(async () => ({
				name: "Saved profile",
				bits: [],
				apps: [{ app_id: "saved-app" }],
			})),
			getSettingsProfile: vi.fn(async () => ({
				hub_profile: await this.userState.getProfile(),
				execution_settings: { gpu_mode: false, max_context_size: 8192 },
			})),
			getAllSettingsProfiles: vi.fn(async () => []),
		};
		appState = {
			getApps: vi.fn(
				async (): Promise<[{ id: string }, { name: string }][]> => [
					[{ id: "saved-app" }, { name: "Saved app" }],
				],
			),
			getMyGroups: vi.fn(async () => []),
		};
	}
	return {
		WebBackend,
		backend: new WebBackend(),
		path: "/apps/app-1",
		childMounts: 0,
		showLibrary: false,
		showCallbackReadiness: false,
		useRealOidc: false,
		getUser: vi.fn(),
		signinCallback: vi.fn(),
		loadConfiguration: vi.fn(),
		createUserManager: vi.fn(),
		stopSilentRenew: vi.fn(),
		auth: {
			get: () => state,
			set(next: Partial<AuthContextProps>) {
				state = { ...state, ...next };
				for (const listener of listeners) listener();
			},
			reset(next: AuthContextProps) {
				state = next;
			},
			subscribe(listener: () => void) {
				listeners.add(listener);
				return () => listeners.delete(listener);
			},
		},
	};
});

vi.mock("@flow-like/flow-like-ui", () => ({
	LoadingScreen: () => <output>Loading</output>,
	useBackend: () => harness.backend,
}));
vi.mock("@flow-like/flow-like-ui/components/account/account-session", () => ({
	createAccountTokenProvider: () => ({ getTokens: async () => null }),
}));
vi.mock("aws-amplify", () => ({ Amplify: { configure: vi.fn() } }));
vi.mock("aws-amplify/auth", () => ({ decodeJWT: vi.fn() }));
vi.mock("next/navigation", () => ({ usePathname: () => harness.path }));
vi.mock("oidc-client-ts", () => ({
	UserManager: class {
		settings;
		events = {
			addUserLoaded: vi.fn(),
			removeUserLoaded: vi.fn(),
			addUserUnloaded: vi.fn(),
			removeUserUnloaded: vi.fn(),
			addUserSignedOut: vi.fn(),
			removeUserSignedOut: vi.fn(),
			addAccessTokenExpired: vi.fn(),
			removeAccessTokenExpired: vi.fn(),
			addSilentRenewError: vi.fn(),
			removeSilentRenewError: vi.fn(),
		};
		getUser = harness.getUser;
		signinCallback = harness.signinCallback;
		constructor(settings: unknown) {
			this.settings = settings;
			harness.createUserManager(settings);
		}
		stopSilentRenew = harness.stopSilentRenew;
	},
}));
vi.mock(
	"@flow-like/flow-like-ui/lib/auth/oidc-renewal",
	async (importOriginal) => ({
		...(await importOriginal<
			typeof import("@flow-like/flow-like-ui/lib/auth/oidc-renewal")
		>()),
		coordinateOidcRenewal: vi.fn(),
	}),
);
vi.mock("react-oidc-context", async (importOriginal) => {
	const actual = await importOriginal<typeof import("react-oidc-context")>();
	const { useContext, useSyncExternalStore } = await import("react");
	return {
		AuthProvider: (props: React.ComponentProps<typeof actual.AuthProvider>) =>
			harness.useRealOidc ? <actual.AuthProvider {...props} /> : props.children,
		useAuth: () => {
			const simulated = useSyncExternalStore(
				harness.auth.subscribe,
				harness.auth.get,
			);
			const restored = useContext(actual.AuthContext);
			return harness.useRealOidc ? restored : simulated;
		},
	};
});
vi.mock("../lib/oidc-configuration", () => ({
	loadWebOidcConfiguration: harness.loadConfiguration,
}));
vi.mock("../lib/oidc-settings", () => ({
	getWebOidcSettings: (settings: unknown) => settings,
}));
vi.mock("../lib/public-config", () => ({
	getPublicApiUrl: () => "https://api.test",
	getPublicWebConfig: () => ({}),
}));
vi.mock("./sign-in-required", () => ({
	SignInRequired: () => <div>Sign in required</div>,
}));
vi.mock("./web-provider", () => ({ WebBackend: harness.WebBackend }));

const { WebAuthProvider, useOAuthCallbackReady } = await import(
	"./auth-provider"
);

function ProtectedContent() {
	const callbackReady = useOAuthCallbackReady();
	useEffect(() => {
		harness.childMounts += 1;
	}, []);
	return (
		<div data-testid="protected-content">
			<input aria-label="Workspace draft" defaultValue="" />
			{harness.showCallbackReadiness ? (
				<output>{callbackReady ? "Callback ready" : "Callback waiting"}</output>
			) : harness.showLibrary ? (
				<LibraryProbe />
			) : (
				"App content"
			)}
		</div>
	);
}

function LibraryProbe() {
	const profile = useQuery({
		queryKey: ["getSettingsProfile"],
		queryFn: () => harness.backend.userState.getSettingsProfile(),
	});
	const apps = useQuery({
		queryKey: ["getApps"],
		queryFn: () => harness.backend.appState.getApps(),
	});
	const appIds = new Set(
		profile.data?.hub_profile.apps.map((app) => app.app_id),
	);
	return (
		<output>
			{apps.data
				?.filter(([app]) => appIds.has(app.id))
				.map(([, meta]) => meta.name)
				.join(", ") || "No apps yet"}
		</output>
	);
}

function user(
	accessToken = "token-1",
	{
		subject = "user-1",
		expired = false,
	}: { subject?: string; expired?: boolean | (() => boolean) } = {},
) {
	return {
		access_token: accessToken,
		id_token: "unchanged-id-token",
		profile: { sub: subject },
		get expired() {
			return typeof expired === "function" ? expired() : expired;
		},
	} as NonNullable<AuthContextProps["user"]>;
}

let root: Root;
let container: HTMLDivElement;
let queryClient: QueryClient;

async function renderProvider() {
	await act(async () =>
		root.render(
			<QueryClientProvider client={queryClient}>
				<WebAuthProvider>
					<ProtectedContent />
				</WebAuthProvider>
			</QueryClientProvider>,
		),
	);
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	harness.backend = new harness.WebBackend();
	for (const state of [harness.backend.userState, harness.backend.appState]) {
		for (const [name, fn] of Object.entries(state)) {
			Object.defineProperty(fn, "name", { value: name });
		}
	}
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false, staleTime: 30_000 } },
	});
	harness.path = "/apps/app-1";
	harness.childMounts = 0;
	harness.showLibrary = false;
	harness.showCallbackReadiness = false;
	harness.useRealOidc = false;
	harness.getUser.mockReset().mockResolvedValue(user());
	harness.signinCallback.mockReset().mockResolvedValue(user());
	harness.loadConfiguration.mockReset().mockResolvedValue({
		client_id: "web-client",
		authority: "https://login.test",
	});
	harness.createUserManager.mockReset();
	harness.stopSilentRenew.mockReset();
	window.history.replaceState(null, "", harness.path);
	localStorage.clear();
	sessionStorage.clear();
	harness.auth.reset({
		isLoading: false,
		isAuthenticated: true,
		user: user(),
		events: {
			addAccessTokenExpired: vi.fn(),
			removeAccessTokenExpired: vi.fn(),
			addSilentRenewError: vi.fn(),
			removeSilentRenewError: vi.fn(),
		},
		signinRedirect: vi.fn(async () => undefined),
		removeUser: vi.fn(async () => {
			harness.auth.set({ isAuthenticated: false, user: undefined });
		}),
	} as unknown as AuthContextProps);
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	queryClient.clear();
	vi.useRealTimers();
});

describe("WebAuthProvider", () => {
	it("does not initialize an OIDC session after its waiting provider unmounts", async () => {
		let finish!: (config: unknown) => void;
		harness.loadConfiguration.mockReturnValueOnce(
			new Promise((resolve) => {
				finish = resolve;
			}),
		);
		await renderProvider();
		expect(container.textContent).toBe("Loading");
		await act(async () => root.render(null));
		await act(async () => finish({ client_id: "late-client" }));
		expect(harness.createUserManager).not.toHaveBeenCalled();
		expect(harness.childMounts).toBe(0);
	});

	it("stops the initialized manager on unmount", async () => {
		await renderProvider();
		expect(harness.createUserManager).toHaveBeenCalledOnce();
		await act(async () => root.render(null));
		expect(harness.stopSilentRenew).toHaveBeenCalledOnce();
	});

	it("reports a failed configuration request without initializing an OIDC session", async () => {
		harness.loadConfiguration.mockRejectedValueOnce(
			new Error("Network unavailable"),
		);
		await renderProvider();
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			"Web configuration is unavailable",
		);
		expect(harness.createUserManager).not.toHaveBeenCalled();
		expect(harness.childMounts).toBe(0);
	});

	it.each(["/thirdparty/callback", "/thirdparty/callback/"])(
		"restores the Hub user without consuming third-party state on %s",
		async (path) => {
			harness.useRealOidc = true;
			harness.showCallbackReadiness = true;
			harness.path = path;
			window.history.replaceState(
				null,
				"",
				`${path}?code=google&state=workflow`,
			);
			harness.signinCallback.mockRejectedValue(new Error("No matching state"));

			await renderProvider();

			expect(harness.signinCallback).not.toHaveBeenCalled();
			expect(harness.getUser).toHaveBeenCalledOnce();
			expect(harness.backend.pushAuthContext).toHaveBeenLastCalledWith(
				expect.objectContaining({
					isAuthenticated: true,
					user: user(),
				}),
			);
			expect(container.textContent).toBe("Callback ready");
		},
	);

	it("still consumes the Hub sign-in callback on the account callback route", async () => {
		harness.useRealOidc = true;
		harness.path = "/callback";
		window.history.replaceState(null, "", "/callback?code=hub&state=account");

		await renderProvider();

		expect(harness.signinCallback).toHaveBeenCalledOnce();
		expect(harness.getUser).not.toHaveBeenCalled();
		expect(harness.backend.pushAuthContext).toHaveBeenLastCalledWith(
			expect.objectContaining({ isAuthenticated: true, user: user() }),
		);
	});

	it("keeps a public OAuth callback waiting until the Hub user and profile are ready", async () => {
		harness.path = "/thirdparty/callback";
		harness.showCallbackReadiness = true;
		harness.auth.set({
			isLoading: true,
			isAuthenticated: false,
			user: undefined,
		});
		let finishProfile!: (
			settings: Awaited<
				ReturnType<typeof harness.backend.userState.getSettingsProfile>
			>,
		) => void;
		harness.backend.userState.getSettingsProfile.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finishProfile = resolve;
				}),
		);

		await renderProvider();
		expect(container.textContent).toBe("Callback waiting");
		expect(harness.backend.userState.getSettingsProfile).not.toHaveBeenCalled();

		await act(async () =>
			harness.auth.set({
				isLoading: false,
				isAuthenticated: true,
				user: user(),
			}),
		);
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledOnce();
		expect(container.textContent).toBe("Callback waiting");
		expect(harness.backend.pushProfile).not.toHaveBeenCalled();

		await act(async () =>
			finishProfile({
				hub_profile: await harness.backend.userState.getProfile(),
				execution_settings: { gpu_mode: false, max_context_size: 8192 },
			}),
		);
		expect(harness.backend.pushProfile).toHaveBeenCalledOnce();
		expect(container.textContent).toBe("Callback ready");
	});

	it("settles OAuth callback readiness after initialization without a signed-in user", async () => {
		harness.path = "/thirdparty/callback";
		harness.showCallbackReadiness = true;
		harness.auth.set({
			isLoading: true,
			isAuthenticated: false,
			user: undefined,
		});
		await renderProvider();
		expect(container.textContent).toBe("Callback waiting");

		await act(async () => harness.auth.set({ isLoading: false }));

		expect(container.textContent).toBe("Callback ready");
		expect(harness.backend.userState.getSettingsProfile).not.toHaveBeenCalled();
	});

	it("replaces fresh empty library caches after login without switching profiles", async () => {
		harness.showLibrary = true;
		queryClient.setQueryData(["getApps"], []);
		queryClient.setQueryData(["getSettingsProfile"], {
			hub_profile: { name: "Saved profile", bits: [], apps: [] },
		});
		await renderProvider();
		await vi.waitFor(() => expect(container.textContent).toBe("Saved app"));
		expect(harness.backend.appState.getApps).toHaveBeenCalledTimes(1);
		expect(harness.backend.userState.getProfile).toHaveBeenCalledTimes(1);
	});

	it("seeds the account-scoped sidebar reads before mounting the workspace", async () => {
		await renderProvider();
		// useDeveloperMode and account dialogs share the legacy getInfo key.
		await queryClient.fetchQuery({
			queryKey: ["getInfo"],
			queryFn: () => harness.backend.userState.getInfo(),
		});
		const deps = accountQueryDeps(
			harness.backend.userState,
			harness.auth.get(),
		);
		const functions = [
			"getInfo",
			"getProfile",
			"getSettingsProfile",
			"getAllSettingsProfiles",
		] as const;
		for (const name of functions) {
			const queryKey = [name, ...deps];
			expect(queryClient.getQueryData(queryKey)).toBeDefined();
			expect(queryClient.getQueryDefaults(queryKey).persister).toBeUndefined();
			await queryClient.fetchQuery<unknown>({
				queryKey,
				queryFn: () => harness.backend.userState[name](),
			});
		}
		expect(harness.backend.userState.getInfo).toHaveBeenCalledTimes(1);
		expect(harness.backend.userState.getProfile).toHaveBeenCalledTimes(1);
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledTimes(
			1,
		);
		expect(
			harness.backend.userState.getAllSettingsProfiles,
		).toHaveBeenCalledTimes(1);
		expect(
			queryClient.getQueryData(["getAllSettingsProfiles", ...deps]),
		).toEqual([queryClient.getQueryData(["getSettingsProfile", ...deps])]);
	});

	it("refreshes the same account's seeded reads after signing out and back in", async () => {
		await renderProvider();
		const deps = accountQueryDeps(
			harness.backend.userState,
			harness.auth.get(),
		);
		await act(async () => harness.auth.get().removeUser());
		expect(container.textContent).toBe("Sign in required");
		harness.backend.userState.getInfo.mockResolvedValueOnce({
			id: "updated-user",
		});
		await act(async () =>
			harness.auth.set({ user: user("new-session"), isAuthenticated: true }),
		);
		expect(container.textContent).toBe("App content");
		expect(harness.backend.userState.getInfo).toHaveBeenCalledTimes(2);
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledTimes(
			2,
		);
		expect(queryClient.getQueryData(["getInfo", ...deps])).toEqual({
			id: "updated-user",
		});
		expect(queryClient.getQueryData(["getInfo"])).toEqual({
			id: "updated-user",
		});
	});

	it("discards a pending pre-login user response before seeding shared UI hooks", async () => {
		let finishOld!: (info: { id: string }) => void;
		const observer = new QueryObserver(queryClient, {
			queryKey: ["getInfo"],
			queryFn: () =>
				new Promise<{ id: string }>((resolve) => {
					finishOld = resolve;
				}),
		});
		const unsubscribe = observer.subscribe(() => {});
		try {
			await renderProvider();
			await act(async () => finishOld({ id: "previous-account" }));
			expect(queryClient.getQueryData(["getInfo"])).toEqual({ id: "user-1" });
			expect(harness.backend.userState.getInfo).toHaveBeenCalledOnce();
		} finally {
			unsubscribe();
		}
	});

	it("bypasses persisted empty lists even when the first signed-in request retries after mount", async () => {
		harness.showLibrary = true;
		const get = vi.fn(async (storageKey: string) => {
			const queryKey = JSON.parse(storageKey.slice("fl-q-".length));
			return JSON.stringify({
				buster: "fl-qp-v1",
				queryKey,
				queryHash: JSON.stringify(queryKey),
				state: {
					data:
						queryKey[0] === "getSettingsProfile"
							? { hub_profile: { name: "Old profile", bits: [], apps: [] } }
							: [],
					dataUpdatedAt: Date.now(),
				},
			});
		});
		const persister = createSmartQueryPersister({
			backend: {
				get,
				set: async () => {},
				del: async () => {},
				entries: async () => [],
			},
		});
		queryClient.setDefaultOptions({
			queries: {
				staleTime: 30_000,
				retry: 1,
				retryDelay: 0,
				persister: persister.persisterFn,
			},
		});
		let rejectFirst!: (reason: Error) => void;
		harness.backend.appState.getApps.mockImplementationOnce(
			() =>
				new Promise((_, reject) => {
					rejectFirst = reject;
				}),
		);
		await renderProvider();
		expect(harness.childMounts).toBe(1);
		await act(async () => rejectFirst(new Error("Temporary network failure")));
		await vi.waitFor(() => expect(container.textContent).toBe("Saved app"));
		expect(harness.backend.appState.getApps).toHaveBeenCalledTimes(2);
		expect(get).not.toHaveBeenCalled();
	});

	it("discards a pending pre-login app response after loading the signed-in library", async () => {
		harness.showLibrary = true;
		let finishSignedOut!: (apps: []) => void;
		const observer = new QueryObserver(queryClient, {
			queryKey: ["getApps"],
			queryFn: () =>
				new Promise<[]>((resolve) => {
					finishSignedOut = resolve;
				}),
		});
		const unsubscribe = observer.subscribe(() => {});
		try {
			await renderProvider();
			await vi.waitFor(() => expect(container.textContent).toBe("Saved app"));
			await act(async () => finishSignedOut([]));
			expect(container.textContent).toBe("Saved app");
			expect(queryClient.getQueryData(["getApps"])).toEqual([
				[{ id: "saved-app" }, { name: "Saved app" }],
			]);
		} finally {
			unsubscribe();
		}
	});

	it("stops user initialization on 401 without retrying or loading protected content", async () => {
		vi.useFakeTimers();
		harness.backend.userState.getInfo.mockRejectedValue(
			new ApiResponseError({ status: 401, message: "Session expired" }),
		);
		await renderProvider();
		await act(async () => vi.runAllTimersAsync());

		expect(harness.backend.userState.getInfo).toHaveBeenCalledTimes(1);
		expect(harness.backend.userState.getProfile).not.toHaveBeenCalled();
		expect(harness.backend.pushProfile).not.toHaveBeenCalled();
		expect(harness.childMounts).toBe(0);
	});

	it("does not substitute a default profile when the profile request returns 401", async () => {
		harness.backend.userState.getProfile.mockRejectedValue(
			new ApiResponseError({ status: 401, message: "Session expired" }),
		);
		await renderProvider();

		expect(harness.backend.userState.getProfile).toHaveBeenCalledTimes(1);
		expect(harness.backend.pushProfile).not.toHaveBeenCalled();
		expect(harness.childMounts).toBe(0);
	});

	it("pushes a renewed access token to the backend when the ID token is unchanged", async () => {
		await renderProvider();
		expect(harness.childMounts).toBe(1);
		harness.backend.pushAuthContext.mockClear();
		await act(async () => harness.auth.set({ user: user("token-2") }));

		expect(harness.backend.pushAuthContext).toHaveBeenLastCalledWith(
			expect.objectContaining({
				user: expect.objectContaining({
					access_token: "token-2",
					id_token: "unchanged-id-token",
				}),
			}),
		);
		expect(container.textContent).toBe("App content");
		expect(harness.childMounts).toBe(1);
		expect(harness.backend.userState.getInfo).toHaveBeenCalledOnce();
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledOnce();
		expect(harness.backend.appState.getApps).toHaveBeenCalledOnce();
		expect(harness.backend.appState.getMyGroups).toHaveBeenCalledOnce();
	});

	it("keeps the loaded workspace mounted during silent token renewal", async () => {
		await renderProvider();
		const content = container.querySelector(
			'[data-testid="protected-content"]',
		);

		await act(async () =>
			harness.auth.set({ isLoading: true, activeNavigator: "signinSilent" }),
		);
		expect(container.textContent).toBe("App content");
		expect(container.querySelector('[data-testid="protected-content"]')).toBe(
			content,
		);

		await act(async () =>
			harness.auth.set({
				isLoading: false,
				activeNavigator: undefined,
				user: user("token-2"),
			}),
		);
		expect(harness.childMounts).toBe(1);
		expect(harness.backend.userState.getInfo).toHaveBeenCalledOnce();
		expect(harness.backend.appState.getApps).toHaveBeenCalledOnce();
	});

	it.each([false, true])(
		"preserves workspace state through expiry recovery (active navigator: %s)",
		async (navigatorActive) => {
			let expired = false;
			let finishRenewal!: (renewed: ReturnType<typeof user>) => void;
			const renewal = new Promise<ReturnType<typeof user>>((resolve) => {
				finishRenewal = resolve;
			});
			const signinSilent = vi.fn(async () => {
				if (navigatorActive) {
					harness.auth.set({
						isLoading: true,
						activeNavigator: "signinSilent",
					});
				}
				const renewed = await renewal;
				harness.auth.set({
					user: renewed,
					isLoading: false,
					activeNavigator: undefined,
				});
				return renewed;
			});
			harness.auth.set({
				signinSilent,
				user: user("token-1", { expired: () => expired }),
			});
			await renderProvider();
			const content = container.querySelector(
				'[data-testid="protected-content"]',
			);
			const draft = container.querySelector("input");
			if (!draft) throw new Error("Workspace draft did not mount");
			draft.value = "Unsaved widget state";

			await act(async () => {
				if (navigatorActive) void signinSilent();
				expired = true;
				vi.mocked(
					harness.auth.get().events.addAccessTokenExpired,
				).mock.calls[0][0]();
			});
			expect(signinSilent).toHaveBeenCalledOnce();
			expect(container.textContent).toBe("App content");
			expect(container.querySelector('[data-testid="protected-content"]')).toBe(
				content,
			);
			expect(container.querySelector("input")).toBe(draft);
			expect(draft.value).toBe("Unsaved widget state");

			await act(async () => finishRenewal(user("token-2")));
			expect(container.querySelector("input")).toBe(draft);
			expect(draft.value).toBe("Unsaved widget state");
			expect(harness.childMounts).toBe(1);
			expect(
				harness.backend.userState.getSettingsProfile,
			).toHaveBeenCalledOnce();
			expect(harness.auth.get().signinRedirect).not.toHaveBeenCalled();
			expect(harness.auth.get().removeUser).not.toHaveBeenCalled();
		},
	);

	it("waits for expiry recovery before mounting a cold workspace", async () => {
		let finishRenewal!: (renewed: ReturnType<typeof user>) => void;
		harness.auth.set({
			isAuthenticated: false,
			user: user("token-1", { expired: true }),
			signinSilent: vi.fn(async () => {
				const renewed = await new Promise<ReturnType<typeof user>>(
					(resolve) => {
						finishRenewal = resolve;
					},
				);
				harness.auth.set({ isAuthenticated: true, user: renewed });
				return renewed;
			}),
		});
		await renderProvider();
		expect(container.textContent).toBe("Loading");
		expect(harness.childMounts).toBe(0);
		expect(harness.backend.userState.getSettingsProfile).not.toHaveBeenCalled();

		await act(async () => finishRenewal(user("token-2")));
		expect(container.textContent).toBe("App content");
		expect(harness.childMounts).toBe(1);
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledOnce();
	});

	it("removes the workspace when expiry recovery requires sign-in", async () => {
		let rejectRenewal!: (error: Error) => void;
		harness.auth.set({
			signinSilent: vi.fn(
				() =>
					new Promise<ReturnType<typeof user>>((_, reject) => {
						rejectRenewal = reject;
					}),
			),
		});
		await renderProvider();
		await act(async () =>
			harness.auth.set({ user: user("token-1", { expired: true }) }),
		);
		expect(container.textContent).toBe("App content");

		await act(async () =>
			rejectRenewal(
				Object.assign(new Error("Sign in required"), {
					error: "invalid_grant",
				}),
			),
		);
		expect(
			container.querySelector('[data-testid="protected-content"]'),
		).toBeNull();
		expect(harness.auth.get().removeUser).toHaveBeenCalledOnce();
		expect(harness.auth.get().signinRedirect).toHaveBeenCalledOnce();
		expect(harness.childMounts).toBe(1);
	});

	it.each(["network", "refusal"])(
		"keeps signout effective when pending renewal ends with %s",
		async (failure) => {
			let rejectRenewal!: (error: Error) => void;
			harness.auth.set({
				signinSilent: vi.fn(
					() =>
						new Promise<ReturnType<typeof user>>((_, reject) => {
							rejectRenewal = reject;
						}),
				),
			});
			await renderProvider();
			await act(async () =>
				harness.auth.set({ user: user("token-1", { expired: true }) }),
			);
			expect(container.textContent).toBe("App content");
			await act(async () => harness.auth.get().removeUser());
			expect(container.textContent).toBe("Sign in required");

			await act(async () =>
				rejectRenewal(
					failure === "network"
						? new TypeError("Failed to fetch")
						: Object.assign(new Error("Sign in required"), {
								error: "invalid_grant",
							}),
				),
			);
			expect(container.textContent).toBe("Sign in required");
			expect(harness.auth.get().removeUser).toHaveBeenCalledOnce();
			expect(harness.auth.get().signinRedirect).not.toHaveBeenCalled();
		},
	);

	it("loads a new account's profile before restoring the workspace", async () => {
		await renderProvider();
		let finishProfile!: (
			settings: Awaited<
				ReturnType<typeof harness.backend.userState.getSettingsProfile>
			>,
		) => void;
		harness.backend.userState.getSettingsProfile.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					finishProfile = resolve;
				}),
		);
		await act(async () =>
			harness.auth.set({
				user: user("token-2", { subject: "user-2" }),
			}),
		);
		expect(container.textContent).toBe("Loading");
		expect(harness.backend.userState.getSettingsProfile).toHaveBeenCalledTimes(
			2,
		);

		await act(async () =>
			finishProfile({
				hub_profile: await harness.backend.userState.getProfile(),
				execution_settings: { gpu_mode: false, max_context_size: 8192 },
			}),
		);
		expect(container.textContent).toBe("App content");
		expect(harness.backend.appState.getApps).toHaveBeenCalledTimes(2);
	});

	it("removes an already loaded app after rejection and shows sign-in if navigation fails", async () => {
		vi.mocked(harness.auth.get().signinRedirect).mockRejectedValue(
			new Error("Navigation failed"),
		);
		await renderProvider();
		expect(container.textContent).toBe("App content");
		await act(async () => reportUnauthorized(harness.auth.get(), "token-1"));

		expect(
			container.querySelector('[data-testid="protected-content"]'),
		).toBeNull();
		expect(container.textContent).toBe("Sign in required");
		expect(harness.auth.get().signinRedirect).toHaveBeenCalledTimes(1);

		await act(async () =>
			harness.auth.set({ isAuthenticated: true, user: user("token-2") }),
		);
		expect(container.textContent).toBe("App content");
	});

	it.each(["/store/explore", "/callback", "/thirdparty/callback"])(
		"allows %s to render while an old session is rejected",
		async (path) => {
			harness.path = path;
			window.history.replaceState(null, "", path);
			reportUnauthorized(harness.auth.get(), "token-1");
			await renderProvider();

			expect(container.textContent).toBe("App content");
			expect(harness.auth.get().removeUser).not.toHaveBeenCalled();
			expect(harness.auth.get().signinRedirect).not.toHaveBeenCalled();
		},
	);

	it("returns to normal sign-in after a recovered session is deliberately signed out", async () => {
		await renderProvider();
		await act(async () => reportUnauthorized(harness.auth.get(), "token-1"));
		await act(async () => reportUnauthorized(harness.auth.get(), undefined));
		await act(async () =>
			harness.auth.set({ isAuthenticated: true, user: user("token-2") }),
		);
		expect(container.textContent).toBe("App content");
		await act(async () =>
			harness.auth.set({ isAuthenticated: false, user: undefined }),
		);

		expect(container.textContent).toBe("Sign in required");
		expect(harness.auth.get().signinRedirect).toHaveBeenCalledTimes(1);
	});
});
