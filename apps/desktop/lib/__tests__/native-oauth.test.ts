import { beforeEach, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	backend: null as any,
	tokens: [] as unknown[],
	consents: [] as unknown[],
	setConsent: vi.fn(async () => undefined),
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
	isTauri: () => true,
}));
vi.mock("@flow-like/flow-like-ui", async () => ({
	...(await import("../../../../packages/ui/lib/oauth/scope")),
	configureOAuthSession: vi.fn(),
	useBackendStore: {
		getState: () => ({ backend: mocks.backend }),
		subscribe: vi.fn(),
	},
	oauthTokenStore: {
		setToken: vi.fn(),
		replaceToken: vi.fn(),
		deleteToken: vi.fn(),
	},
	oauthConsentStore: { setConsent: mocks.setConsent },
	oauthDB: {
		scopedTokens: {
			where: () => ({ equals: () => ({ toArray: async () => mocks.tokens }) }),
		},
		scopedConsents: {
			where: () => ({
				equals: () => ({ toArray: async () => mocks.consents }),
			}),
		},
	},
}));
import { oauthConsentStore, syncNativeOAuth } from "../oauth-db";
import { oauthSessionKey } from "../../../../packages/ui/lib/oauth/scope";

beforeEach(() => {
	mocks.backend = {
		profile: { id: "one", hub: "https://hub.test" },
		auth: { user: { profile: { sub: "alice", iss: "issuer" } } },
	};
	mocks.invoke
		.mockReset()
		.mockImplementation(async (command: string) =>
			command === "native_auth_read"
				? {
						generation: "g1",
						user: JSON.stringify({ profile: { sub: "alice", iss: "issuer" } }),
					}
				: undefined,
		);
	mocks.tokens = [
		{ providerId: "provider", refresh_token: "fl_oauth_connection" },
	];
	mocks.consents = [{ appId: "app", providerId: "provider", scopes: ["read"] }];
	mocks.setConsent.mockReset().mockResolvedValue(undefined);
});

test("consent is durably synchronized to its profile before completion", async () => {
	await oauthConsentStore.setConsent("app", "provider", ["read"]);
	expect(mocks.setConsent).toHaveBeenCalledWith("app", "provider", ["read"]);
	expect(mocks.invoke).toHaveBeenLastCalledWith("native_oauth_sync", {
		profileId: "one",
		hub: "https://hub.test",
		generation: "g1",
		snapshot: { tokens: mocks.tokens, consents: mocks.consents },
		previous: { tokens: mocks.tokens, consents: mocks.consents },
		initializeOnly: false,
	});
});

test("account change cannot copy previous account grants into native storage", async () => {
	mocks.invoke.mockResolvedValue({
		generation: "g2",
		user: JSON.stringify({ profile: { sub: "bob" } }),
	});
	await syncNativeOAuth();
	expect(mocks.invoke).toHaveBeenCalledTimes(1);
});

test("persistence failures reach the caller and the next write can recover", async () => {
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "native_auth_read")
			return {
				generation: "g1",
				user: JSON.stringify({ profile: { sub: "alice", iss: "issuer" } }),
			};
		throw new Error("credential store locked");
	});
	await expect(syncNativeOAuth()).rejects.toThrow("credential store locked");
	mocks.invoke.mockImplementation(async (command: string) =>
		command === "native_auth_read"
			? {
					generation: "g1",
					user: JSON.stringify({ profile: { sub: "alice", iss: "issuer" } }),
				}
			: undefined,
	);
	await expect(syncNativeOAuth()).resolves.toBeUndefined();
});

test("same-account desktop profiles have separate keys and web keeps its format", () => {
	const session = {
		hub: "https://hub.test",
		subject: "alice",
		issuer: "issuer",
	};
	expect(oauthSessionKey({ ...session, profileId: "one" })).not.toBe(
		oauthSessionKey({ ...session, profileId: "two" }),
	);
	expect(JSON.parse(oauthSessionKey(session))).toEqual([
		session.hub,
		session.issuer,
		session.subject,
	]);
});

test("relogin during a browser mutation cannot publish into the new native session", async () => {
	mocks.setConsent.mockImplementationOnce(async () => {
		mocks.invoke.mockImplementation(async (command: string) =>
			command === "native_auth_read"
				? {
						generation: "g2",
						user: JSON.stringify({ profile: { sub: "alice", iss: "issuer" } }),
					}
				: undefined,
		);
	});
	await expect(
		oauthConsentStore.setConsent("app", "provider", ["read"]),
	).rejects.toThrow("authentication session changed");
	const writes = mocks.invoke.mock.calls.filter(
		([command]) => command === "native_oauth_sync",
	);
	expect(writes).toHaveLength(1);
	expect(writes[0][1].initializeOnly).toBe(true);
});

test("a profile change during persistence prevents the browser mutation", async () => {
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "native_oauth_sync") mocks.backend.profile.id = "two";
		return command === "native_auth_read"
			? {
					generation: "g1",
					user: JSON.stringify({ profile: { sub: "alice", iss: "issuer" } }),
				}
			: undefined;
	});
	await expect(
		oauthConsentStore.setConsent("app", "provider", ["read"]),
	).rejects.toThrow("profile or account changed");
	expect(mocks.setConsent).not.toHaveBeenCalled();
});
