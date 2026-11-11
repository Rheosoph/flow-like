import type { ISettingsProfile } from "@flow-like/flow-like-ui";
import { afterEach, describe, expect, test, vi } from "vitest";
import type { WebBackendRef } from "./api-utils";
import { WebUserState } from "./user-state";

function storedProfile(hiddenBots?: string[] | null) {
	return {
		id: "profile-1",
		name: "Work",
		hub: "api.example.test",
		user_id: "user-1",
		created_at: "2026-11-11T10:00:00Z",
		updated_at: "2026-11-11T10:01:00Z",
		...(hiddenBots === undefined ? {} : { hidden_bots: hiddenBots }),
	};
}

function settingsProfile(): ISettingsProfile {
	return {
		hub_profile: { id: "profile-1", name: "Work", bits: [] },
	} as unknown as ISettingsProfile;
}

function state() {
	return new WebUserState({
		auth: { user: { access_token: "test-token" } },
	} as WebBackendRef);
}

function respond(body: unknown) {
	const request = vi.fn(async (_url: unknown, _init?: RequestInit) =>
		Response.json(body),
	);
	vi.stubGlobal("fetch", request);
	return request;
}

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("WebUserState hidden bots through the HTTP adapter", () => {
	test("sends only the hidden bots, so no other profile field is rewritten", async () => {
		const request = respond({ profile: storedProfile(["bot-a", "general"]) });
		const profile = settingsProfile();

		await state().updateProfileHiddenBots(profile, ["bot-a", "general"]);

		expect(request).toHaveBeenCalledTimes(1);
		const [url, init] = request.mock.calls[0];
		expect(String(url)).toContain("/profile/profile-1");
		expect(init?.method).toBe("POST");
		expect(JSON.parse(String(init?.body))).toEqual({
			hidden_bots: ["bot-a", "general"],
		});
		expect(profile.hub_profile.hidden_bots).toEqual(["bot-a", "general"]);
	});

	test("rejects an HTTP 200 from a hub that silently ignores the field", async () => {
		respond({ profile: storedProfile() });

		await expect(
			state().updateProfileHiddenBots(settingsProfile(), ["bot-a"]),
		).rejects.toThrow("This server does not support hiding bots yet.");
	});

	test("reads a profile that never hid a bot as hiding none", async () => {
		respond({ profile: storedProfile(null) });
		const profile = settingsProfile();

		await state().updateProfileHiddenBots(profile, []);

		expect(profile.hub_profile.hidden_bots).toEqual([]);
	});
});
