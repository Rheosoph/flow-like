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
	profile: { hub: "api.flow-like.com", secure: true },
	configRequests: [] as ConfigRequest[],
	invalidate: async () => undefined,
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
vi.mock("@tauri-apps/api/event", () => ({
	listen: async () => () => undefined,
}));
vi.mock("@tauri-apps/plugin-deep-link", () => ({
	getCurrent: async () => null,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
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
	mocks.profile = { hub: "api.flow-like.com", secure: true };
	mocks.configRequests = [];
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
});

describe("DesktopAuthProvider", () => {
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

	test("remounts the app when a hub switch changes the client", async () => {
		await render();
		await answer(0, oidcConfig("client-a"));

		mocks.profile = { hub: "hub.example.com", secure: true };
		await render();
		expect(mocks.configRequests[1]?.hub).toBe("hub.example.com");
		await answer(1, oidcConfig("client-b"));

		expect(probe.unmounts).toBe(1);
		expect(probe.mountedWith).toEqual([undefined, "client-b"]);
		expect(probe.auth?.settings.client_id).toBe("client-b");
	});

	test("ignores a configuration answer for a hub that is no longer current", async () => {
		await render();
		mocks.profile = { hub: "hub.example.com", secure: true };
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
		mocks.profile = { hub: "another-hub.test", secure: true };
		await render();
		await answer(1, oidcConfig("shared-client"));
		expect(probe.unmounts).toBe(1);
		expect(probe.auth).not.toBe(previousAuth);
		expect(probe.auth?.settings.client_id).toBe("shared-client");
	});
});
