// @vitest-environment happy-dom
import { act, useSyncExternalStore } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	isSessionRejected,
	reportUnauthorized,
	subscribeSessionRejection,
} from "../lib/auth-session";
import { consumeReturnUrl } from "../lib/return-url";
import { useSessionRecovery } from "./use-session-recovery";

function createEvents() {
	const expired = new Set<() => void>();
	const renewError = new Set<() => void>();
	return {
		addAccessTokenExpired: (listener: () => void) => expired.add(listener),
		removeAccessTokenExpired: (listener: () => void) =>
			expired.delete(listener),
		addSilentRenewError: (listener: () => void) => renewError.add(listener),
		removeSilentRenewError: (listener: () => void) =>
			renewError.delete(listener),
		emitExpired: () => {
			for (const listener of expired) listener();
		},
		emitRenewError: () => {
			for (const listener of renewError) listener();
		},
		listenerCount: () => expired.size + renewError.size,
	};
}

const listeners = new Set<() => void>();
let auth: AuthContextProps;
let events: ReturnType<typeof createEvents>;
let tokenExpired = false;

function user(accessToken = "token-1") {
	return {
		access_token: accessToken,
		id_token: "same-id-token",
		profile: { sub: "user-1" },
		get expired() {
			return tokenExpired;
		},
	} as AuthContextProps["user"];
}

const authStore = {
	get: () => auth,
	subscribe(listener: () => void) {
		listeners.add(listener);
		return () => listeners.delete(listener);
	},
	set(next: Partial<AuthContextProps>) {
		auth = { ...auth, ...next };
		for (const listener of listeners) listener();
	},
};

function ProtectedView({ isPublicPath = false }: { isPublicPath?: boolean }) {
	const current = useSyncExternalStore(authStore.subscribe, authStore.get);
	const recovery = useSessionRecovery(current, isPublicPath);
	return recovery === "ready" ? (
		<div data-testid="protected-content">App content</div>
	) : (
		<output>{recovery}</output>
	);
}

function deferred() {
	let resolve!: () => void;
	const promise = new Promise<void>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	window.history.replaceState(
		null,
		"",
		"/apps/app-1/boards?view=canvas#node-7",
	);
	localStorage.clear();
	sessionStorage.clear();
	tokenExpired = false;
	events = createEvents();
	auth = {
		isLoading: false,
		isAuthenticated: true,
		user: user(),
		events,
		signinRedirect: vi.fn(async () => undefined),
		removeUser: vi.fn(async () => {
			authStore.set({ isAuthenticated: false, user: undefined });
		}),
	} as unknown as AuthContextProps;
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	expect(events.listenerCount()).toBe(0);
	container.remove();
});

describe("useSessionRecovery", () => {
	it("hides protected content immediately and redirects once for concurrent 401s across auth updates", async () => {
		const removal = deferred();
		const requestAuth = auth;
		vi.mocked(auth.removeUser).mockImplementation(async () => {
			await removal.promise;
			authStore.set({ isAuthenticated: false, user: undefined });
		});
		await act(async () => root.render(<ProtectedView />));
		expect(
			container.querySelector('[data-testid="protected-content"]'),
		).not.toBeNull();

		act(() => reportUnauthorized(requestAuth, "token-1"));
		expect(
			container.querySelector('[data-testid="protected-content"]'),
		).toBeNull();
		expect(container.textContent).toBe("redirecting");
		expect(auth.removeUser).toHaveBeenCalledTimes(1);
		expect(auth.signinRedirect).not.toHaveBeenCalled();

		await act(async () => {
			authStore.set({ user: user() });
			reportUnauthorized(auth, "token-1");
			reportUnauthorized(requestAuth, "token-1");
			removal.resolve();
		});

		expect(auth.removeUser).toHaveBeenCalledTimes(1);
		expect(auth.signinRedirect).toHaveBeenCalledExactlyOnceWith({
			url_state: "/apps/app-1/boards?view=canvas#node-7",
		});
		expect(consumeReturnUrl()).toBe("/apps/app-1/boards?view=canvas#node-7");
		expect(
			container.querySelector('[data-testid="protected-content"]'),
		).toBeNull();
	});

	it("ignores a late 401 for the previous token after the same user renews", async () => {
		const previousAuth = auth;
		await act(async () => root.render(<ProtectedView />));
		await act(async () => authStore.set({ user: user("token-2") }));
		await act(async () => reportUnauthorized(previousAuth, "token-1"));

		expect(container.textContent).toBe("App content");
		expect(auth.removeUser).not.toHaveBeenCalled();
		expect(auth.signinRedirect).not.toHaveBeenCalled();
	});

	it("keeps a current-token rejection when an old-token 401 arrives in the same batch", async () => {
		const previousAuth = auth;
		await act(async () => root.render(<ProtectedView />));
		await act(async () => authStore.set({ user: user("token-2") }));
		await act(async () => {
			reportUnauthorized(auth, "token-2");
			reportUnauthorized(previousAuth, "token-1");
		});

		expect(container.textContent).toBe("redirecting");
		expect(auth.removeUser).toHaveBeenCalledTimes(1);
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});

	it("does not start another redirect for anonymous 401s after clearing the user", async () => {
		await act(async () => root.render(<ProtectedView />));
		await act(async () => reportUnauthorized(auth, "token-1"));
		expect(auth.user).toBeUndefined();
		await act(async () => reportUnauthorized(auth, undefined));

		expect(container.textContent).toBe("redirecting");
		expect(auth.removeUser).toHaveBeenCalledTimes(1);
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});

	it("detects token expiry without an auth context or subject change", async () => {
		await act(async () => root.render(<ProtectedView />));
		await act(async () => {
			tokenExpired = true;
			events.emitExpired();
		});

		expect(container.textContent).toBe("redirecting");
		expect(auth.removeUser).toHaveBeenCalledTimes(1);
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});

	it("keeps a still-valid session when silent renewal temporarily fails", async () => {
		await act(async () => root.render(<ProtectedView />));
		await act(async () => events.emitRenewError());

		expect(container.textContent).toBe("App content");
		expect(auth.removeUser).not.toHaveBeenCalled();
		expect(auth.signinRedirect).not.toHaveBeenCalled();
	});

	it("recovers when silent renewal fails after the token has expired", async () => {
		await act(async () => root.render(<ProtectedView />));
		await act(async () => {
			tokenExpired = true;
			events.emitRenewError();
		});

		expect(container.textContent).toBe("redirecting");
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});

	it.each(["/store/explore", "/callback?code=returned&state=saved"])(
		"leaves the public path %s accessible without redirecting",
		async (path) => {
			window.history.replaceState(null, "", path);
			await act(async () => root.render(<ProtectedView isPublicPath />));
			await act(async () => {
				tokenExpired = true;
				reportUnauthorized(auth, "token-1");
			});

			expect(container.textContent).toBe("App content");
			expect(auth.removeUser).not.toHaveBeenCalled();
			expect(auth.signinRedirect).not.toHaveBeenCalled();
			expect(consumeReturnUrl()).toBeNull();
		},
	);

	it("offers sign-in after redirect failure and accepts a later fresh session", async () => {
		vi.mocked(auth.signinRedirect).mockRejectedValue(
			new Error("Navigation failed"),
		);
		await act(async () => root.render(<ProtectedView />));
		await act(async () => reportUnauthorized(auth, "token-1"));

		expect(container.textContent).toBe("sign-in-required");
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);

		await act(async () =>
			authStore.set({ isAuthenticated: true, user: user("token-2") }),
		);
		expect(container.textContent).toBe("App content");
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});

	it("offers sign-in when the OIDC provider records a redirect error", async () => {
		await act(async () => root.render(<ProtectedView />));
		await act(async () => reportUnauthorized(auth, "token-1"));
		await act(async () =>
			authStore.set({
				error: Object.assign(new Error("Navigation failed"), {
					source: "signinRedirect" as const,
					args: undefined,
				}),
			}),
		);

		expect(container.textContent).toBe("sign-in-required");
		expect(auth.signinRedirect).toHaveBeenCalledTimes(1);
	});
});

describe("session rejection notifications", () => {
	it("deduplicates reports across auth object replacements and unsubscribes listeners", () => {
		const listener = vi.fn();
		const unsubscribe = subscribeSessionRejection(listener);
		reportUnauthorized(auth, "token-1");
		reportUnauthorized({ ...auth }, "token-1");
		expect(isSessionRejected(auth, "token-1")).toBe(true);
		expect(isSessionRejected(auth, "token-2")).toBe(false);
		expect(listener).toHaveBeenCalledTimes(1);

		unsubscribe();
		reportUnauthorized(auth, "token-2");
		expect(listener).toHaveBeenCalledTimes(1);
	});

	it("does not reject a session for a request without an auth context", () => {
		reportUnauthorized(undefined, undefined);
		expect(isSessionRejected(auth, undefined)).toBe(false);
	});
});
