import { afterEach, describe, expect, test, vi } from "vitest";
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import type { WebBackendRef } from "./api-utils";
import { WebUserState } from "./user-state";

function backend(subject = "user-a"): WebBackendRef {
	return {
		auth: {
			isAuthenticated: true,
			user: {
				access_token: subject,
				profile: { sub: subject, iss: "https://login.test" },
			},
		},
	} as WebBackendRef;
}

function pendingResponse() {
	let resolve!: (response: Response) => void;
	const promise = new Promise<Response>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

const profile = {
	id: "profile-a",
	name: "Workspace",
	hub: "api.test",
	user_id: "user-a",
	created_at: "2026-09-01",
	updated_at: "2026-09-01",
};

afterEach(() => {
	vi.unstubAllGlobals();
	vi.unstubAllEnvs();
	vi.restoreAllMocks();
});

describe("account reads in one synchronous fan-out", () => {
	test("derives the selected profile and settings list from one HTTP read", async () => {
		const response = pendingResponse();
		const request = vi.fn(async () => (await response.promise).clone());
		vi.stubGlobal("fetch", request);
		const state = new WebUserState(backend());
		const selected = state.getSettingsProfile();
		const profiles = state.getAllSettingsProfiles();
		response.resolve(Response.json([profile]));
		expect((await selected).hub_profile.id).toBe("profile-a");
		expect((await profiles).map((entry) => entry.hub_profile.id)).toEqual([
			"profile-a",
		]);
		expect(request).toHaveBeenCalledTimes(1);
		await state.getProfiles();
		expect(request).toHaveBeenCalledTimes(2);
	});

	test("shares same-turn user info, keeping explicit refetches fresh", async () => {
		const response = pendingResponse();
		const request = vi.fn(async () => (await response.promise).clone());
		vi.stubGlobal("fetch", request);
		const state = new WebUserState(backend());
		const first = state.getInfo();
		const second = state.getInfo();
		response.resolve(Response.json({ id: "user-a" }));
		expect(await first).toEqual(await second);
		expect(request).toHaveBeenCalledTimes(1);
		await state.getInfo();
		expect(request).toHaveBeenCalledTimes(2);
	});

	test("never joins a pending request from another account or backend", async () => {
		const responses = [pendingResponse(), pendingResponse(), pendingResponse()];
		const request = vi.fn(
			() => responses[request.mock.calls.length - 1].promise,
		);
		vi.stubGlobal("fetch", request);
		const reference = backend();
		const state = new WebUserState(reference);
		const first = state.getInfo();
		reference.auth = backend("user-b").auth;
		const second = state.getInfo();
		const other = new WebUserState(reference).getInfo();
		responses.forEach((entry, index) =>
			entry.resolve(Response.json({ id: index })),
		);
		expect(await Promise.all([first, second, other])).toEqual([
			{ id: 0 },
			{ id: 1 },
			{ id: 2 },
		]);
		expect(request).toHaveBeenCalledTimes(3);
	});

	test("a failed shared read is retried by the next caller", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		const response = pendingResponse();
		const request = vi.fn(async () => (await response.promise).clone());
		vi.stubGlobal("fetch", request);
		const state = new WebUserState(backend());
		const first = state.getInfo();
		const second = state.getInfo();
		response.resolve(
			Response.json({ message: "Unavailable" }, { status: 503 }),
		);
		const result = await Promise.allSettled([first, second]);
		expect(result.map((entry) => entry.status)).toEqual([
			"rejected",
			"rejected",
		]);
		request.mockImplementation(() =>
			Promise.resolve(Response.json({ id: "user-a" })),
		);
		await expect(state.getInfo()).resolves.toEqual({ id: "user-a" });
		expect(request).toHaveBeenCalledTimes(2);
	});

	test("a successful write makes the next read independent of older in-flight reads", async () => {
		const old = pendingResponse();
		let reads = 0;
		const request = vi.fn((_url: string, options: RequestInit) => {
			if (options.method === "POST") return Promise.resolve(Response.json({}));
			return ++reads === 1
				? old.promise
				: Promise.resolve(Response.json([{ ...profile, name: "Updated" }]));
		});
		vi.stubGlobal("fetch", request);
		const state = new WebUserState(backend());
		const first = state.getProfiles();
		await state.updateProfileShortcuts(
			{ hub_profile: profile } as unknown as ISettingsProfile,
			[],
		);
		expect((await state.getProfiles())[0].name).toBe("Updated");
		old.resolve(Response.json([profile]));
		expect((await first)[0].name).toBe("Workspace");
		expect(reads).toBe(2);
	});

	test("a post-await read starts fresh while the initial response is still pending", async () => {
		const old = pendingResponse();
		const fresh = pendingResponse();
		const request = vi.fn(() =>
			request.mock.calls.length === 1 ? old.promise : fresh.promise,
		);
		vi.stubGlobal("fetch", request);
		const state = new WebUserState(backend());
		const first = state.getProfiles();
		await Promise.resolve();
		const second = state.getProfiles();
		const third = state.getProfiles();
		expect(request).toHaveBeenCalledTimes(2);
		old.resolve(Response.json([profile]));
		fresh.resolve(Response.json([{ ...profile, name: "Updated" }]));
		expect((await first)[0].name).toBe("Workspace");
		expect((await second)[0].name).toBe("Updated");
		expect((await third)[0].name).toBe("Updated");
	});
});
