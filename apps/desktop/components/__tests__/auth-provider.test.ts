// @vitest-environment happy-dom
import { User } from "oidc-client-ts";
import { act, createElement, useEffect, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import { type AuthContextProps, useAuth } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

interface ConfigRequest {
	hub: string;
	resolve: (value: unknown) => void;
	reject: (error: unknown) => void;
}

const mocks = vi.hoisted(() => ({
	backend: { userState: {}, teamState: {}, appState: {} },
	profile: { id: "profile-a", hub: "api.flow-like.com", secure: true },
	configRequests: [] as ConfigRequest[],
	invalidate: async () => undefined,
	openUrl: vi.fn().mockResolvedValue(undefined),
	signInLink: undefined as string | undefined,
	isTauri: false,
	invoke: vi.fn(),
	toastError: vi.fn(),
	nativeUser: null as string | null,
	listenersReady: undefined as Promise<void> | undefined,
	listeners: new Map<string, (event: { payload: unknown }) => Promise<void>>(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	useBackend: () => mocks.backend,
	useInvoke: () => ({ data: mocks.profile }),
	useInvalidateInvoke: () => mocks.invalidate,
	useInvalidateInfiniteInvoke: () => mocks.invalidate,
}));
vi.mock("@flow-like/flow-like-ui/components/account/account-session", () => ({
	createAccountTokenProvider: () => ({ getTokens: async () => null }),
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
	isTauri: () => mocks.isTauri,
}));
vi.mock("@tauri-apps/api/event", () => ({
	listen: async (
		name: string,
		callback: (event: { payload: unknown }) => Promise<void>,
	) => {
		await mocks.listenersReady;
		mocks.listeners.set(name, callback);
		return () => mocks.listeners.delete(name);
	},
}));
vi.mock("sonner", () => ({ toast: { error: mocks.toastError } }));
vi.mock("@tauri-apps/plugin-deep-link", () => ({
	getCurrent: async () => null,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("aws-amplify", () => ({ Amplify: { configure: vi.fn() } }));
vi.mock("aws-amplify/auth", () => ({ decodeJWT: vi.fn() }));
vi.mock("../../lib/api", () => ({
	get: (profile: { hub: string }) =>
		new Promise((resolve, reject) => {
			mocks.configRequests.push({ hub: profile.hub, resolve, reject });
		}),
}));
vi.mock("../tauri-provider", () => ({
	ProfileSyncer: () => null,
	TauriBackend: class {},
}));
vi.mock("../sign-in-link-dialog", () => ({
	SignInLinkDialog: ({ url }: { url?: string }) => {
		mocks.signInLink = url;
		return null;
	},
}));

import { DesktopAuthProvider } from "../auth-provider";

const probe = {
	mounts: 0,
	unmounts: 0,
	mountedWith: [] as (string | undefined)[],
	auth: undefined as AuthContextProps | undefined,
};

function Probe() {
	const auth = useAuth();
	probe.auth = auth;
	const [clientAtMount] = useState(auth?.settings.client_id);
	useEffect(() => {
		probe.mounts += 1;
		probe.mountedWith.push(clientAtMount);
		return () => {
			probe.unmounts += 1;
		};
	}, [clientAtMount]);
	return null;
}

const oidcConfig = (clientId: string) => ({
	authority: "https://auth.flow-like.test",
	client_id: clientId,
	redirect_uri: "https://app.flow-like.com/desktop/callback",
});

let root: Root;
let container: HTMLDivElement;

async function flush() {
	for (let i = 0; i < 5; i++)
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 0));
		});
}

async function render() {
	await act(async () =>
		root.render(createElement(DesktopAuthProvider, null, createElement(Probe))),
	);
	await flush();
}

async function answer(index: number, config: unknown) {
	await act(async () => mocks.configRequests[index].resolve(config));
	await flush();
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	mocks.profile = { id: "profile-a", hub: "api.flow-like.com", secure: true };
	mocks.configRequests = [];
	mocks.openUrl.mockReset().mockResolvedValue(undefined);
	mocks.signInLink = undefined;
	mocks.isTauri = false;
	mocks.nativeUser = null;
	mocks.listenersReady = undefined;
	mocks.listeners.clear();
	mocks.toastError.mockReset();
	mocks.invoke.mockReset().mockImplementation(async (command: string) => {
		if (command === "native_auth_read")
			return {
				generation: "generation-a",
				initialized: true,
				user: mocks.nativeUser,
				authority: "https://auth.flow-like.test",
				clientId: "client-a",
			};
		if (command === "begin_login") return;
		throw new Error(`Unexpected command ${command}`);
	});
	probe.mounts = 0;
	probe.unmounts = 0;
	probe.mountedWith = [];
	probe.auth = undefined;
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	localStorage.clear();
	vi.restoreAllMocks();
});

describe("DesktopAuthProvider", () => {
	const browserConfig = () => ({
		...oidcConfig("client-a"),
		post_logout_redirect_uri: "https://app.flow-like.com/desktop/logout",
		metadata: {
			issuer: "https://auth.flow-like.test",
			authorization_endpoint: "https://auth.flow-like.test/authorize",
			end_session_endpoint: "https://auth.flow-like.test/logout",
		},
	});
	const storedUser = () =>
		new User({
			access_token: "access-a",
			id_token: "id-a",
			token_type: "Bearer",
			expires_at: Math.floor(Date.now() / 1000) + 3600,
			profile: {
				sub: "user-a",
				iss: "https://auth.flow-like.test",
				aud: "client-a",
				exp: Math.floor(Date.now() / 1000) + 3600,
				iat: Math.floor(Date.now() / 1000),
			},
		});

	test("reloads a completed native debug callback even when its change event was missed", async () => {
		mocks.isTauri = true;
		await render();
		await answer(0, browserConfig());
		expect(probe.auth?.isAuthenticated).toBe(false);
		await act(async () => probe.auth?.signinRedirect());
		expect(mocks.invoke).toHaveBeenCalledWith("begin_login", {
			profileId: "profile-a",
		});
		const invokeNative = mocks.invoke.getMockImplementation();
		if (!invokeNative) throw new Error("Native command mock is missing");
		let complete!: () => void;
		mocks.invoke.mockImplementation(async (command: string) => {
			if (command !== "deeplink_debug_dispatch") return invokeNative(command);
			await new Promise<void>((resolve) => {
				complete = resolve;
			});
			mocks.nativeUser = storedUser().toStorageString();
		});
		const url =
			"https://flow-like.com/callback?code=test-code&state=fl-native-test";
		await act(async () => {
			window.dispatchEvent(new CustomEvent("debug-oidc", { detail: { url } }));
		});
		expect(mocks.invoke).toHaveBeenCalledWith("deeplink_debug_dispatch", {
			url,
		});
		expect(probe.auth?.isAuthenticated).toBe(false);
		await act(async () => complete());
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(true);
		expect(probe.auth?.user?.access_token).toBe("access-a");
		expect(mocks.toastError).not.toHaveBeenCalled();
	});

	test("loads and clears native sessions only for the configured profile and hub", async () => {
		mocks.isTauri = true;
		await render();
		await answer(0, browserConfig());
		const changed = mocks.listeners.get("native-auth-changed");
		if (!changed) throw new Error("Native authentication listener is missing");
		mocks.nativeUser = storedUser().toStorageString();
		await act(async () =>
			changed({
				payload: {
					profileId: "other-profile",
					hub: "https://api.flow-like.com",
				},
			}),
		);
		expect(probe.auth?.isAuthenticated).toBe(false);
		await act(async () =>
			changed({
				payload: { profileId: "profile-a", hub: "https://other-hub.test" },
			}),
		);
		expect(probe.auth?.isAuthenticated).toBe(false);
		await act(async () =>
			changed({
				payload: { profileId: "profile-a", hub: "https://api.flow-like.com" },
			}),
		);
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(true);
		mocks.nativeUser = null;
		await act(async () =>
			changed({
				payload: { profileId: "profile-a", hub: "https://api.flow-like.com" },
			}),
		);
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(false);
		expect(probe.auth?.user).toBeUndefined();
	});

	test("recovers a native login completed before the event listeners were ready", async () => {
		mocks.isTauri = true;
		let ready!: () => void;
		mocks.listenersReady = new Promise<void>((resolve) => {
			ready = resolve;
		});
		await render();
		await answer(0, browserConfig());
		expect(probe.auth?.isAuthenticated).toBe(false);
		mocks.nativeUser = storedUser().toStorageString();
		await act(async () => ready());
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(true);
	});

	test("shows a failed native callback instead of leaving sign-in silently pending", async () => {
		mocks.isTauri = true;
		await render();
		await answer(0, browserConfig());
		const message =
			"Sign-in could not be completed. Start a new sign-in request.";
		await act(async () =>
			mocks.listeners.get("native-auth-error")?.({ payload: { message } }),
		);
		expect(mocks.toastError).toHaveBeenCalledWith(message);
		expect(probe.auth?.isAuthenticated).toBe(false);
	});

	test("reports a refused debug callback without publishing a signed-in session", async () => {
		mocks.isTauri = true;
		await render();
		await answer(0, browserConfig());
		mocks.invoke.mockRejectedValueOnce(
			"Authentication state is unknown or was already consumed",
		);
		const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
		await act(async () => {
			window.dispatchEvent(
				new CustomEvent("debug-oidc", {
					detail: {
						url: "https://flow-like.com/callback?code=test-code&state=fl-native-test",
					},
				}),
			);
		});
		await flush();
		expect(mocks.toastError).toHaveBeenCalledWith(
			"Sign-in could not be completed. Start a new sign-in request.",
		);
		expect(errorLog).toHaveBeenCalledWith(
			"Debug OIDC dispatch failed:",
			"Authentication state is unknown or was already consumed",
		);
		expect(probe.auth?.isAuthenticated).toBe(false);
		errorLog.mockRestore();
	});

	test("finishes native browser navigation without leaving sign-in or sign-out loading", async () => {
		localStorage.setItem(
			"oidc.user:https://auth.flow-like.test:client-a",
			storedUser().toStorageString(),
		);
		await render();
		await answer(0, browserConfig());
		expect(probe.auth?.isAuthenticated).toBe(true);
		await act(async () => probe.auth?.signoutRedirect());
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(false);
		expect(probe.auth?.isLoading).toBe(false);
		expect(probe.auth?.activeNavigator).toBeUndefined();
		expect(mocks.signInLink).toBeUndefined();
		await act(async () => probe.auth?.signinRedirect());
		await flush();
		expect(mocks.openUrl).toHaveBeenCalledTimes(2);
		expect(mocks.signInLink).toBe(mocks.openUrl.mock.calls[1][0]);
		expect(mocks.signInLink).toMatch(
			/^https:\/\/auth\.flow-like\.test\/authorize\?.*response_type=code/,
		);
		expect(probe.auth?.isLoading).toBe(false);
		expect(probe.auth?.activeNavigator).toBeUndefined();
		await act(async () => probe.auth?.events.load(storedUser()));
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(true);
		expect(probe.auth?.activeNavigator).toBeUndefined();
		expect(mocks.signInLink).toBeUndefined();
	});

	test("surfaces a system-browser failure and clears navigation state", async () => {
		await render();
		await answer(0, browserConfig());
		mocks.openUrl.mockRejectedValueOnce(new Error("browser unavailable"));
		await act(async () => probe.auth?.signinRedirect());
		await flush();
		expect(probe.auth?.error?.message).toBe("browser unavailable");
		expect(probe.auth?.isLoading).toBe(false);
		expect(probe.auth?.activeNavigator).toBeUndefined();
		expect(mocks.signInLink).toBe(mocks.openUrl.mock.calls[0][0]);
	});

	test("hides the previous hub's user while the new hub config is pending or unavailable", async () => {
		localStorage.setItem(
			"oidc.user:https://auth.flow-like.test:client-a",
			storedUser().toStorageString(),
		);
		await render();
		await answer(0, browserConfig());
		expect(probe.auth?.user?.access_token).toBe("access-a");
		mocks.profile = { ...mocks.profile, hub: "second-hub.test", secure: true };
		await render();
		expect(probe.auth?.isAuthenticated).toBe(false);
		expect(probe.auth?.user).toBeNull();
		await act(async () =>
			mocks.configRequests[1].reject(new Error("hub unavailable")),
		);
		await flush();
		expect(probe.auth?.user).toBeNull();
	});
	test("keeps the app mounted when the sign-in configuration arrives", async () => {
		await render();
		expect(probe.mounts).toBe(1);
		expect(probe.auth).toBeDefined();
		expect(mocks.configRequests.map((request) => request.hub)).toEqual([
			"api.flow-like.com",
		]);

		await answer(0, oidcConfig("client-a"));

		expect(probe.mounts).toBe(1);
		expect(probe.unmounts).toBe(0);
		expect(probe.auth?.settings.client_id).toBe("client-a");
		expect(probe.auth?.isLoading).toBe(false);
	});

	test("propagates sign-in and sign-out to the mounted app", async () => {
		await render();
		await answer(0, oidcConfig("client-a"));
		const now = Math.floor(Date.now() / 1000);

		await act(async () =>
			probe.auth?.events.load(
				new User({
					access_token: "access-1",
					id_token: "id-1",
					token_type: "Bearer",
					expires_at: now + 3600,
					profile: {
						sub: "user-1",
						iss: "https://auth.flow-like.test",
						aud: "client-a",
						exp: now + 3600,
						iat: now,
					},
				}),
			),
		);
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(true);
		expect(probe.auth?.user?.access_token).toBe("access-1");

		await act(async () => probe.auth?.removeUser());
		await flush();
		expect(probe.auth?.isAuthenticated).toBe(false);
		expect(probe.mounts).toBe(1);
		expect(probe.unmounts).toBe(0);
	});

	test("renders the app signed out when the configuration cannot load", async () => {
		const logError = vi.spyOn(console, "error").mockImplementation(() => {});
		await render();
		await act(async () =>
			mocks.configRequests[0].reject(new Error("network unreachable")),
		);
		await flush();

		expect(logError).toHaveBeenCalledWith(
			"Failed to fetch OpenID config:",
			expect.any(Error),
		);
		logError.mockRestore();
		expect(probe.mounts).toBe(1);
		expect(probe.unmounts).toBe(0);
		expect(probe.auth?.isLoading).toBe(false);
		expect(probe.auth?.isAuthenticated).toBe(false);
	});

	test("restores the stored session from the last configuration while the hub is unreachable", async () => {
		await render();
		await answer(0, oidcConfig("client-a"));
		const now = Math.floor(Date.now() / 1000);
		const expired = new User({
			access_token: "access-1",
			id_token: "id-1",
			token_type: "Bearer",
			expires_at: now - 60,
			profile: {
				sub: "user-1",
				iss: "https://auth.flow-like.test",
				aud: "client-a",
				exp: now - 60,
				iat: now - 3660,
			},
		});
		localStorage.setItem(
			"oidc.user:https://auth.flow-like.test:client-a",
			expired.toStorageString(),
		);
		await act(async () => root.unmount());
		root = createRoot(container);

		const logError = vi.spyOn(console, "error").mockImplementation(() => {});
		await render();
		await act(async () =>
			mocks.configRequests[1].reject(new Error("network unreachable")),
		);
		await flush();
		logError.mockRestore();

		expect(probe.auth?.settings.client_id).toBe("client-a");
		expect(probe.auth?.user?.profile.sub).toBe("user-1");
		expect(probe.auth?.isAuthenticated).toBe(false);
	});

	test("remounts the app when a hub switch changes the client", async () => {
		await render();
		await answer(0, oidcConfig("client-a"));

		mocks.profile = { ...mocks.profile, hub: "hub.example.com", secure: true };
		await render();
		expect(mocks.configRequests[1]?.hub).toBe("hub.example.com");
		await answer(1, oidcConfig("client-b"));

		expect(probe.unmounts).toBe(1);
		expect(probe.mountedWith).toEqual([undefined, "client-b"]);
		expect(probe.auth?.settings.client_id).toBe("client-b");
	});

	test("ignores a configuration answer for a hub that is no longer current", async () => {
		await render();
		mocks.profile = { ...mocks.profile, hub: "hub.example.com", secure: true };
		await render();

		await answer(1, oidcConfig("client-b"));
		await answer(0, oidcConfig("client-a"));

		expect(probe.auth?.settings.client_id).toBe("client-b");
		expect(probe.mounts).toBe(1);
		expect(probe.unmounts).toBe(0);
	});

	test("creates a new auth scope when another hub uses the same client ID", async () => {
		await render();
		await answer(0, oidcConfig("shared-client"));
		const previousAuth = probe.auth;
		mocks.profile = { ...mocks.profile, hub: "another-hub.test", secure: true };
		await render();
		await answer(1, oidcConfig("shared-client"));
		expect(probe.unmounts).toBe(1);
		expect(probe.auth).not.toBe(previousAuth);
		expect(probe.auth?.settings.client_id).toBe("shared-client");
	});
});
