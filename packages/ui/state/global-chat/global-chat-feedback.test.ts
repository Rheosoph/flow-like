import { describe, expect, test } from "bun:test";
import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import { IRole } from "../../lib/schema/llm/history";
import {
	FLOWPILOT_RATING_NEGATIVE,
	FLOWPILOT_RATING_POSITIVE,
	FLOWPILOT_RATING_WITHDRAWN,
	buildFlowPilotFeedbackContext,
	flowPilotRatingForUi,
	messageText,
} from "./global-chat-feedback";

const SESSION = "conv-1";

function message(
	id: string,
	role: IRole,
	content: string,
	timestamp: number,
	extra: Partial<IMessage> = {},
): IMessage {
	return {
		id,
		appId: "global",
		sessionId: SESSION,
		inner: { role, content },
		files: [],
		timestamp,
		...extra,
	};
}

describe("messageText", () => {
	test("reads both the string and the structured content shapes", () => {
		expect(messageText(message("a", IRole.User, "plain", 1))).toBe("plain");

		const structured = message("b", IRole.Assistant, "", 2);
		structured.inner = {
			role: IRole.Assistant,
			content: [
				{ type: "text", text: "first" } as never,
				{ type: "image_url" } as never,
				{ type: "text", text: "second" } as never,
			],
		};
		expect(messageText(structured)).toBe("first\nsecond");
	});
});

describe("buildFlowPilotFeedbackContext", () => {
	test("attributes the prompt to the stamped user message, not the newest one", () => {
		// Steering commits extra user messages with LATER timestamps than the reply they steered,
		// so a naive backwards scan credits the wrong prompt.
		const prompt = message("user-1", IRole.User, "build me a CRM", 100);
		const steer = message("user-2", IRole.User, "actually make it blue", 300);
		const reply = message("run-1", IRole.Assistant, "done", 200, {
			run_context: {
				schema: "flowpilot.run-context/v1",
				user_message_id: "user-1",
			},
		});

		const context = buildFlowPilotFeedbackContext(reply, [
			prompt,
			reply,
			steer,
		]);

		expect(context.prompt).toBe("build me a CRM");
		expect(context.prompt_message_id).toBe("user-1");
		expect(context.response).toBe("done");
		expect(context.conversation_id).toBe(SESSION);
	});

	test("falls back to the preceding user turn for messages stored before run contexts existed", () => {
		const prompt = message("user-1", IRole.User, "older prompt", 100);
		const reply = message("run-1", IRole.Assistant, "older reply", 200);

		const context = buildFlowPilotFeedbackContext(reply, [prompt, reply]);

		expect(context.prompt).toBe("older prompt");
		expect(context.prompt_message_id).toBe("user-1");
	});

	test("folds usage across the parent turn and its specialists", () => {
		const reply = message("run-1", IRole.Assistant, "done", 200, {
			usage_stats: [
				{
					step_name: "main",
					stats: {
						usage: {
							prompt_tokens: 10,
							completion_tokens: 5,
							total_tokens: 15,
							cost: 0.01,
						},
						calls: [
							{
								model: "gpt-5",
								usage: {
									prompt_tokens: 10,
									completion_tokens: 5,
									total_tokens: 15,
								},
							},
						],
					},
				},
				{
					step_name: "specialist",
					stats: {
						usage: {
							prompt_tokens: 4,
							completion_tokens: 1,
							total_tokens: 5,
						},
						model: "haiku",
					},
				},
			],
		});

		const usage = buildFlowPilotFeedbackContext(reply, [reply]).usage;

		expect(usage?.total_tokens).toBe(20);
		expect(usage?.prompt_tokens).toBe(14);
		expect(usage?.cost).toBeCloseTo(0.01);
		// The top-level `model` is only set by some backends and the per-call entries by others, so
		// both are collected rather than trusting either alone.
		expect(usage?.models.sort()).toEqual(["gpt-5", "haiku"]);
	});

	test("attaches the transcript only when the user opted in", () => {
		const prompt = message("user-1", IRole.User, "hi", 100);
		const reply = message("run-1", IRole.Assistant, "hello", 200);

		expect(
			buildFlowPilotFeedbackContext(reply, [prompt, reply]).transcript,
		).toBeUndefined();

		const shared = buildFlowPilotFeedbackContext(reply, [prompt, reply], {
			includeTranscript: true,
			canContact: true,
		});
		expect(shared.transcript).toHaveLength(2);
		expect(shared.transcript?.[0]?.role).toBe("user");
		expect(shared.can_contact).toBe(true);
	});

	test("stays inside the route's size limit for a pathological turn", () => {
		// Every field is individually capped, so this asserts the caps actually compose: a huge
		// conversation, a huge reply, a hundred plan steps and a hundred app refs together must still
		// produce a body the route accepts, or the rating is lost to a silent 400.
		const filler = "x".repeat(50_000);
		const history = Array.from({ length: 60 }, (_, index) =>
			message(`hist-${index}`, IRole.User, filler, index + 1),
		);
		const reply = message("run-1", IRole.Assistant, filler, 500, {
			plan_steps: Array.from({ length: 100 }, (_, index) => ({
				id: String(index),
				title: filler,
				status: "done" as const,
				toolName: `tool-${index}`,
			})),
			app_refs: Array.from({ length: 100 }, (_, index) => `app-${index}`),
		});

		const context = buildFlowPilotFeedbackContext(reply, [...history, reply], {
			includeTranscript: true,
		});

		expect(JSON.stringify(context).length).toBeLessThanOrEqual(256 * 1024);
		expect(context.transcript?.length).toBeLessThanOrEqual(24);
		expect(context.steps?.length).toBe(40);
		expect(context.app_refs?.length).toBe(20);
		// The rating still carries the beginning of what was actually said.
		expect(context.response.startsWith("xxx")).toBe(true);
	});

	test("carries the plan steps and tools that describe what the turn did", () => {
		const reply = message("run-1", IRole.Assistant, "done", 200, {
			plan_steps: [
				{ id: "1", title: "Plan the board", status: "done", toolName: "plan" },
				{ id: "2", title: "Write nodes", status: "done", toolName: "write" },
				{ id: "3", title: "Write more", status: "done", toolName: "write" },
			],
			app_refs: ["app-1"],
		});

		const context = buildFlowPilotFeedbackContext(reply, [reply]);

		expect(context.steps).toEqual([
			"Plan the board",
			"Write nodes",
			"Write more",
		]);
		expect(context.tools).toEqual(["plan", "write"]);
		expect(context.app_refs).toEqual(["app-1"]);
	});
});

describe("flowPilotRatingForUi", () => {
	test("maps the UI's signed thumb onto the stored unsigned 0..5 scale", () => {
		expect(flowPilotRatingForUi(1)).toBe(FLOWPILOT_RATING_POSITIVE);
		expect(flowPilotRatingForUi(-1)).toBe(FLOWPILOT_RATING_NEGATIVE);
		expect(flowPilotRatingForUi(0)).toBe(FLOWPILOT_RATING_WITHDRAWN);
		expect(flowPilotRatingForUi(undefined)).toBe(FLOWPILOT_RATING_WITHDRAWN);
	});
});
