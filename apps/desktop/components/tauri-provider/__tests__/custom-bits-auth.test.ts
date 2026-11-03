import type { IBit, IProfile } from "@flow-like/flow-like-ui";
import { User } from "oidc-client-ts";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { TauriBackend } from "../../tauri-provider";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), fetch: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("@flow-like/flow-like-ui", () => ({
	IBitTypes: {},
	getActiveTraceContext: () => undefined,
	getTelemetryTraceparent: () => undefined,
}));

import { TauriApiState } from "../api-state";
import { BitState } from "../bit-state";

const profile = { id: "profile", hub: "https://hub.test" } as IProfile;
const localBit = { id: "local-model" } as IBit;
const remoteBit = { id: "remote-model" } as IBit;
let local: IBit[];

function user(token: string, expiresIn: number) {
	return new User({
		access_token: token,
		token_type: "Bearer",
		expires_at: Math.floor(Date.now() / 1000) + expiresIn,
		profile: {
			sub: "alice",
			iss: "https://issuer.test",
			aud: "client",
			exp: 1,
			iat: 0,
		},
	});
}

function backend(auth?: AuthContextProps) {
	const tasks: Promise<unknown>[] = [];
	const apiState = new TauriApiState();
	apiState.setAuth(auth ?? null);
	const state = new BitState({
		profile,
		auth,
		apiState,
		backgroundTaskHandler: (task: Promise<unknown>) => tasks.push(task),
	} as unknown as TauriBackend);
	return { state, tasks };
}

beforeEach(() => {
	vi.resetAllMocks();
	vi.stubGlobal("navigator", { onLine: true });
	local = [localBit];
	mocks.invoke.mockImplementation(async (command, args) => {
		if (command === "get_custom_bits") return [...local];
		if (command === "upsert_custom_bit") {
			local = [...local.filter((bit) => bit.id !== args.bit.id), args.bit];
			return args.bit;
		}
		if (command === "remove_custom_bit") {
			local = local.filter((bit) => bit.id !== args.bitId);
			return;
		}
		throw new Error(`Unexpected command ${command}`);
	});
	mocks.fetch.mockImplementation(async () => new Response("{}"));
});

afterEach(() => vi.unstubAllGlobals());

describe("custom model synchronization authentication", () => {
	test.each([
		["without an auth context", undefined],
		[
			"while sign-in is loading",
			{ isLoading: true, isAuthenticated: false, user: null },
		],
		[
			"after sign-out",
			{ isLoading: false, isAuthenticated: false, user: null },
		],
	])(
		"local models remain usable %s without anonymous API requests",
		async (_label, auth) => {
			const { state, tasks } = backend(auth as AuthContextProps | undefined);
			await expect(state.listCustomBits()).resolves.toEqual([localBit]);
			await expect(state.upsertCustomBit(remoteBit)).resolves.toEqual(
				remoteBit,
			);
			await state.deleteCustomBit("local-model");
			expect(local).toEqual([remoteBit]);
			expect(tasks).toHaveLength(0);
			expect(mocks.fetch).not.toHaveBeenCalled();
		},
	);

	test("an expired stored session renews and merges the remote library", async () => {
		const signinSilent = vi.fn(async () => user("renewed", 3600));
		const { state, tasks } = backend({
			isAuthenticated: false,
			user: user("expired", -60),
			signinSilent,
		} as unknown as AuthContextProps);
		mocks.fetch.mockResolvedValue(new Response(JSON.stringify([remoteBit])));

		await expect(state.listCustomBits()).resolves.toEqual([localBit]);
		await Promise.all(tasks);

		expect(signinSilent).toHaveBeenCalledTimes(1);
		expect(local).toEqual([localBit, remoteBit]);
		expect(mocks.fetch).toHaveBeenCalledTimes(1);
		const [url, init] = mocks.fetch.mock.calls[0];
		expect(url).toBe(
			"https://hub.test/api/v1/user/bits?include_secrets=true&device_models=true",
		);
		expect(new Headers(init.headers).get("Authorization")).toBe(
			"Bearer renewed",
		);
	});

	test("signed-in model changes still update the authenticated remote library", async () => {
		const { state } = backend({
			isAuthenticated: true,
			user: user("current", 3600),
		} as AuthContextProps);
		await state.upsertCustomBit(remoteBit);
		await state.deleteCustomBit("remote-model");

		expect(
			mocks.fetch.mock.calls.map(([url, init]) => [url, init.method]),
		).toEqual([
			["https://hub.test/api/v1/user/bits/remote-model", "PUT"],
			["https://hub.test/api/v1/user/bits/remote-model", "DELETE"],
		]);
		for (const [, init] of mocks.fetch.mock.calls) {
			expect(new Headers(init.headers).get("Authorization")).toBe(
				"Bearer current",
			);
		}
	});
});
