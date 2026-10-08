// @vitest-environment happy-dom
import { ApiResponseError } from "@flow-like/flow-like-ui/lib/api-error";
import { act, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { reportUnauthorized } from "../lib/auth-session";

const harness = vi.hoisted(() => {
	let state = {} as AuthContextProps;
	const listeners = new Set<() => void>();
	class WebBackend {
		pushAuthContext = vi.fn();
		pushProfile = vi.fn();
		userState = {
			getInfo: vi.fn(async () => ({ id: "user-1" })),
			getProfile: vi.fn(async () => ({ name: "Saved profile", bits: [] })),
		};
	}
	return {
		WebBackend,
		backend: new WebBackend(),
		path: "/apps/app-1",
		childMounts: 0,
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
vi.mock("oidc-client-ts", () => ({ UserManager: class {} }));
vi.mock("react-oidc-context", async () => {
	const { useSyncExternalStore } = await import("react");
	return {
		AuthProvider: ({ children }: { children: React.ReactNode }) => children,
		useAuth: () =>
			useSyncExternalStore(harness.auth.subscribe, harness.auth.get),
	};
});
vi.mock("../lib/api", () => ({
	get: async () => ({
		client_id: "web-client",
		authority: "https://login.test",
	}),
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

const { WebAuthProvider } = await import("./auth-provider");

function ProtectedContent() {
	useEffect(() => {
		harness.childMounts += 1;
	}, []);
	return <div data-testid="protected-content">App content</div>;
}

function user(accessToken = "token-1") {
	return {
		access_token: accessToken,
		id_token: "unchanged-id-token",
		profile: { sub: "user-1" },
		expired: false,
	} as AuthContextProps["user"];
}

let root: Root;
let container: HTMLDivElement;

async function renderProvider() {
	await act(async () =>
		root.render(
			<WebAuthProvider>
				<ProtectedContent />
			</WebAuthProvider>,
		),
	);
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	harness.backend = new harness.WebBackend();
	harness.path = "/apps/app-1";
	harness.childMounts = 0;
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
	vi.useRealTimers();
});

describe("WebAuthProvider", () => {
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
