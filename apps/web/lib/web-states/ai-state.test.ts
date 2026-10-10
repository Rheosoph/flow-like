import type { IHistoryMessage, IResponse } from "@flow-like/flow-like-ui";
import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiPost: vi.fn() }));
vi.mock("./api-utils", () => ({
	apiPost: mocks.apiPost,
	getApiBaseUrl: () => "https://api.example.test",
}));

import { WebAIState } from "./ai-state";

describe("web inline completion", () => {
	beforeEach(() => {
		mocks.apiPost.mockReset();
	});

	test("sends the active profile and app to the plain completion route", async () => {
		const backend = {
			auth: { user: { access_token: "account-token" } },
			profile: { id: "profile-a" },
		};
		const state = new WebAIState(backend as never);
		const messages = [
			{ role: "system", content: "Continue the sentence." },
			{ role: "user", content: "This is a standard" },
		] as IHistoryMessage[];
		const response: IResponse = {
			choices: [
				{
					index: 0,
					finish_reason: "stop",
					message: { role: "assistant", content: " test." },
				},
			],
			usage: { prompt_tokens: 20, completion_tokens: 2, total_tokens: 22 },
		};
		mocks.apiPost.mockResolvedValue(response);

		await expect(state.chatComplete(messages, "app-1")).resolves.toBe(response);
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"ai/completion",
			{ messages, app_id: "app-1", profile_id: "profile-a" },
			backend.auth,
		);

		backend.profile = { id: "profile-b" };
		await state.chatComplete(messages);
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"ai/completion",
			{ messages, app_id: undefined, profile_id: "profile-b" },
			backend.auth,
		);
	});

	test("does not let a missing active profile select an unrelated profile", async () => {
		await expect(new WebAIState({}).chatComplete([])).rejects.toThrow(
			"Select a profile",
		);
		expect(mocks.apiPost).not.toHaveBeenCalled();
	});

	test("propagates provider and plan failures to the editor", async () => {
		mocks.apiPost.mockRejectedValue(new Error("Model budget exhausted"));
		const state = new WebAIState({ profile: { id: "profile-a" } } as never);
		await expect(state.chatComplete([])).rejects.toThrow(
			"Model budget exhausted",
		);
	});
});
