import "fake-indexeddb/auto";
import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import {
	configureOAuthSession,
	oauthConsentStore,
	oauthDB,
	oauthTokenStore,
} from "../../db/oauth-db";
import {
	getOAuthTokensForProviders,
	storedTokenToBackendFormat,
} from "./helpers";
import { type OAuthSession, localOAuthSession } from "./scope";
import { OAuthRefreshError, createOAuthService } from "./service";
import type { IOAuthProvider, IOAuthRuntime, IStoredOAuthToken } from "./types";

const provider: IOAuthProvider = {
	id: "example",
	name: "Example",
	auth_url: "https://provider.test/authorize",
	token_url: "https://provider.test/token",
	client_id: "client-one",
	scopes: ["read", "write"],
	pkce_required: true,
	requires_secret_proxy: true,
	use_device_flow: false,
	device_auth_url: "https://provider.test/device",
};
let session: OAuthSession | undefined;
let calls: Array<{ url: string; body: string; binding?: unknown }>;
let transport: IOAuthRuntime["httpPost"];
let runtime: IOAuthRuntime;
function response(body: unknown, status = 200) {
	return new Response(JSON.stringify(body), { status });
}
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => {
		resolve = r;
	});
	return { promise, resolve };
}
function service(store = oauthTokenStore) {
	return createOAuthService({
		runtime,
		tokenStore: store,
		platform: "web-dev",
	});
}
async function seed(overrides: Partial<IStoredOAuthToken> = {}) {
	const token: IStoredOAuthToken = {
		providerId: provider.id,
		binding: await oauthTokenStore.getBinding!(provider),
		access_token: "old-access",
		refresh_token: "fl_oauth_connection",
		expires_at: 0,
		scopes: ["read", "write"],
		storedAt: 1,
		...overrides,
	};
	await oauthTokenStore.setToken(token);
	return token;
}
beforeEach(async () => {
	session = {
		hub: "https://hub.test",
		subject: crypto.randomUUID(),
		issuer: "https://issuer.test",
	};
	configureOAuthSession(() => session);
	await Promise.all([
		oauthDB.scopedTokens.clear(),
		oauthDB.scopedConsents.clear(),
		oauthDB.scopedPendingAuth.clear(),
		oauthDB.refreshLeases.clear(),
	]);
	calls = [];
	transport = async () =>
		response({
			access_token: "new-access",
			refresh_token: "fl_oauth_connection",
			expires_in: 3600,
			scope: "read",
		});
	runtime = {
		openUrl: async () => {},
		httpPost: async (url, body, headers, binding) => {
			calls.push({ url, body, binding });
			return transport(url, body, headers, binding);
		},
		httpGet: async () => response({}),
	};
});
afterAll(async () => {
	await oauthDB.delete();
});

describe("OAuth credential ownership", () => {
	it("isolates credentials and consent by Hub, user, issuer and client", async () => {
		const initial = session!;
		const token = await seed();
		await oauthConsentStore.setConsent("app", provider.id, ["read"]);
		for (const other of [
			{ ...initial, hub: "https://other.test" },
			{ ...initial, subject: "other" },
			{ ...initial, issuer: "https://other-issuer.test" },
		]) {
			session = other;
			expect(
				await oauthTokenStore.getToken(provider.id, provider),
			).toBeUndefined();
			expect(await oauthConsentStore.hasConsent("app", provider.id)).toBe(
				false,
			);
		}
		session = initial;
		expect(
			(await oauthTokenStore.getToken(provider.id, provider))?.access_token,
		).toBe(token.access_token);
		expect(await oauthConsentStore.hasConsent("app", provider.id)).toBe(true);
		expect(
			await oauthTokenStore.getToken(provider.id, {
				...provider,
				client_id: "other-client",
			}),
		).toBeUndefined();
		expect(await oauthConsentStore.hasConsent("app", provider.id)).toBe(false);
		expect(
			await oauthTokenStore.getToken(provider.id, {
				...provider,
				token_url: "https://other.test/token",
			}),
		).toBeUndefined();
	});
	it("rejects unscoped legacy credentials and writes after account switching", async () => {
		await expect(
			oauthTokenStore.setToken({
				providerId: provider.id,
				access_token: "legacy",
				scopes: [],
				storedAt: 1,
			}),
		).rejects.toThrow("changed");
		const token = await seed();
		session = { ...session!, subject: "other" };
		await expect(oauthTokenStore.setToken(token)).rejects.toThrow("changed");
		expect(await oauthTokenStore.getAllTokens()).toEqual([]);
	});
	it("claims a pending callback once and expires abandoned authorizations", async () => {
		const auth = service();
		const { state } = await auth.startAuthorization(provider);
		const consumed = await Promise.all([
			oauthTokenStore.consumePendingAuth(state),
			oauthTokenStore.consumePendingAuth(state),
		]);
		expect(consumed.filter(Boolean)).toHaveLength(1);
		await oauthTokenStore.setPendingAuth({
			...consumed.find(Boolean)!,
			state: "expired",
			initiatedAt: Date.now() - 600001,
		});
		expect(await oauthTokenStore.consumePendingAuth("expired")).toBeUndefined();
	});
	it("does not restore an old client configuration from a pending callback", async () => {
		const { state } = await service().startAuthorization(provider);
		await oauthTokenStore.getToken(provider.id, {
			...provider,
			client_id: "replacement",
		});
		expect(await oauthTokenStore.consumePendingAuth(state)).toBeUndefined();
	});
});

describe("signed out of the Hub", () => {
	const direct = { ...provider, requires_secret_proxy: false };
	const rawGrant = () =>
		response({
			access_token: "local-access",
			refresh_token: "provider-refresh",
			expires_in: 3600,
			scope: "read",
		});
	beforeEach(() => {
		session = localOAuthSession("https://hub.test", "profile-a");
		transport = async () => rawGrant();
	});
	it("keeps local grants apart from Hub accounts and other profiles", async () => {
		const local = session;
		const token = await seed({ refresh_token: "provider-refresh" });
		expect(token.binding?.local).toBe(true);
		for (const other of [
			{
				hub: "https://hub.test",
				subject: "",
				issuer: "",
				profileId: "profile-a",
			},
			localOAuthSession("https://hub.test", "profile-b"),
			{
				hub: "https://hub.test",
				subject: "user",
				issuer: "https://issuer.test",
			},
		]) {
			session = other;
			expect(
				await oauthTokenStore.getToken(provider.id, provider),
			).toBeUndefined();
		}
		await expect(oauthTokenStore.setToken(token)).rejects.toThrow("changed");
		session = local;
		expect(
			(await oauthTokenStore.getToken(provider.id, provider))?.access_token,
		).toBe("old-access");
	});
	it("redeems a public client with the provider and never stores a Hub connection", async () => {
		const auth = service();
		const { state } = await auth.startAuthorization(direct);
		const token = await auth.handleCallback(
			`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
			direct,
		);
		expect(calls.map((c) => c.url)).toEqual([direct.token_url]);
		expect(calls[0]?.binding).toBeUndefined();
		expect(token.refresh_token).toBe("provider-refresh");
		expect(storedTokenToBackendFormat(token)).toMatchObject({
			hub_origin: undefined,
			hub_subject: undefined,
			refresh_token: "provider-refresh",
		});
	});
	it("uses the Hub only as the secret proxy, without an account", async () => {
		const auth = service();
		const { state } = await auth.startAuthorization(provider);
		const token = await auth.handleCallback(
			`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
			provider,
		);
		expect(calls).toHaveLength(1);
		expect(calls[0]?.url).toBe("https://hub.test/api/v1/oauth/token/example");
		expect(calls[0]?.binding).toBeUndefined();
		expect(token.refresh_token).toBe("provider-refresh");
		await auth.refreshToken(
			provider,
			await seed({ refresh_token: "provider-refresh" }),
		);
		expect(calls[1]?.url).toBe("https://hub.test/api/v1/oauth/refresh/example");
		expect(calls[1]?.binding).toBeUndefined();
	});
	it("refreshes a public client's local grant with the provider", async () => {
		const token = await seed({ refresh_token: "provider-refresh" });
		const refreshed = await service().refreshToken(direct, token);
		expect(calls.map((c) => c.url)).toEqual([direct.token_url]);
		expect(calls[0]?.binding).toBeUndefined();
		expect(refreshed.access_token).toBe("local-access");
	});
});

describe("OAuth refresh", () => {
	it("shares concurrent refreshes across service instances", async () => {
		const token = await seed();
		const gate = deferred<Response>();
		const entered = deferred<void>();
		transport = async () => {
			entered.resolve();
			return gate.promise;
		};
		const first = service().refreshToken(provider, token);
		await entered.promise;
		const second = service().refreshToken(provider, token);
		gate.resolve(
			response({
				access_token: "rotated",
				refresh_token: "fl_oauth_rotated",
				expires_in: 3600,
				scope: "read",
			}),
		);
		const result = await Promise.all([first, second]);
		expect(calls).toHaveLength(1);
		expect(result[0]).toEqual(result[1]);
		expect(result[0].scopes).toEqual(["read"]);
		expect((await oauthTokenStore.getToken(provider.id))?.refresh_token).toBe(
			"fl_oauth_rotated",
		);
	});
	it("serializes separate store facades through the persisted lease", async () => {
		const token = await seed();
		const gate = deferred<Response>();
		const entered = deferred<void>();
		transport = async () => {
			entered.resolve();
			return gate.promise;
		};
		const first = service({ ...oauthTokenStore }).refreshToken(provider, token);
		await entered.promise;
		const second = service({ ...oauthTokenStore }).refreshToken(
			provider,
			token,
		);
		gate.resolve(response({ access_token: "rotated", expires_in: 3600 }));
		const result = await Promise.all([first, second]);
		expect(calls).toHaveLength(1);
		expect(result[0].access_token).toBe(result[1].access_token);
		expect(await oauthDB.refreshLeases.count()).toBe(0);
	});
	it("routes canonical handles through the owning Hub even for public clients", async () => {
		const publicProvider = { ...provider, requires_secret_proxy: false };
		const token = await seed();
		await service().refreshToken(publicProvider, token);
		expect(calls[0]?.url).toBe("https://hub.test/api/v1/oauth/refresh/example");
		expect(JSON.parse(calls[0]!.body)).toEqual({
			refresh_token: "fl_oauth_connection",
			client_id: provider.client_id,
		});
	});
	it("keeps a replacement connection when an older refresh fails permanently", async () => {
		const token = await seed();
		const gate = deferred<Response>();
		const entered = deferred<void>();
		transport = async () => {
			entered.resolve();
			return gate.promise;
		};
		const result = service().refreshToken(provider, token);
		await entered.promise;
		const replacement = await seed({
			access_token: "reauthorized",
			refresh_token: "fl_oauth_reauthorized",
			storedAt: 2,
			expires_at: Date.now() + 3600000,
		});
		gate.resolve(response({ error: "invalid_grant" }, 400));
		expect((await result).access_token).toBe(replacement.access_token);
		expect((await oauthTokenStore.getToken(provider.id))?.access_token).toBe(
			replacement.access_token,
		);
	});
	it("removes a refused grant but preserves transient errors without requesting consent", async () => {
		const token = await seed();
		transport = async () => response({ error: "oauth_refresh_pending" }, 503);
		const auth = service();
		const consent = vi.fn();
		await expect(
			auth.ensureTokensForProviders([provider], consent),
		).rejects.toMatchObject({ reauthorizationRequired: false });
		expect(consent).not.toHaveBeenCalled();
		expect(await oauthTokenStore.getToken(provider.id)).toBeDefined();
		transport = async () =>
			response({ error: "oauth_reauthorization_required" }, 401);
		await expect(auth.refreshToken(provider, token)).rejects.toBeInstanceOf(
			OAuthRefreshError,
		);
		expect(await oauthTokenStore.getToken(provider.id)).toBeUndefined();
	});
	it("does not save a refresh response into a newly selected account", async () => {
		const token = await seed();
		const gate = deferred<Response>();
		const entered = deferred<void>();
		transport = async () => {
			entered.resolve();
			return gate.promise;
		};
		const result = service().refreshToken(provider, token);
		await entered.promise;
		session = { ...session!, subject: "other" };
		gate.resolve(response({ access_token: "rotated", expires_in: 3600 }));
		await expect(result).rejects.toThrow("changed");
		expect(await oauthTokenStore.getAllTokens()).toEqual([]);
	});
	it("does not treat a successful malformed response as a replacement token", async () => {
		const token = await seed();
		transport = async () => response({ error: "upstream_error" });
		await expect(service().refreshToken(provider, token)).rejects.toThrow(
			"no access_token",
		);
		expect((await oauthTokenStore.getToken(provider.id))?.access_token).toBe(
			"old-access",
		);
	});
});

describe("authorization and revocation", () => {
	it("requests audience, PKCE and offline access and records only granted scopes", async () => {
		const auth = service();
		const { state, authUrl } = await auth.startAuthorization({
			...provider,
			audience: "api.atlassian.com",
		});
		const params = new URL(authUrl).searchParams;
		expect(params.get("audience")).toBe("api.atlassian.com");
		expect(params.get("response_type")).toBe("code");
		expect(params.get("code_challenge_method")).toBe("S256");
		expect(params.get("access_type")).toBe("offline");
		const token = await auth.handleCallback(
			`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
			provider,
		);
		expect(token.scopes).toEqual(["read"]);
	});
	it("adopts raw refresh responses before persisting them and preserves an explicit empty grant", async () => {
		const direct = { ...provider, requires_secret_proxy: false };
		const auth = service();
		const { state } = await auth.startAuthorization(direct);
		transport = async (url) =>
			url.endsWith("/oauth/token/example")
				? response({
						access_token: "direct",
						refresh_token: "provider-rotating",
						expires_in: 3600,
						scope: "",
					})
				: response({
						access_token: "direct",
						refresh_token: "fl_oauth_adopted",
						expires_in: 3600,
						scope: "",
					});
		const token = await auth.handleCallback(
			`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
			direct,
		);
		expect(calls.map((c) => c.url)).toEqual([
			"https://hub.test/api/v1/oauth/token/example",
			"https://hub.test/api/v1/oauth/connection/example",
		]);
		expect(token.refresh_token).toBe("fl_oauth_adopted");
		expect(token.scopes).toEqual([]);
		expect(JSON.parse(calls[1]!.body).client_id).toBe(provider.client_id);
	});
	it("records offline_access from the refresh token, which Entra never lists in scope", async () => {
		const microsoft = { ...provider, scopes: ["openid", "offline_access"] };
		const auth = service();
		const granted = async (refresh_token?: string) => {
			transport = async () =>
				response({
					access_token: "graph",
					refresh_token,
					expires_in: 3600,
					scope: "openid Calendars.ReadWrite",
				});
			const { state } = await auth.startAuthorization(microsoft);
			const token = await auth.handleCallback(
				`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
				microsoft,
			);
			return token.scopes;
		};
		expect(await granted("fl_oauth_connection")).toEqual([
			"openid",
			"Calendars.ReadWrite",
			"offline_access",
		]);
		expect(await granted()).toEqual(["openid", "Calendars.ReadWrite"]);
	});
	it("reports the provider's reason when the Hub cannot redeem a code", async () => {
		const auth = service();
		const { state } = await auth.startAuthorization(provider);
		transport = async () =>
			response(
				{
					error: "oauth_proxy_error",
					error_description:
						"Token exchange failed: invalid_request - AADSTS9002327: Tokens issued for the 'Single-Page Application' client-type may only be redeemed via cross-origin requests.\r\nTrace ID: abc",
				},
				400,
			);
		const failure = await auth
			.handleCallback(
				`https://flow-like.com/thirdparty/callback?state=${state}&code=code`,
				provider,
			)
			.catch((error: Error) => error);
		expect(failure).toBeInstanceOf(Error);
		expect((failure as Error).message).toBe(
			"Token exchange failed: 400 - invalid_request - AADSTS9002327: Tokens issued for the 'Single-Page Application' client-type may only be redeemed via cross-origin requests. Trace ID: abc",
		);
	});
	it("rejects unexpected callback destinations without consuming pending state", async () => {
		const auth = service();
		const { state } = await auth.startAuthorization(provider);
		await expect(
			auth.handleCallback(
				`https://attacker.test/thirdparty/callback?state=${state}&code=code`,
				provider,
			),
		).rejects.toThrow("destination");
		expect(await oauthTokenStore.getPendingAuth(state)).toBeDefined();
		expect(calls).toHaveLength(0);
	});
	it("retains a connection when canonical revocation fails, even without a provider revocation endpoint", async () => {
		const token = await seed();
		transport = async () => response({ error: "unavailable" }, 503);
		await expect(service().revokeToken(provider, token)).rejects.toThrow("503");
		expect(JSON.parse(calls[0]!.body).token).toBe(token.refresh_token);
		expect(await oauthTokenStore.getToken(provider.id)).toBeDefined();
		transport = async () => response({});
		await service().revokeToken(provider, token);
		expect(await oauthTokenStore.getToken(provider.id)).toBeUndefined();
	});
	it("propagates device slow_down and saves the provider's reduced scopes", async () => {
		const auth = service();
		transport = async () =>
			response({
				device_code: "device",
				user_code: "code",
				verification_uri: "https://provider.test/verify",
				expires_in: 600,
				interval: 5,
			});
		await auth.startDeviceAuthorization(provider);
		transport = async () => response({ error: "slow_down" }, 400);
		await expect(
			auth.pollDeviceAuthorization(provider, "device", ["read", "write"]),
		).rejects.toThrow("slow_down");
		transport = async () =>
			response({
				access_token: "device-access",
				refresh_token: "fl_oauth_device",
				expires_in: 3600,
				scope: "read",
			});
		expect(
			(
				await auth.pollDeviceAuthorization(provider, "device", [
					"read",
					"write",
				])
			)?.scopes,
		).toEqual(["read"]);
	});
});

describe("execution grants", () => {
	it.each([
		{
			name: "Google returns full identity scope URLs",
			requestedIdentityScopes: ["email", "profile"],
			grantedIdentityScopes: [
				"https://www.googleapis.com/auth/userinfo.email",
				"https://www.googleapis.com/auth/userinfo.profile",
			],
		},
		{
			name: "Google returns short identity scope names",
			requestedIdentityScopes: [
				"https://www.googleapis.com/auth/userinfo.email",
				"https://www.googleapis.com/auth/userinfo.profile",
			],
			grantedIdentityScopes: ["email", "profile"],
		},
	])(
		"accepts equivalent grants when $name",
		async ({ requestedIdentityScopes, grantedIdentityScopes }) => {
			const apiScope = "https://www.googleapis.com/auth/calendar.readonly";
			const google: IOAuthProvider = {
				...provider,
				id: "google",
				name: "Google",
				auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
				token_url: "https://oauth2.googleapis.com/token",
				scopes: ["openid", ...requestedIdentityScopes],
				merged_scopes: ["openid", ...requestedIdentityScopes, apiScope],
			};
			const granted = ["openid", ...grantedIdentityScopes, apiScope];
			transport = async () =>
				response({
					access_token: "google-access",
					refresh_token: "fl_oauth_google",
					expires_in: 3600,
					scope: granted.join(" "),
				});
			const auth = service();
			const { state } = await auth.startAuthorization(google);
			const token = await auth.handleCallback(
				`flow-like://thirdparty/callback?state=${state}&code=google-code`,
				google,
			);
			expect(token.scopes).toEqual(granted);
			expect(
				(await oauthTokenStore.getToken(google.id, google))?.scopes,
			).toEqual(granted);

			const execution = await getOAuthTokensForProviders(
				[google],
				oauthTokenStore,
			);
			expect(execution.missingProviders).toEqual([]);
			expect(execution.tokens.google.access_token).toBe(token.access_token);
			const status = await auth.getValidTokensForProviders([google]);
			expect(status.missing).toEqual([]);
			expect(status.needsRefresh.size).toBe(0);
			expect(status.valid.get(google.id)?.access_token).toBe(
				token.access_token,
			);
			expect(
				(await auth.ensureTokensForProviders([google])).get(google.id)
					?.access_token,
			).toBe(token.access_token);

			await oauthConsentStore.setConsent(
				"app",
				google.id,
				google.merged_scopes!,
			);
			for (const requiredScopes of [google.merged_scopes!, granted]) {
				expect(
					await oauthConsentStore.hasConsentWithScopes(
						"app",
						google.id,
						requiredScopes,
					),
				).toBe(true);
				expect(
					await oauthConsentStore.getConsentedProviderIdsWithScopes(
						"app",
						new Map([[google.id, requiredScopes]]),
					),
				).toEqual(new Set([google.id]));
			}
		},
	);
	it("accepts Google's canonical identity scopes after refreshing an expired grant", async () => {
		const google = {
			...provider,
			id: "google",
			scopes: ["openid", "email", "profile"],
		};
		await oauthTokenStore.setToken({
			providerId: google.id,
			binding: await oauthTokenStore.getBinding!(google),
			access_token: "expired-google-access",
			refresh_token: "fl_oauth_google",
			expires_at: 0,
			scopes: google.scopes,
			storedAt: 1,
		});
		const granted = [
			"openid",
			"https://www.googleapis.com/auth/userinfo.email",
			"https://www.googleapis.com/auth/userinfo.profile",
		];
		transport = async () =>
			response({
				access_token: "refreshed-google-access",
				refresh_token: "fl_oauth_google",
				expires_in: 3600,
				scope: granted.join(" "),
			});
		const valid = await service().ensureTokensForProviders([google]);
		expect(valid.get(google.id)?.scopes).toEqual(granted);
		expect(valid.get(google.id)?.access_token).toBe("refreshed-google-access");
		expect(calls).toHaveLength(1);
	});
	it("stamps canonical handles with their Hub owner and preserves access-only credentials", async () => {
		const token = await seed();
		expect(storedTokenToBackendFormat(token)).toMatchObject({
			hub_origin: session!.hub,
			hub_subject: session!.subject,
			expires_at: 0,
		});
		expect(() =>
			storedTokenToBackendFormat({ ...token, binding: undefined }),
		).toThrow("Hub owner");
		expect(
			storedTokenToBackendFormat({
				...token,
				binding: undefined,
				refresh_token: undefined,
			}),
		).toMatchObject({ access_token: "old-access", expires_at: 0 });
	});
	it("does not remember scopes the provider refused or send that token for execution", async () => {
		await seed({ scopes: ["read"], expires_at: Date.now() + 3600000 });
		await expect(
			oauthConsentStore.setConsent("app", provider.id, ["read", "write"]),
		).rejects.toThrow("required scopes");
		expect(await oauthConsentStore.hasConsent("app", provider.id)).toBe(false);
		const result = await getOAuthTokensForProviders(
			[provider],
			oauthTokenStore,
		);
		expect(result.tokens).toEqual({});
		expect(result.missingProviders).toEqual([provider]);
	});
	it("surfaces temporary refresh failures to execution without deleting the grant", async () => {
		await seed();
		transport = async () => response({ error: "oauth_refresh_pending" }, 503);
		await expect(
			getOAuthTokensForProviders([provider], oauthTokenStore, {
				refreshToken: service().refreshToken,
			}),
		).rejects.toMatchObject({ reauthorizationRequired: false });
		expect(await oauthTokenStore.getToken(provider.id)).toBeDefined();
	});
});

it("does not combine execution tokens from different accounts across asynchronous reads", async () => {
	await seed({ expires_at: Date.now() + 3600000 });
	const original = oauthTokenStore.getToken;
	const store = {
		...oauthTokenStore,
		getToken: async (id: string, config?: IOAuthProvider) => {
			const token = await original(id, config);
			session = { ...session!, subject: "bob" };
			return token;
		},
	};
	await expect(getOAuthTokensForProviders([provider], store)).rejects.toThrow(
		"account or provider changed",
	);
});

it("exchanges public custom-client authorizations through the Hub's canonical owner", async () => {
	const custom = {
		...provider,
		client_id: "custom-public-client",
		requires_secret_proxy: false,
	};
	const auth = service();
	const { state } = await auth.startAuthorization(custom);
	await auth.handleCallback(
		`https://flow-like.com/thirdparty/callback?state=${state}&code=fresh-code`,
		custom,
	);
	expect(calls[0]?.url).toBe("https://hub.test/api/v1/oauth/token/example");
	expect(JSON.parse(calls[0]!.body)).toMatchObject({
		client_id: "custom-public-client",
		code: "fresh-code",
	});
	transport = async () =>
		response({
			device_code: "custom-device",
			user_code: "code",
			verification_uri: "https://provider.test/verify",
			expires_in: 600,
			interval: 5,
		});
	await auth.startDeviceAuthorization(custom);
	expect(calls[1]?.url).toBe(
		"https://hub.test/api/v1/oauth/device/start/example",
	);
	expect(JSON.parse(calls[1]!.body).client_id).toBe(custom.client_id);
	transport = async () =>
		response({
			access_token: "device",
			refresh_token: "fl_oauth_custom",
			scope: "read",
			expires_in: 3600,
		});
	await auth.pollDeviceAuthorization(custom, "custom-device", ["read"]);
	expect(calls[2]?.url).toBe(
		"https://hub.test/api/v1/oauth/device/poll/example",
	);
	expect(JSON.parse(calls[2]!.body).client_id).toBe(custom.client_id);
});
