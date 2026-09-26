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
		pageMounts: 0,
		pageUnmounts: 0,
		bootstrapTokens: [] as (string | undefined)[],
		requestTokens: [] as (string | undefined)[],
		bootstrapStatus: 200,
		runtimeRequest: null as
			| null
			| ((suffix?: string, init?: RequestInit) => Promise<Response>),
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
		createHostedRequest:
			(target: { variant?: string }, token?: string) =>
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
						kind: "u",
						auth_proxy: false,
						bootstrap: {
							event: { id: "event-1", name: "Home", config: [] },
							page: { id: "page-1" },
							servedVariant: "stable",
						},
					}),
				);
			},
		createHostedBackend: (
			_data: unknown,
			request: typeof harness.runtimeRequest,
		) => {
			harness.runtimeRequest = request;
			return {};
		},
	};
});

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
	ChatInterface: () => null,
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
vi.mock(
	"@flow-like/flow-like-ui/components/interfaces/generic-event-form",
	() => ({ GenericEventFormInterface: () => null }),
);
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
	ExecutionEngineProviderComponent: passthrough,
}));
vi.mock("@flow-like/locales", () => ({ I18nProvider: passthrough }));
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
	harness.pageMounts = 0;
	harness.pageUnmounts = 0;
	harness.bootstrapTokens = [];
	harness.requestTokens = [];
	harness.bootstrapStatus = 200;
	harness.runtimeRequest = null;
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
});
