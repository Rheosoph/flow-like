// @vitest-environment happy-dom
import { StrictMode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { consumeReturnUrl, saveReturnUrl } from "../../lib/return-url";
import CallbackPage from "./page";

const harness = vi.hoisted(() => ({
	auth: {} as AuthContextProps,
	router: { replace: vi.fn() },
}));

vi.mock("react-oidc-context", () => ({ useAuth: () => harness.auth }));
vi.mock("next/navigation", () => ({ useRouter: () => harness.router }));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

const RECOVERY_KEY = "flow-like-signin-callback-recovery";
const callbackUrl = "/callback?code=rejected-code&state=missing-state";
let root: Root;
let container: HTMLDivElement;
let postMessage: ReturnType<typeof vi.fn>;

function stateError(message = "No matching state found in storage") {
	return { name: "Error", message, source: "signinCallback" } as NonNullable<
		AuthContextProps["error"]
	>;
}

function render() {
	return act(async () => {
		root.render(
			<StrictMode>
				<CallbackPage />
			</StrictMode>,
		);
	});
}

async function remount() {
	await act(async () => root.unmount());
	root = createRoot(container);
	await render();
}

function button(text: string): HTMLButtonElement {
	const match = Array.from(container.querySelectorAll("button")).find(
		(candidate) => candidate.textContent === text,
	);
	if (!match) throw new Error(`Missing button: ${text}`);
	return match;
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	window.history.replaceState({ preserved: true }, "", callbackUrl);
	localStorage.clear();
	sessionStorage.clear();
	harness.router.replace.mockReset();
	harness.auth = {
		isLoading: false,
		isAuthenticated: false,
		error: stateError(),
		signinRedirect: vi.fn(() => new Promise<void>(() => {})),
	} as unknown as AuthContextProps;
	postMessage = vi.fn();
	vi.stubGlobal(
		"BroadcastChannel",
		class {
			postMessage = postMessage;
			close() {}
		},
	);
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

it.each([
	"No matching state found in storage",
	"State does not match",
	"No state in response",
])("starts one fresh login for %s, even under StrictMode", async (message) => {
	harness.auth.error = stateError(message);
	saveReturnUrl("/join?appId=app-1&token=invite#accept");
	await render();
	await render();
	expect(harness.auth.signinRedirect).toHaveBeenCalledExactlyOnceWith({
		url_state: "/join?appId=app-1&token=invite#accept",
		redirectMethod: "replace",
	});
	expect(window.location.search).toBe("");
	expect(window.history.state).toEqual({ preserved: true });
	expect(sessionStorage.getItem(RECOVERY_KEY)).toBe("1");
	expect(consumeReturnUrl()).toBe("/join?appId=app-1&token=invite#accept");
	expect(container.textContent).toContain("Signing you in...");
	expect(container.querySelector('[role="alert"]')).toBeNull();
	expect(postMessage).not.toHaveBeenCalled();
});

it("stops automatic retries across a full callback remount and allows manual retry", async () => {
	await render();
	window.history.replaceState(null, "", callbackUrl);
	await remount();
	expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(1);
	expect(container.textContent).toContain("Login expired, please try again.");
	expect(container.textContent).not.toContain("No matching state");
	await act(async () => button("Try Again").click());
	expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(2);
	window.history.replaceState(null, "", callbackUrl);
	await remount();
	expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(2);
});

it.each([
	"/callback?state=missing-state",
	"/callback?code=&state=missing-state",
	"/callback?code=rejected-code&state=missing-state&error=access_denied",
])(
	"requires a code and no provider error for automatic retry: %s",
	async (url) => {
		window.history.replaceState(null, "", url);
		await render();
		expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
		expect(container.textContent).toContain("Login expired, please try again.");
	},
);

it.each([
	"nonce mismatch",
	"authority mismatch on settings vs. signin state",
	"client_id mismatch on settings vs. signin state",
	"invalid_grant",
	"Failed to fetch",
])("leaves %s to manual retry and hides raw error details", async (message) => {
	harness.auth.error = stateError(message);
	await render();
	expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
	expect(container.textContent).toContain(
		"We couldn't complete your sign-in. Please try again.",
	);
	expect(container.textContent).not.toContain(message);
});

it("does not recover matching text from a different OIDC operation", async () => {
	harness.auth.error = {
		...stateError(),
		source: "signinRedirect",
	} as NonNullable<AuthContextProps["error"]>;
	await render();
	expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
});

it.each([
	{ isLoading: true },
	{ isAuthenticated: true },
	{ activeNavigator: "signinSilent" as const },
])(
	"does not restart while auth is busy or already signed in: %j",
	async (state) => {
		Object.assign(harness.auth, state);
		await render();
		expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
		expect(harness.router.replace).not.toHaveBeenCalled();
		expect(postMessage).not.toHaveBeenCalled();
	},
);

it.each(["getItem", "setItem"] as const)(
	"falls back to manual retry if sessionStorage.%s fails",
	async (operation) => {
		vi.stubGlobal("sessionStorage", {
			getItem: () => null,
			setItem: () => {},
			removeItem: () => {},
			[operation]: () => {
				throw new DOMException("Storage blocked", "SecurityError");
			},
		});
		await render();
		expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
		expect(container.textContent).toContain("Login expired, please try again.");
		await act(async () => button("Try Again").click());
		expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(1);
	},
);

it.each(["reject", "resolve-null"])(
	"restores retry controls when signinRedirect fails via %s",
	async (mode) => {
		vi.mocked(harness.auth.signinRedirect).mockImplementation(async () => {
			if (mode === "reject") throw new Error("Network unavailable");
			harness.auth.error = {
				...stateError("Network unavailable"),
				source: "signinRedirect",
			} as NonNullable<AuthContextProps["error"]>;
			return null as never;
		});
		await render();
		await render();
		expect(button("Try Again")).toBeDefined();
		expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(1);
		await act(async () => button("Try Again").click());
		expect(harness.auth.signinRedirect).toHaveBeenCalledTimes(2);
		expect(button("Try Again")).toBeDefined();
	},
);

it.each(["//evil.example", callbackUrl])(
	"ignores unvalidated callback state and stored return destination %s",
	async (returnUrl) => {
		window.history.replaceState(
			null,
			"",
			`${callbackUrl}%3B%2F%2Fevil.example`,
		);
		for (const storage of [localStorage, sessionStorage]) {
			storage.setItem(
				"flow-like-return-url",
				JSON.stringify({ url: returnUrl, ts: Date.now() }),
			);
		}
		await render();
		expect(harness.auth.signinRedirect).toHaveBeenCalledExactlyOnceWith({
			url_state: undefined,
			redirectMethod: "replace",
		});
	},
);

it("shows a friendly timeout with manual retry for a code without state", async () => {
	vi.useFakeTimers();
	harness.auth.error = undefined;
	window.history.replaceState(null, "", "/callback?code=unmatched-code");
	await render();
	await act(async () => vi.advanceTimersByTime(8000));
	expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
	expect(container.textContent).toContain("Login expired, please try again.");
	expect(button("Try Again")).toBeDefined();
});

it("clears the retry guard and callback parameters on validated success", async () => {
	sessionStorage.setItem(RECOVERY_KEY, "1");
	saveReturnUrl("/library");
	harness.auth.error = undefined;
	harness.auth.isAuthenticated = true;
	harness.auth.user = {
		url_state: "/join?appId=app-1#accept",
	} as AuthContextProps["user"];
	await render();
	expect(sessionStorage.getItem(RECOVERY_KEY)).toBeNull();
	expect(window.location.search).toBe("");
	expect(consumeReturnUrl()).toBeNull();
	expect(harness.router.replace).toHaveBeenCalledExactlyOnceWith(
		"/join?appId=app-1#accept",
	);
	expect(postMessage).toHaveBeenCalledExactlyOnceWith({ type: "AUTH_SUCCESS" });
	expect(harness.auth.signinRedirect).not.toHaveBeenCalled();
});

it.each(["https://evil.example", callbackUrl])(
	"uses the saved destination instead of successful user return state %s",
	async (returnUrl) => {
		saveReturnUrl("/library");
		harness.auth.error = undefined;
		harness.auth.isAuthenticated = true;
		harness.auth.user = {
			url_state: returnUrl,
		} as AuthContextProps["user"];
		await render();
		expect(harness.router.replace).toHaveBeenCalledExactlyOnceWith("/library");
	},
);

it("replaces the failed callback with home and resets recovery", async () => {
	sessionStorage.setItem(RECOVERY_KEY, "1");
	await render();
	await act(async () => button("Return Home").click());
	expect(harness.router.replace).toHaveBeenCalledExactlyOnceWith("/");
	expect(sessionStorage.getItem(RECOVERY_KEY)).toBeNull();
});
