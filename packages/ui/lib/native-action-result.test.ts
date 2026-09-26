import { describe, expect, mock, test } from "bun:test";
import {
	type NativeActionCompletionContext,
	type NativeActionRequest,
	assertNativeActionCurrent,
	completeNativeAction,
	nativeChatOutcome,
	runNativeAction,
} from "./native-action-result";

const request: NativeActionRequest = {
	id: "request-1",
	scope: "workspace-a",
	responseMode: "text",
	responseDeadline: new Date(100_000).toISOString(),
};
const context = () => ({
	getCurrentScope: () => request.scope,
	now: () => 1000,
	invoke: mock<NonNullable<NativeActionCompletionContext["invoke"]>>(
		async () => undefined,
	),
});

describe("native action results", () => {
	test("returns final answer text without reasoning, tools, or debugging fields", async () => {
		const message = {
			inner: { content: "The package arrives Tuesday." },
			plan_steps: [{ content: "Private reasoning" }],
			debug_report: { token: "private" },
		};
		const options = context();
		expect(
			await completeNativeAction(request, nativeChatOutcome(message), options),
		).toBe(true);
		expect(options.invoke).toHaveBeenCalledWith("native_complete_action", {
			result: {
				id: request.id,
				scope: request.scope,
				status: "success",
				text: "The package arrives Tuesday.",
			},
		});
	});

	test("failed streams never return partial content as a successful answer", () => {
		expect(
			nativeChatOutcome({
				inner: { content: "Partial answer" },
				error: { title: "Stopped", message: "Cancelled by the user" },
			}),
		).toEqual({ status: "error", error: "Stopped: Cancelled by the user" });
		expect(
			nativeChatOutcome({
				inner: { content: "Partial answer" },
				run_context: { outcome: "error" },
			}).status,
		).toBe("error");
		expect(nativeChatOutcome({ inner: { content: "" } }).status).toBe(
			"interaction_required",
		);
	});

	test("extracts only text from structured final content", () => {
		expect(
			nativeChatOutcome({
				inner: {
					content: [
						{ type: "reasoning", text: "private" },
						{ type: "text", text: "Visible answer" },
						{ type: "image_url", image_url: { url: "private" } },
					],
				},
			}),
		).toEqual({ status: "success", text: "Visible answer" });
	});

	test("JSON output stays valid JSON text for Shortcuts dictionary conversion", async () => {
		const options = context();
		const json = JSON.stringify({ result: [1, true, null], name: "東京" });
		await completeNativeAction(
			{ ...request, responseMode: "result" },
			{ status: "success", json },
			options,
		);
		expect(options.invoke.mock.calls[0][1].result.json).toBe(json);
		expect(
			JSON.parse(options.invoke.mock.calls[0][1].result.json ?? "null"),
		).toEqual({
			result: [1, true, null],
			name: "東京",
		});
	});

	test("rejects expired and invalid deadlines before starting", async () => {
		const run = mock(async () => ({
			status: "success" as const,
			text: "unused",
		}));
		const options = context();
		await runNativeAction(
			{ ...request, responseDeadline: new Date(999).toISOString() },
			{ ...options, run },
		);
		await runNativeAction(
			{ ...request, responseDeadline: "invalid" },
			{ ...options, run },
		);
		expect(run).not.toHaveBeenCalled();
		expect(options.invoke).not.toHaveBeenCalled();
	});

	test("account changes during asynchronous preparation stop dispatch and completion", async () => {
		let scope: string | undefined = request.scope;
		const start = mock(async () => {});
		const options = context();
		await runNativeAction(request, {
			...options,
			getCurrentScope: () => scope,
			run: async (check) => {
				await Promise.resolve();
				scope = "workspace-b";
				check();
				await start();
				return { status: "success", text: "Never sent" };
			},
		});
		expect(start).not.toHaveBeenCalled();
		expect(options.invoke).not.toHaveBeenCalled();
	});

	test("completed runs after the deadline remain in the app but cannot answer another request", async () => {
		let now = 1000;
		const options = context();
		await runNativeAction(request, {
			...options,
			now: () => now,
			run: async () => {
				now = 100_000;
				return { status: "success", text: "Late answer" };
			},
		});
		expect(options.invoke).not.toHaveBeenCalled();
	});

	test("returns preparation errors, and does not complete capacity-deferred requests", async () => {
		const options = context();
		await runNativeAction(request, {
			...options,
			run: async () => {
				throw new Error("Attachment could not be read");
			},
		});
		expect(options.invoke.mock.calls[0][1].result).toMatchObject({
			status: "error",
			error: "Attachment could not be read",
		});
		options.invoke.mockClear();
		await runNativeAction(request, { ...options, run: async () => undefined });
		expect(options.invoke).not.toHaveBeenCalled();
	});

	test("enforces UTF8 field limits and total escaped envelope size", async () => {
		for (const outcome of [
			{ status: "success" as const, text: "界".repeat(65_537) },
			{
				status: "success" as const,
				json: JSON.stringify("a".repeat(192 * 1024)),
			},
			{ status: "success" as const, text: "\u0000".repeat(50_000) },
			{ status: "error" as const, error: "a".repeat(4097) },
			{ status: "success" as const, json: "{broken" },
		]) {
			const options = context();
			await completeNativeAction(request, outcome, options);
			expect(options.invoke.mock.calls[0][1].result.status).toBe("error");
			expect(options.invoke.mock.calls[0][1].result.text).toBeUndefined();
			expect(options.invoke.mock.calls[0][1].result.json).toBeUndefined();
		}
	});

	test("legacy navigation requests and unavailable native completion remain safe no-ops", async () => {
		const options = context();
		expect(
			await completeNativeAction(
				{ ...request, responseMode: undefined },
				{ status: "success" },
				options,
			),
		).toBe(false);
		expect(options.invoke).not.toHaveBeenCalled();
		options.invoke.mockImplementation(async () => {
			throw new Error("Receiver closed");
		});
		expect(
			await completeNativeAction(
				request,
				{ status: "success", text: "Answer" },
				options,
			),
		).toBe(false);
		expect(() => assertNativeActionCurrent(request, undefined, 1000)).toThrow(
			"different account",
		);
	});
});
