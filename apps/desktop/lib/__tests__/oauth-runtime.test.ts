import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { oauthBinding } from "../../../../packages/ui/lib/oauth/scope";
import type { IOAuthProvider } from "../../../../packages/ui/lib/oauth/types";
const state = vi.hoisted(() => ({ backend: null as unknown, fetch: vi.fn() }));
vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await import("../../../../packages/ui/lib/oauth/scope")),
	useBackendStore: { getState: () => ({ backend: state.backend }) },
}));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: state.fetch }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
import { bindHubAuth } from "../hub-auth";
import { tauriOAuthRuntime } from "../tauri-oauth-runtime";
const owner = {
	hub: "https://hub.test",
	subject: "alice",
	issuer: "https://issuer.test",
};
const binding = oauthBinding(owner, {
	id: "provider",
	client_id: "client",
	auth_url: "https://provider.test/authorize",
	token_url: "https://provider.test/token",
} as IOAuthProvider);
type User = NonNullable<AuthContextProps["user"]>;
const user = (sub = "alice", expired = false) =>
	({
		access_token: expired ? `expired-${sub}` : `access-${sub}`,
		id_token: `id-${sub}`,
		profile: { sub, iss: owner.issuer },
		expired,
		refresh_token: "refresh",
	}) as User;
function context(initial = user()) {
	const listeners = new Set<(user: User) => void>();
	const auth = {
		user: initial,
		isAuthenticated: !initial.expired,
		signinSilent: vi.fn(async () => user()),
		events: {
			addUserLoaded: (fn: (user: User) => void) => listeners.add(fn),
			addUserUnloaded: () => {},
		},
	} as unknown as AuthContextProps;
	bindHubAuth(auth, owner.hub);
	state.backend = { profile: { hub: owner.hub, secure: true }, auth };
	return {
		auth,
		load: (next: User) => {
			for (const fn of listeners) fn(next);
		},
	};
}
const endpoint = "https://hub.test/api/v1/oauth/refresh/provider";
const request = () => tauriOAuthRuntime.httpPost(endpoint, "{}", {}, binding);
beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("fetch", state.fetch);
	state.fetch.mockResolvedValue(new Response("{}"));
	context();
});
afterEach(() => vi.unstubAllGlobals());
it("sends the access token to the matching Hub and never its ID token", async () => {
	await request();
	expect(state.fetch).toHaveBeenCalledOnce();
	expect(
		new Headers(state.fetch.mock.calls[0][1].headers).get("Authorization"),
	).toBe("Bearer access-alice");
});
it("rejects wrong Hub or switched user before sending provider credentials", async () => {
	await expect(
		tauriOAuthRuntime.httpPost(
			"https://other.test/api/v1/oauth/refresh/provider",
			"{}",
			{},
			binding,
		),
	).rejects.toThrow("Hub");
	context(user("bob"));
	await expect(request()).rejects.toThrow("Hub");
	expect(state.fetch).not.toHaveBeenCalled();
});
it("keeps the Hub session off direct provider HTTP requests", async () => {
	await tauriOAuthRuntime.httpPost(
		"https://provider.test/token",
		"grant_type=authorization_code",
		{ "Content-Type": "application/x-www-form-urlencoded" },
		binding,
	);
	expect(
		new Headers(state.fetch.mock.calls[0][1].headers).has("Authorization"),
	).toBe(false);
});
it("renews an expired Hub user even when react reports unauthenticated", async () => {
	const { auth } = context(user("alice", true));
	await request();
	expect(auth.signinSilent).toHaveBeenCalledOnce();
	expect(state.fetch).toHaveBeenCalledOnce();
	expect(
		new Headers(state.fetch.mock.calls[0][1].headers).get("Authorization"),
	).toBe("Bearer access-alice");
});
it("does not send an adoption or refresh after an account switch during Hub renewal", async () => {
	const { auth, load } = context(user("alice", true));
	let finish!: (value: User) => void;
	let entered!: () => void;
	const started = new Promise<void>((resolve) => {
		entered = resolve;
	});
	vi.mocked(auth.signinSilent).mockImplementation(() => {
		entered();
		return new Promise((resolve) => {
			finish = resolve;
		});
	});
	const pending = request();
	await started;
	load(user("bob"));
	finish(user("alice"));
	await expect(pending).rejects.toThrow("Sign in again");
	expect(state.fetch).not.toHaveBeenCalled();
});
it("sends direct provider requests without an Origin", async () => {
	await tauriOAuthRuntime.httpPost(
		"https://provider.test/token",
		"grant_type=authorization_code",
		{ "Content-Type": "application/x-www-form-urlencoded" },
	);
	expect(new Headers(state.fetch.mock.calls[0][1].headers).get("Origin")).toBe(
		"",
	);
});
it("lets a signed-out profile use only its own Hub's secret proxy, without credentials", async () => {
	state.backend = { profile: { hub: owner.hub, secure: true } };
	await tauriOAuthRuntime.httpPost(endpoint, "{}", {});
	expect(
		new Headers(state.fetch.mock.calls[0][1].headers).has("Authorization"),
	).toBe(false);
	await expect(
		tauriOAuthRuntime.httpPost(
			"https://other.test/api/v1/oauth/refresh/provider",
			"{}",
			{},
		),
	).rejects.toThrow("Hub");
	await expect(request()).rejects.toThrow("Hub");
	expect(state.fetch).toHaveBeenCalledOnce();
});
