// @vitest-environment happy-dom
import { act, createContext, forwardRef, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface AuthUser {
	access_token: string;
	profile: { sub: string };
}

interface AuthState {
	isLoading: boolean;
	isAuthenticated: boolean;
	activeNavigator?: string;
	error?: Error;
	user?: AuthUser;
	signinRedirect: (args?: unknown) => Promise<void>;
}

const harness = vi.hoisted(() => {
	let state = {} as AuthState;
	const listeners = new Set<() => void>();
	return {
		kind: "u" as "u" | "c" | "f",
		formProps: [] as Record<string, unknown>[],
		chatRenders: 0,
		pageMounts: 0,
		pageUnmounts: 0,
		bootstrapTokens: [] as (string | undefined)[],
		requestTokens: [] as (string | undefined)[],
		bootstrapStatus: 200,
		runtimeRequest: null as
			| null
			| (((suffix?: string, init?: RequestInit) => Promise<Response>) & {
					prepareExecution: (body: string) => Promise<{
						request: { body: string; headers: { authorization: string } };
					}>;
					executionScope: () => string;
			  }),
		accessToken: null as null | (() => string | undefined),
		backendBuilds: 0,
		executionScopes: [] as (string | undefined)[],
		authProxy: false,
		oauthTokens: undefined as unknown,
		oauthApps: [] as string[],
		oauthOpened: 0,
		promptMounts: 0,
		auth: {
			get: () => state,
			set(next: Partial<AuthState>) {
				state = { ...state, ...next };
				for (const listener of listeners) listener();
			},
			reset(next: AuthState) {
				state = next;
			},
			subscribe(listener: () => void) {
				listeners.add(listener);
				return () => listeners.delete(listener);
			},
		},
	};
});

vi.mock("react-oidc-context", async () => {
	const { useSyncExternalStore } = await import("react");
	return {
		AuthProvider: ({ children }: { children: React.ReactNode }) => children,
		useAuth: () =>
			useSyncExternalStore(harness.auth.subscribe, harness.auth.get),
	};
});

const searchParams = new URLSearchParams();
vi.mock("next/navigation", () => ({ useSearchParams: () => searchParams }));

vi.mock("../lib/hosted-backend", () => {
	class HostedHttpError extends Error {
		constructor(
			public readonly status: number,
			message: string,
		) {
			super(message);
		}
	}
	return {
		HostedHttpError,
		createHostedRequest: (
			target: { variant?: string },
			token?: string,
			auth?: AuthState,
		) =>
			Object.assign(
				async (suffix = "") => {
					if (target.variant) {
						harness.requestTokens.push(token);
						return new Response("[]");
					}
					harness.bootstrapTokens.push(token);
					if (harness.bootstrapStatus !== 200)
						throw new HostedHttpError(harness.bootstrapStatus, "denied");
					return new Response(
						JSON.stringify({
							app_id: "app-1",
							kind: harness.kind,
							auth_proxy: harness.authProxy,
							bootstrap: {
								event: { id: "event-1", name: "Home", config: [] },
								page: { id: "page-1" },
								servedVariant: "stable",
							},
						}),
					);
				},
				{
					prepareExecution: async (body: string) => ({
						request: { body, headers: { authorization: `Bearer ${token}` } },
					}),
					executionScope: () =>
						`hosted:${auth?.user?.profile.sub ?? localStorage.getItem("flow-like-hosted-session") ?? "anonymous"}`,
				},
			),
		createHostedBackend: (
			_data: unknown,
			request: typeof harness.runtimeRequest,
			accessToken: typeof harness.accessToken,
			_auth: unknown,
			oauthTokens: unknown,
		) => {
			harness.runtimeRequest = request;
			harness.accessToken = accessToken;
			harness.oauthTokens = oauthTokens;
			harness.backendBuilds += 1;
			return {};
		},
	};
});

vi.mock("../lib/hosted-oauth", () => ({
	HostedOAuth: class {
		readonly tokensFor = async () => undefined;
		constructor(appId: string) {
			harness.oauthApps.push(appId);
		}
		open() {
			harness.oauthOpened += 1;
			return () => {
				harness.oauthOpened -= 1;
			};
		}
	},
}));

vi.mock("./hosted-oauth-prompt", () => ({
	HostedOAuthPrompt: () => {
		useEffect(() => {
			harness.promptMounts += 1;
			return () => {
				harness.promptMounts -= 1;
			};
		}, []);
		return null;
	},
}));

vi.mock("@flow-like/flow-like-ui/components/interfaces/page-interface", () => ({
	PageInterface: () => {
		useEffect(() => {
			harness.pageMounts += 1;
			return () => {
				harness.pageUnmounts += 1;
			};
		}, []);
		return <div data-testid="page" />;
	},
}));

const passthrough = ({ children }: { children?: React.ReactNode }) => (
	<>{children}</>
);

vi.mock("@flow-like/flow-like-ui/components/interfaces/chat-default", () => ({
	ChatInterface: () => {
		harness.chatRenders += 1;
		return null;
	},
}));
vi.mock(
	"@flow-like/flow-like-ui/components/interfaces/chat-default/message",
	() => ({ ChatFeedbackEnabledContext: createContext(true) }),
);
vi.mock("@flow-like/flow-like-ui/components/interfaces/container", () => ({
	Container: forwardRef<HTMLDivElement, { children?: React.ReactNode }>(
		({ children }, ref) => <div ref={ref}>{children}</div>,
	),
}));
vi.mock("@flow-like/flow-like-ui/components/interfaces/form-workbench", () => ({
	FormWorkbenchInterface: (props: Record<string, unknown>) => {
		harness.formProps.push(props);
		return null;
	},
}));
vi.mock("@flow-like/flow-like-ui/components/scoped-custom-css", () => ({
	ScopedCustomCss: () => null,
}));
vi.mock("@flow-like/flow-like-ui/components/theme-provider", () => ({
	ThemeProvider: passthrough,
}));
vi.mock("@flow-like/flow-like-ui/components/ui/sonner", () => ({
	Toaster: () => null,
}));
vi.mock("@flow-like/flow-like-ui/components/ui/tooltip", () => ({
	TooltipProvider: passthrough,
}));
vi.mock("@flow-like/flow-like-ui/lib/api-url", () => ({
	getApiUrl: (_: unknown, path: string) => `https://api.test/${path}`,
}));
vi.mock("@flow-like/flow-like-ui/lib/set-query-params", () => ({
	QueryParamNavigationContext: createContext(null),
}));
vi.mock("@flow-like/flow-like-ui/lib/uint8", () => ({
	parseUint8ArrayToJson: () => ({}),
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => {
	let backend: unknown;
	return {
		useBackendStore: {
			getState: () => ({
				backend,
				setBackend: (next: unknown) => {
					backend = next;
				},
			}),
			setState: (next: { backend: unknown }) => {
				backend = next.backend;
			},
		},
	};
});
vi.mock("@flow-like/flow-like-ui/state/execution-engine-context", () => ({
	ExecutionEngineProviderComponent: ({
		children,
		executionScope,
	}: { children: React.ReactNode; executionScope?: string }) => {
		harness.executionScopes.push(executionScope);
		return children;
	},
}));
vi.mock("@flow-like/locales", () => ({
	I18nProvider: passthrough,
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("../lib/return-url", () => ({ saveReturnUrl: () => undefined }));
vi.mock("../lib/oidc-settings", () => ({
	getWebOidcSettings: (s: unknown) => s,
}));

const { HostedSession } = await import("./hosted-frontend");

const signedIn = (sub: string, token: string): Partial<AuthState> => ({
	isLoading: false,
	isAuthenticated: true,
	user: { access_token: token, profile: { sub } },
});

async function flush() {
	for (let i = 0; i < 5; i++)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
}

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	window.history.replaceState(null, "", "/a/app-1/home");
	harness.kind = "u";
	harness.formProps = [];
	harness.chatRenders = 0;
	harness.pageMounts = 0;
	harness.pageUnmounts = 0;
	harness.bootstrapTokens = [];
	harness.requestTokens = [];
	harness.bootstrapStatus = 200;
	harness.runtimeRequest = null;
	harness.accessToken = null;
	harness.backendBuilds = 0;
	harness.executionScopes = [];
	localStorage.removeItem("flow-like-hosted-session");
	harness.authProxy = false;
	harness.oauthTokens = undefined;
	harness.oauthApps = [];
	harness.oauthOpened = 0;
	harness.promptMounts = 0;
	harness.auth.reset({
		isLoading: false,
		isAuthenticated: true,
		user: { access_token: "token-1", profile: { sub: "user-1" } },
		signinRedirect: vi.fn(async () => undefined),
	});
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("HostedSession", () => {
	it("keeps the runtime mounted when the access token is renewed", async () => {
		harness.authProxy = true;
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.pageMounts).toBe(1);
		expect(harness.bootstrapTokens).toEqual(["token-1"]);

		await act(async () => harness.auth.set(signedIn("user-1", "token-2")));
		await flush();

		expect(harness.pageMounts).toBe(1);
		expect(harness.pageUnmounts).toBe(0);
		expect(harness.bootstrapTokens).toEqual(["token-1"]);

		await harness.runtimeRequest?.("/routes");
		expect(harness.requestTokens).toEqual(["token-2"]);
		expect(await harness.runtimeRequest?.prepareExecution("{}")).toEqual({
			request: { body: "{}", headers: { authorization: "Bearer token-2" } },
		});
		expect(harness.runtimeRequest?.executionScope()).toBe("hosted:user-1");
	});

	it("lets package widgets ask as the viewer, with the renewed token and the same backend", async () => {
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.backendBuilds).toBe(1);
		expect(harness.accessToken?.()).toBe("token-1");

		await act(async () => harness.auth.set(signedIn("user-1", "token-2")));
		await flush();

		expect(harness.backendBuilds).toBe(1);
		expect(harness.accessToken?.()).toBe("token-2");
	});

	it("reloads the interface for a different user", async () => {
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.pageMounts).toBe(1);

		await act(async () => harness.auth.set(signedIn("user-2", "token-9")));
		await flush();

		expect(harness.pageUnmounts).toBe(1);
		expect(harness.pageMounts).toBe(2);
		expect(harness.bootstrapTokens).toEqual(["token-1", "token-9"]);
	});

	it("drops the interface and signs in again after sign-out", async () => {
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.pageMounts).toBe(1);

		harness.bootstrapStatus = 401;
		await act(async () =>
			harness.auth.set({ isAuthenticated: false, user: undefined }),
		);
		await flush();

		expect(harness.pageUnmounts).toBe(1);
		expect(harness.bootstrapTokens).toEqual(["token-1", undefined]);
		expect(harness.auth.get().signinRedirect).toHaveBeenCalledTimes(1);
	});

	it("opens a form link as the hosted host, with the hosted header as its toolbar", async () => {
		harness.kind = "f";
		await act(async () => root.render(<HostedSession />));
		await flush();

		expect(harness.formProps.at(-1)).toMatchObject({
			host: "hosted",
			appId: "app-1",
			event: { id: "event-1" },
			onNavigate: expect.any(Function),
			toolbarRef: {
				current: {
					pushToolbarElements: expect.any(Function),
					pushNavElements: expect.any(Function),
				},
			},
		});
		expect(harness.chatRenders).toBe(0);
		expect(harness.pageMounts).toBe(0);
	});

	it("never mounts the form for a chat or Page link", async () => {
		harness.kind = "c";
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.chatRenders).toBeGreaterThan(0);

		await act(async () => root.unmount());
		root = createRoot(container);
		harness.kind = "u";
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.pageMounts).toBe(1);

		expect(harness.formProps).toEqual([]);
	});

	it("offers the visitor of a sign-in interface their own sign-ins, for every kind", async () => {
		harness.authProxy = true;
		for (const kind of ["c", "f", "u"] as const) {
			harness.kind = kind;
			await act(async () => root.render(<HostedSession />));
			await flush();
			expect(harness.oauthTokens).toEqual(expect.any(Function));
			expect(harness.promptMounts).toBe(1);
			expect(harness.oauthOpened).toBe(1);
			await act(async () => root.unmount());
			root = createRoot(container);
		}
		expect(harness.oauthApps).toEqual(["app-1", "app-1", "app-1"]);
		expect(harness.promptMounts).toBe(0);
		expect(harness.oauthOpened).toBe(0);
	});

	it("never offers an anonymous interface's visitor the prompt", async () => {
		for (const kind of ["c", "f", "u"] as const) {
			harness.kind = kind;
			await act(async () => root.render(<HostedSession />));
			await flush();
			expect(harness.backendBuilds).toBeGreaterThan(0);
			expect(harness.oauthTokens).toBeUndefined();
			await act(async () => root.unmount());
			root = createRoot(container);
		}
		expect(harness.oauthApps).toEqual([]);
		expect(harness.promptMounts).toBe(0);
		expect(harness.oauthOpened).toBe(0);
	});

	it("shares the anonymous scope when another tab establishes the browser session", async () => {
		localStorage.setItem("flow-like-hosted-session", "anonymous-first-session");
		await act(async () => root.render(<HostedSession />));
		await flush();
		expect(harness.executionScopes.at(-1)).toBe(
			"hosted:anonymous-first-session",
		);
		await harness.runtimeRequest?.("/routes");
		expect(harness.requestTokens).toEqual([undefined]);
		await act(async () => {
			localStorage.setItem(
				"flow-like-hosted-session",
				"anonymous-shared-session",
			);
			window.dispatchEvent(
				new StorageEvent("storage", { key: "flow-like-hosted-session" }),
			);
		});
		expect(harness.executionScopes.at(-1)).toBe(
			"hosted:anonymous-shared-session",
		);
		expect(harness.pageMounts).toBe(1);
	});
});
