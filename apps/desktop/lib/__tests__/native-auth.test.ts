import { User, UserManager, type StateStore } from "oidc-client-ts";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
	isTauri: () => true,
}));
import { NativeUserStore } from "../native-auth";

const settings = {
	authority: "https://issuer.test",
	client_id: "client",
	redirect_uri: "flow-like://callback",
	metadata: {
		issuer: "https://issuer.test",
		token_endpoint: "https://issuer.test/token",
	},
	automaticSilentRenew: false,
};
const serializedUser = new User({
	access_token: "first",
	refresh_token: "refresh-first",
	token_type: "Bearer",
	expires_at: 1,
	profile: {
		sub: "alice",
		iss: settings.authority,
		aud: "client",
		exp: 1,
		iat: 0,
	},
}).toStorageString();
const empty = () => ({
	generation: "initial",
	initialized: false,
	user: null as string | null,
	authority: null as string | null,
	clientId: null as string | null,
});
let saved = empty();
let legacy: StateStore;
let store: NativeUserStore;
let manager: UserManager;

function publicSnapshot() {
	const user = saved.user ? User.fromStorageString(saved.user) : undefined;
	if (user) user.refresh_token = undefined;
	return { ...saved, user: user?.toStorageString() ?? null };
}

beforeEach(() => {
	saved = empty();
	legacy = {
		get: vi.fn(async () => serializedUser),
		set: vi.fn(),
		remove: vi.fn(async () => serializedUser),
		getAllKeys: vi.fn(async () => []),
	};
	mocks.invoke
		.mockReset()
		.mockImplementation(
			async (command: string, args: Record<string, unknown>) => {
				if (command === "native_auth_read") return publicSnapshot();
				if (command === "begin_login") return;
				if (command === "native_auth_logout") {
					saved = { ...empty(), generation: "signed-out", initialized: true };
					return;
				}
				if (command === "native_auth_write") {
					if (args.generation !== saved.generation)
						throw "The authentication session changed";
					saved = {
						generation: "signed-in",
						initialized: true,
						user: args.user as string,
						authority: settings.authority,
						clientId: "client",
					};
					return publicSnapshot();
				}
				if (command === "native_auth_clear") {
					if (args.generation !== saved.generation)
						throw "The authentication session changed";
					saved = { ...empty(), generation: "signed-out", initialized: true };
					return publicSnapshot();
				}
				if (command === "native_auth_refresh") {
					const user = User.fromStorageString(saved.user!);
					user.access_token = "renewed";
					user.refresh_token = "refresh-rotated";
					user.expires_at = Math.floor(Date.now() / 1000) + 3600;
					saved.user = user.toStorageString();
					return publicSnapshot();
				}
				throw new Error(`Unexpected command ${command}`);
			},
		);
	store = new NativeUserStore("profile", "https://hub.test", settings, legacy);
	manager = new UserManager({ ...settings, userStore: store });
	store.bind(manager);
});

describe("native login grant ownership", () => {
	test("concurrent startup migrates once and erases browser credentials after commit", async () => {
		const users = await Promise.all([store.get("user"), store.get("user")]);
		expect(users).toEqual([publicSnapshot().user, publicSnapshot().user]);
		expect(JSON.parse(users[0]!).refresh_token).toBeUndefined();
		expect(
			mocks.invoke.mock.calls.filter(
				([command]) => command === "native_auth_write",
			),
		).toHaveLength(1);
		expect(legacy.remove).toHaveBeenCalledTimes(1);
	});

	test("interactive sign-in and logout are owned by the selected native profile", async () => {
		await store.get("user");
		await manager.signinRedirect();
		expect(mocks.invoke).toHaveBeenCalledWith("begin_login", {
			profileId: "profile",
		});
		await manager.signoutRedirect();
		expect(mocks.invoke).toHaveBeenCalledWith("native_auth_logout", {
			profileId: "profile",
		});
		expect(await manager.getUser()).toBeNull();
	});

	test("a failed secure-storage commit retains the only browser grant for recovery", async () => {
		mocks.invoke.mockImplementation(async (command) => {
			if (command === "native_auth_read") return empty();
			throw new Error("Credential store locked");
		});
		await expect(store.get("user")).rejects.toThrow("Credential store locked");
		expect(legacy.remove).not.toHaveBeenCalled();
	});

	test("a logout tombstone prevents another window from migrating stale credentials", async () => {
		await store.get("user");
		await store.remove("user");
		const nextWindow = new NativeUserStore(
			"profile",
			"https://hub.test",
			settings,
			legacy,
		);
		expect(await nextWindow.get("user")).toBeNull();
		expect(legacy.get).toHaveBeenCalledTimes(1);
	});

	test("silent renewal uses native rotation without exporting the refresh grant", async () => {
		await store.get("user");
		const loaded = vi.fn();
		manager.events.addUserLoaded(loaded);
		const user = await manager.signinSilent();
		expect(user?.access_token).toBe("renewed");
		expect(user?.refresh_token).toBeUndefined();
		expect(User.fromStorageString(saved.user!).refresh_token).toBe("refresh-rotated");
		expect(loaded).toHaveBeenCalledWith(user);
		expect(mocks.invoke).toHaveBeenCalledWith("native_auth_refresh", {
			profileId: "profile",
			hub: "https://hub.test",
			subject: "alice",
			generation: "signed-in",
			rejectedToken: "first",
		});
	});

	test("a changed identity-provider configuration cannot load the previous login", async () => {
		await store.get("user");
		const changed = new NativeUserStore(
			"profile",
			"https://hub.test",
			{ ...settings, client_id: "another-client" },
			legacy,
		);
		expect(await changed.get("user")).toBeNull();
	});

	test("a stale window cannot remove a newer login", async () => {
		await store.get("user");
		saved.generation = "another-login";
		await expect(store.remove("user")).rejects.toBe(
			"The authentication session changed",
		);
		expect(saved.user).toBe(serializedUser);
	});
});
