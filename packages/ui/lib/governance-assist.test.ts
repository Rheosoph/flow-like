import { afterEach, describe, expect, mock, spyOn, test } from "bun:test";
import { EmptyApiState } from "../state/backend-state/empty-states/api-state";
import type { IProfile } from "../types";
import { requestGovernanceAssist } from "./governance-assist";

const profile = { hub: "https://governance.example" } as IProfile;
const path = "apps/app-id/ai-act/assessment/suggest";
const result = {
	suggestion: { purpose: "Review café orders", suggestedAnswers: [] },
	signals: { hasAi: true },
	model: "governance-model",
};

function streamingResponse(events: string): Response {
	const bytes = new TextEncoder().encode(events);
	return new Response(
		new ReadableStream<Uint8Array>({
			start(controller) {
				for (let index = 0; index < bytes.length; index += 7) {
					controller.enqueue(bytes.slice(index, index + 7));
				}
				controller.close();
			},
		}),
		{ headers: { "Content-Type": "text/event-stream" } },
	);
}

afterEach(() => mock.restore());

describe("governance assessment assistance", () => {
	test("requests streaming and preserves the final result across fragmented events", async () => {
		const fetch = spyOn(globalThis, "fetch").mockResolvedValue(
			streamingResponse(
				`: accepted\n\n: keep-alive\n\nevent: progress\ndata: {"phase":"reviewing"}\n\nevent: completed\ndata: ${JSON.stringify(result)}\n\n`,
			),
		);
		const answer = await requestGovernanceAssist<typeof result>(
			new EmptyApiState(() => "test-token"),
			profile,
			path,
			{ profile, modelId: "override-model" },
		);

		expect(answer).toEqual(result);
		expect(fetch).toHaveBeenCalledTimes(1);
		const [url, options] = fetch.mock.calls[0];
		expect(String(url)).toEndWith(`/api/v1/${path}`);
		expect(options?.method).toBe("POST");
		expect(new Headers(options?.headers).get("Accept")).toBe(
			"text/event-stream",
		);
		expect(new Headers(options?.headers).get("Authorization")).toBe(
			"Bearer test-token",
		);
		expect(JSON.parse(options?.body as string)).toEqual({
			profile,
			modelId: "override-model",
			stream: true,
		});
	});

	test.each([
		": accepted\n\n: keep-alive\n\n",
		'event: completed\ndata: {"suggestion":{}}\n\n',
	])("rejects a closed stream without a complete result", async (events) => {
		spyOn(globalThis, "fetch").mockResolvedValue(streamingResponse(events));
		await expect(
			requestGovernanceAssist(new EmptyApiState(), profile, path, { profile }),
		).rejects.toThrow("disconnected before returning a result");
	});

	test("surfaces public stream errors without retrying the assessment", async () => {
		const fetch = spyOn(globalThis, "fetch").mockResolvedValue(
			streamingResponse(
				': accepted\n\nevent: error\ndata: {"error":"The assessment took too long. Please try again.","code":"REQUEST_TIMEOUT"}\n\n',
			),
		);
		await expect(
			requestGovernanceAssist(new EmptyApiState(), profile, path, { profile }),
		).rejects.toThrow("The assessment took too long. Please try again.");
		expect(fetch).toHaveBeenCalledTimes(1);
	});

	test("preserves an HTTP rejection before streaming starts", async () => {
		const fetch = spyOn(globalThis, "fetch").mockResolvedValue(
			new Response("Forbidden", { status: 403, statusText: "Forbidden" }),
		);
		await expect(
			requestGovernanceAssist(new EmptyApiState(), profile, path, { profile }),
		).rejects.toThrow("403 Forbidden");
		expect(fetch).toHaveBeenCalledTimes(1);
	});
});
