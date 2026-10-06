import { describe, expect, test } from "bun:test";
import type { IIntercomEvent } from "../../../../lib/schema/events/intercom-event";
import type { CreateRunAccumulator } from "../contracts";
import { NO_TERMINAL_SIGNALS, createRunAccumulator } from "./events";

const _contract: CreateRunAccumulator = createRunAccumulator;

let sequence = 0;
const ev = (event_type: string, payload: unknown): IIntercomEvent => {
	sequence += 1;
	return {
		event_id: `event-${sequence}`,
		event_type,
		payload,
		timestamp: {
			secs_since_epoch: 1_790_000_000 + sequence,
			nanos_since_epoch: 0,
		},
	};
};

const reader = () => _contract({ appId: "app-1", eventId: "event-1" });

/** Push Step (push_step.rs): the whole plan so far, the newest step current. */
const stepEvent = (titles: readonly string[], message = "") =>
	ev("chat_stream_partial", {
		chunk: null,
		actions: [],
		attachments: [],
		plan: {
			plan: titles.map((title, index) => [index + 1, title]),
			current_step: titles.length,
			current_message: message,
		},
		widgets: [],
	});

/** Push Chunk (push_chunk.rs): one assistant delta, no plan. */
const chunkEvent = (delta: Record<string, unknown>) =>
	ev("chat_stream_partial", {
		chunk: {
			id: "chunk",
			choices: [{ index: 0, delta: { role: "assistant", ...delta } }],
		},
		actions: [],
		attachments: [],
		plan: null,
		widgets: [],
	});

/** Push Attachment(s): attachments only. */
const attachmentEvent = (attachments: readonly unknown[]) =>
	ev("chat_stream_partial", {
		chunk: null,
		actions: [],
		attachments,
		plan: null,
		widgets: [],
	});

/** Push Response (push_response.rs): the whole message and every attachment so far. */
const responseEvent = (
	message: Record<string, unknown>,
	attachments: readonly unknown[] = [],
) =>
	ev("chat_stream", {
		response: {
			choices: [
				{
					finish_reason: "stop",
					index: 0,
					logprobs: null,
					message: { role: "assistant", ...message },
				},
			],
			usage: { completion_tokens: 0, prompt_tokens: 0, total_tokens: 0 },
		},
		local_session: {},
		global_session: {},
		actions: [],
		attachments,
		widgets: [],
		model_id: null,
	});

const completedEvent = (payload: Record<string, unknown>) =>
	ev("completed", {
		run_id: "run-1",
		duration_ms: 1200,
		log_level: 0,
		...payload,
	});

const INVOICE_FILE = {
	url: "https://files.example.com/invoice.pdf",
	name: "invoice.pdf",
	type: "application/pdf",
	size: 1_284_096,
};

/** A server run as the executor streams it: run id, two Push Steps, chunks, files, result, completed. */
function serverRun() {
	const run = reader();
	run.push([ev("run_initiated", { run_id: "run-1" })]);
	run.push([stepEvent(["Read documents: Loading the invoice"])]);
	run.push([
		stepEvent(
			["Read documents: Loading the invoice", "Run OCR: Recognising text"],
			"6 of 14 pages are scans.",
		),
	]);
	run.push([
		chunkEvent({ content: "## Invoice " }),
		chunkEvent({ content: "extracted" }),
	]);
	run.push([
		attachmentEvent(["https://files.example.com/line-items.csv", INVOICE_FILE]),
	]);
	run.push([
		ev("generic_result", { invoice_number: "RE-2026-0917" }),
		completedEvent({ status: "completed" }),
	]);
	return run.output();
}

describe("createRunAccumulator", () => {
	test("a server run's steps and streamed text", () => {
		const output = serverRun();
		expect(output.eventCount).toBe(8);
		expect(output.answer).toBe("## Invoice extracted");
		expect(
			output.steps.map((step) => [step.number, step.title, step.state]),
		).toEqual([
			[1, "Read documents", "done"],
			[2, "Run OCR", "active"],
		]);
		expect(output.steps[1]?.detail).toBe("Recognising text");
		expect(output.steps[1]?.message).toBe("6 of 14 pages are scans.");
	});

	test("a server run's files, result and terminal signals", () => {
		const output = serverRun();
		expect(output.attachments).toEqual([
			"https://files.example.com/line-items.csv",
			INVOICE_FILE,
		]);
		expect(output.result).toEqual({
			value: { invoice_number: "RE-2026-0917" },
		});
		expect(output.terminal).toEqual({
			runInitiated: true,
			errorMessage: null,
			completedStatus: "completed",
			rejectedStage: null,
			logLevel: 0,
			durationMs: 1200,
		});
	});

	test("Push Response replaces the streamed text and carries every file so far", () => {
		const run = reader();
		run.push([chunkEvent({ content: "draft" })]);
		run.push([
			responseEvent({ content: "Final answer" }, [
				"https://files.example.com/a.pdf",
				{ url: "https://files.example.com/a.pdf", name: "a.pdf" },
			]),
		]);
		const output = run.output();
		expect(output.answer).toBe("Final answer");
		expect(output.attachments).toEqual([
			{ url: "https://files.example.com/a.pdf", name: "a.pdf" },
		]);
	});

	test("a richer record of a file is never replaced by a bare one", () => {
		const run = reader();
		run.push([
			attachmentEvent([
				{ url: "https://x.example/a.png", name: "a.png", size: 10 },
			]),
			responseEvent({
				content: "",
				content_parts: [
					{ type: "image_url", image_url: { url: "https://x.example/a.png" } },
				],
			}),
		]);
		expect(run.output().attachments).toEqual([
			{ url: "https://x.example/a.png", name: "a.png", size: 10 },
		]);
	});

	test("media content parts of the answer become files", () => {
		const run = reader();
		run.push([
			responseEvent({
				content: "caption",
				content_parts: [
					{
						type: "image_url",
						image_url: {
							url: "https://x.example/p.png",
							media_type: "image/png",
						},
					},
					{ type: "audio_url", audio_url: "https://x.example/a.mp3" },
					{ type: "document_url", document_url: "https://x.example/d.pdf" },
				],
			}),
		]);
		const output = run.output();
		expect(output.answer).toBe("caption");
		expect(output.attachments).toEqual([
			{ url: "https://x.example/p.png", type: "image/png" },
			{ url: "https://x.example/a.mp3", type: "audio/*" },
			{ url: "https://x.example/d.pdf" },
		]);
	});

	test("every falsy result is a result; the last one wins", () => {
		for (const value of [false, 0, "", null]) {
			const run = reader();
			run.push([ev("generic_result", value)]);
			expect(run.output().result).toEqual({ value });
		}
		const run = reader();
		run.push([ev("generic_result", "first"), ev("generic_result", { n: 2 })]);
		expect(run.output().result).toEqual({ value: { n: 2 } });
	});

	test("return, output and intercom return are results; run metadata never is", () => {
		const viaReturn = reader();
		viaReturn.push([ev("return", 42)]);
		expect(viaReturn.output().result).toEqual({ value: 42 });

		const viaOutput = reader();
		viaOutput.push([ev("output", ["a", "b"])]);
		expect(viaOutput.output().result).toEqual({ value: ["a", "b"] });

		const viaIntercom = reader();
		viaIntercom.push([ev("intercom", { type: "return", value: "done" })]);
		expect(viaIntercom.output().result).toEqual({ value: "done" });

		const noResult = reader();
		noResult.push([
			ev("run_initiated", {
				run_id: "run-1",
				preamble: { total_ms: 40, steps: [] },
			}),
			ev("log", { message: "Execution started" }),
			ev("intercom", { type: "other" }),
			ev("log", { message: "Execution completed" }),
			completedEvent({ status: "completed" }),
		]);
		expect(noResult.output().result).toBeNull();
		expect(noResult.output().answer).toBe("");
	});

	test("text_output and stream_text append to the answer", () => {
		const run = reader();
		run.push([
			ev("text_output", "Hello"),
			ev("stream_text", { text: ", world" }),
			ev("stream_text", { other: 1 }),
		]);
		expect(run.output().answer).toBe("Hello, world");
	});

	test("a failing server run keeps the first error text", () => {
		const run = reader();
		run.push([
			ev("run_initiated", { run_id: "run-9" }),
			ev("error", { message: "Execution failed" }),
			ev("error", { message: "Later noise" }),
			completedEvent({ status: "failed", log_level: 4 }),
		]);
		const { terminal } = run.output();
		expect(terminal.errorMessage).toBe("Execution failed");
		expect(terminal.completedStatus).toBe("failed");
		expect(terminal.logLevel).toBe(4);
	});

	test("an error event without text still counts, a string payload is its text", () => {
		const bare = reader();
		bare.push([ev("error", {})]);
		expect(bare.output().terminal.errorMessage).toBe("");
		const text = reader();
		text.push([ev("error", "The workflow failed.")]);
		expect(text.output().terminal.errorMessage).toBe("The workflow failed.");
	});

	test("completed status is normalised like the API's", () => {
		const cases: [unknown, string][] = [
			["completed", "completed"],
			["Success", "completed"],
			["succeeded", "completed"],
			["canceled", "cancelled"],
			["cancelled", "cancelled"],
			["timed_out", "timeout"],
			["timeout", "timeout"],
			["failed", "failed"],
			["exploded", "failed"],
			[undefined, "failed"],
		];
		for (const [status, expected] of cases) {
			const run = reader();
			run.push([completedEvent({ status })]);
			expect(run.output().terminal.completedStatus).toBe(expected as never);
		}
	});

	test("a rejection stage, log level and duration come from completed", () => {
		const run = reader();
		run.push([
			completedEvent({
				status: "failed",
				current_step: "rejected:setup",
				log_level: 4,
				duration_ms: 0,
			}),
		]);
		expect(run.output().terminal).toMatchObject({
			rejectedStage: "setup",
			logLevel: 4,
			durationMs: 0,
		});
		const plain = reader();
		plain.push([completedEvent({ status: "failed", current_step: "Run OCR" })]);
		expect(plain.output().terminal.rejectedStage).toBeNull();
	});

	test("questions update by id and an answered one never goes back to pending", () => {
		const request = {
			id: "ask-1",
			name: "Approve?",
			description: "Approve invoice RE-2026-0917",
			interaction_type: {
				type: "single_choice",
				options: [{ id: "yes", label: "Yes" }],
			},
			status: "pending",
			ttl_seconds: 120,
			expires_at: 1_790_000_120,
			run_id: "run-1",
		};
		const run = reader();
		run.push([ev("interaction_request", request)]);
		expect(run.output().interactions).toEqual([request as never]);

		run.push([
			ev("interaction_request", {
				id: "ask-1",
				status: "responded",
				response_value: "yes",
			}),
		]);
		expect(run.output().interactions[0]).toMatchObject({
			name: "Approve?",
			status: "responded",
			response_value: "yes",
		});

		run.push([ev("interaction_request", { ...request })]);
		expect(run.output().interactions[0]?.status).toBe("responded");

		run.push([
			ev("interaction_request", { ...request, id: "ask-2", status: undefined }),
		]);
		expect(
			run.output().interactions.map((item) => [item.id, item.status]),
		).toEqual([
			["ask-1", "responded"],
			["ask-2", "pending"],
		]);
	});

	test("navigateTo intents are taken exactly once and leave the output alone", () => {
		const run = reader();
		run.push([
			ev("a2ui", {
				type: "navigateTo",
				route: "/orders?tab=open",
				replace: false,
			}),
			ev("a2ui", {
				type: "navigateTo",
				route: "/support",
				replace: true,
				queryParams: { id: "7", bad: 3 },
			}),
			ev("a2ui", {
				type: "navigateTo",
				route: "/legacy",
				query_params: { tab: "x" },
			}),
			ev("a2ui", { type: "upsertElement", element_id: "x", value: {} }),
			ev("a2ui", { type: "navigateTo" }),
		]);
		expect(run.takeNavigation()).toEqual([
			{ route: "/orders?tab=open", replace: false },
			{ route: "/support", replace: true, queryParams: { id: "7" } },
			{ route: "/legacy", replace: false, queryParams: { tab: "x" } },
		]);
		expect(run.takeNavigation()).toEqual([]);
		expect(run.output().eventCount).toBe(5);
		expect(run.output().answer).toBe("");

		run.push([
			ev("a2ui", { type: "navigateTo", route: "/next", replace: false }),
		]);
		expect(run.takeNavigation()).toEqual([{ route: "/next", replace: false }]);
	});

	test("reasoning without a plan is reasoning, not a step", () => {
		const run = reader();
		run.push([
			chunkEvent({ reasoning: "Looking at " }),
			chunkEvent({ reasoning: "the invoice" }),
		]);
		run.push([chunkEvent({ content: "Done." })]);
		const output = run.output();
		expect(output.steps).toEqual([]);
		expect(output.reasoning).toBe("Looking at the invoice");
		expect(output.answer).toBe("Done.");
	});

	test("with a plan, a Thinking step from the backend is a step like any other", () => {
		const run = reader();
		run.push([
			ev("chat_stream_partial", {
				chunk: null,
				attachments: [],
				plan: {
					plan: [
						[0, "Thinking"],
						[1, "Answer: Writing"],
					],
					current_step: 1,
					current_message: "",
				},
			}),
		]);
		expect(run.output().steps.map((step) => [step.title, step.state])).toEqual([
			["Thinking", "done"],
			["Answer", "active"],
		]);
		expect(run.output().reasoning).toBeNull();
	});

	test("chat_out finalises the plan", () => {
		const run = reader();
		run.push([stepEvent(["Read documents: Loading"])]);
		run.push([
			ev("chat_out", {
				response: {
					choices: [
						{
							index: 0,
							finish_reason: "stop",
							message: { role: "assistant", content: "All read." },
						},
					],
				},
				attachments: [],
			}),
		]);
		const output = run.output();
		expect(output.answer).toBe("All read.");
		expect(output.steps.map((step) => step.state)).toEqual(["done"]);
	});

	test("the output is a stable snapshot between pushes", () => {
		const run = reader();
		run.push([chunkEvent({ content: "a" })]);
		const first = run.output();
		expect(run.output()).toBe(first);
		run.push([]);
		expect(run.output()).toBe(first);
		run.push([chunkEvent({ content: "b" })]);
		const second = run.output();
		expect(second).not.toBe(first);
		expect(first.answer).toBe("a");
		expect(second.answer).toBe("ab");
	});

	test("malformed events neither throw nor stop the rest of the batch", () => {
		const run = reader();
		const broken = [
			null,
			{ event_id: "x", payload: {} },
			ev("chat_stream_partial", null),
			ev("chat_stream", { response: { choices: null } }),
			ev("chat_stream", { response: { choices: [{ index: 0 }] } }),
			ev("chat_stream_partial", { chunk: { choices: [null] } }),
			ev("completed", null),
			ev("interaction_request", { name: "no id" }),
			chunkEvent({ content: "kept" }),
		] as unknown as IIntercomEvent[];
		expect(() => run.push(broken)).not.toThrow();
		const output = run.output();
		expect(output.eventCount).toBe(7);
		expect(output.answer).toBe("kept");
		expect(output.terminal.completedStatus).toBe("failed");
		expect(output.interactions).toEqual([]);
	});

	test("a run that sent nothing", () => {
		const output = reader().output();
		expect(output).toEqual({
			eventCount: 0,
			steps: [],
			answer: "",
			reasoning: null,
			attachments: [],
			result: null,
			interactions: [],
			terminal: NO_TERMINAL_SIGNALS,
		});
	});
});
