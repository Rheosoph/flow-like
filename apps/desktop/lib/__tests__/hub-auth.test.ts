import {
	InMemoryWebStorage,
	User,
	UserManager,
	WebStorageStateStore,
} from "oidc-client-ts";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetch: vi.fn() }));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));

import { fetcher, streamFetcher } from "../api";
import {
	bindHubAuth,
	hubAuthOrigin,
	renewHubAuth,
	setHubAuthActive,
	withHubAuth,
} from "../hub-auth";

const profile = { hub: "https://first-hub.test", secure: true } as never;
const managers: UserManager[] = [];
const user = (token: string, expiresIn = 3600, subject = "alice") =>
	new User({
		access_token: token,
		refresh_token: `refresh-${token}`,
		token_type: "Bearer",
		expires_at: Math.floor(Date.now() / 1000) + expiresIn,
		profile: {
			sub: subject,
			iss: "https://issuer.test",
			aud: "client",
			exp: Math.floor(Date.now() / 1000) + expiresIn,
			iat: Math.floor(Date.now() / 1000),
		},
	});

function authSession(current = user("first")) {
	const manager = new UserManager({
		authority: "https://issuer.test",
		client_id: "client",
		redirect_uri: "https://app.test/callback",
		automaticSilentRenew: false,
		userStore: new WebStorageStateStore({ store: new InMemoryWebStorage() }),
		stateStore: new WebStorageStateStore({ store: new InMemoryWebStorage() }),
	});
	managers.push(manager);
	const signinSilent = vi.fn(async () => user("renewed"));
	const removeUser = vi.fn(() => manager.removeUser());
	const signinRedirect = vi.fn(async () => undefined);
	const auth = {
		user: current,
		events: manager.events,
		signinSilent,
		removeUser,
		signinRedirect,
		isAuthenticated: !current.expired,
	} as unknown as AuthContextProps;
	bindHubAuth(auth, "https://first-hub.test");
	return { auth, signinSilent, manager, removeUser, signinRedirect };
}

beforeEach(() => {
	vi.stubGlobal("navigator", { onLine: true });
	mocks.fetch
		.mockReset()
		.mockImplementation(async () => new Response("{}", { status: 200 }));
});
afterEach(async () => {
	for (const manager of managers.splice(0)) await manager.events.unload();
	vi.unstubAllGlobals();
});

describe("desktop Hub request authentication", () => {
	test("keeps the bound token origin while a session is inactive or rebinding is refused", () => {
		const { auth } = authSession();
		expect(hubAuthOrigin(auth)).toBe("https://first-hub.test");
		setHubAuthActive(auth, false);
		expect(hubAuthOrigin(auth)).toBe("https://first-hub.test");
		expect(bindHubAuth(auth, "https://second-hub.test")).toBe(false);
		expect(hubAuthOrigin(auth)).toBe("https://first-hub.test");
		expect(hubAuthOrigin(undefined)).toBeUndefined();
		expect(
			hubAuthOrigin({
				...auth,
				events: undefined,
			} as unknown as AuthContextProps),
		).toBeUndefined();
	});

	test("an explicit provider refusal starts one interactive login for concurrent requests", async () => {
		const { auth, signinSilent, removeUser, signinRedirect } = authSession(
			user("expired", -1),
		);
		signinSilent.mockRejectedValue({ error: "invalid_grant" });
		const attempts = await Promise.allSettled([
			renewHubAuth(auth),
			renewHubAuth(auth),
		]);
		expect(attempts.every((attempt) => attempt.status === "rejected")).toBe(
			true,
		);
		expect(signinSilent).toHaveBeenCalledTimes(1);
		expect(removeUser).toHaveBeenCalledTimes(1);
		expect(signinRedirect).toHaveBeenCalledTimes(1);
	});

	test("a detached Hub session cannot open its login when an old renewal fails", async () => {
		const { auth, signinSilent, removeUser, signinRedirect } = authSession(
			user("expired", -1),
		);
		let reject!: (error: unknown) => void;
		signinSilent.mockImplementation(
			() =>
				new Promise((_resolve, fail) => {
					reject = fail;
				}),
		);
		const attempt = renewHubAuth(auth);
		await vi.waitFor(() => expect(signinSilent).toHaveBeenCalledTimes(1));
		setHubAuthActive(auth, false);
		reject({ error: "invalid_grant" });
		await expect(attempt).rejects.toMatchObject({ error: "invalid_grant" });
		expect(removeUser).not.toHaveBeenCalled();
		expect(signinRedirect).not.toHaveBeenCalled();
	});

	test("never attaches another hub's bearer to JSON or streamed requests", async () => {
		const { auth, signinSilent } = authSession();
		const otherProfile = { hub: "https://second-hub.test" } as never;
		await fetcher(otherProfile, "user/info", undefined, auth);
		expect(
			new Headers(mocks.fetch.mock.calls[0][1].headers).has("Authorization"),
		).toBe(false);
		await expect(
			fetcher(otherProfile, "apps/app/board", undefined, auth),
		).rejects.toThrow("Authentication token required");
		await expect(
			streamFetcher(otherProfile, "apps/app/invoke", { method: "POST" }, auth),
		).rejects.toThrow("Authentication token required");
		expect(mocks.fetch).toHaveBeenCalledTimes(1);
		expect(signinSilent).not.toHaveBeenCalled();
	});

	test("an existing manager cannot be rebound to a different Hub", async () => {
		const { auth } = authSession();
		expect(bindHubAuth(auth, "https://second-hub.test")).toBe(false);
		await fetcher(
			{ hub: "https://second-hub.test" } as never,
			"user/info",
			undefined,
			auth,
		);
		expect(
			new Headers(mocks.fetch.mock.calls[0][1].headers).has("Authorization"),
		).toBe(false);
	});

	test("concurrent expired-token requests share renewal and send its returned token immediately", async () => {
		const { auth, signinSilent } = authSession(user("expired", -1));
		await Promise.all([
			fetcher(profile, "user/info", undefined, auth),
			fetcher(profile, "profile", undefined, auth),
		]);
		expect(signinSilent).toHaveBeenCalledTimes(1);
		expect(mocks.fetch).toHaveBeenCalledTimes(2);
		for (const [, init] of mocks.fetch.mock.calls)
			expect(new Headers(init.headers).get("Authorization")).toBe(
				"Bearer renewed",
			);
		expect(auth.user?.access_token).toBe("expired");
	});

	test("late concurrent 401s reuse the completed renewal and each retry only once", async () => {
		const { auth, signinSilent } = authSession();
		const refused: Array<(response: Response) => void> = [];
		mocks.fetch.mockImplementation((_url, init) =>
			new Headers(init.headers).get("Authorization") === "Bearer first"
				? new Promise<Response>((resolve) => refused.push(resolve))
				: Promise.resolve(new Response("{}")),
		);
		const first = fetcher(profile, "user/info", undefined, auth);
		const second = fetcher(profile, "profile", undefined, auth);
		await vi.waitFor(() => expect(refused).toHaveLength(2));
		refused[0](new Response("expired", { status: 401 }));
		await first;
		refused[1](new Response("expired", { status: 401 }));
		await second;
		expect(signinSilent).toHaveBeenCalledTimes(1);
		expect(mocks.fetch).toHaveBeenCalledTimes(4);
		mocks.fetch.mockImplementation(
			async () => new Response("refused", { status: 401 }),
		);
		await expect(
			fetcher(profile, "user/info", undefined, auth),
		).rejects.toMatchObject({ status: 401 });
		expect(mocks.fetch).toHaveBeenCalledTimes(6);
	});

	test("a transient refresh failure keeps the session available for the next attempt", async () => {
		const { auth, signinSilent } = authSession(user("expired", -1));
		signinSilent.mockRejectedValueOnce(new TypeError("Failed to fetch"));
		await expect(
			fetcher(profile, "user/info", undefined, auth),
		).rejects.toThrow("Network unavailable");
		expect(mocks.fetch).not.toHaveBeenCalled();
		await fetcher(profile, "user/info", undefined, auth);
		expect(signinSilent).toHaveBeenCalledTimes(2);
		expect(
			new Headers(mocks.fetch.mock.calls[0][1].headers).get("Authorization"),
		).toBe("Bearer renewed");
	});

	test("a 401 from an earlier account does not retry with a newly signed-in account", async () => {
		const { auth, signinSilent, manager } = authSession();
		let refuse!: (response: Response) => void;
		const send = vi.fn(
			() =>
				new Promise<Response>((resolve) => {
					refuse = resolve;
				}),
		);
		const request = withHubAuth(profile, auth, send);
		await manager.events.unload();
		await manager.events.load(user("bob-token", 3600, "bob"));
		refuse(new Response("unauthorized", { status: 401 }));
		expect((await request).status).toBe(401);
		expect(send).toHaveBeenCalledTimes(1);
		expect(signinSilent).not.toHaveBeenCalled();
	});

	test("streaming POSTs renew and retry before delivering any stream messages", async () => {
		const { auth, signinSilent } = authSession();
		mocks.fetch
			.mockResolvedValueOnce(new Response("expired", { status: 401 }))
			.mockResolvedValueOnce(
				new Response('event: completed\ndata: {"done":true}\n\n', {
					headers: { "Content-Type": "text/event-stream" },
				}),
			);
		const onMessage = vi.fn();
		await streamFetcher(
			profile,
			"apps/app/invoke",
			{ method: "POST" },
			auth,
			onMessage,
		);
		expect(signinSilent).toHaveBeenCalledTimes(1);
		expect(onMessage).toHaveBeenCalledExactlyOnceWith({ done: true });
		expect(
			new Headers(mocks.fetch.mock.calls[1][1].headers).get("Authorization"),
		).toBe("Bearer renewed");
	});
});
