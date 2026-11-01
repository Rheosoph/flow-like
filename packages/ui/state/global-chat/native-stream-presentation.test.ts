import { beforeEach, expect, test } from "bun:test";
import { useGlobalChatStore } from "./global-chat-store";
import { createNativeStreamPresentation } from "./native-stream-presentation";

beforeEach(() => {
	useGlobalChatStore.setState({
		runs: {},
		inlineAppSurfaces: [],
		messages: [],
		activeConversationId: "chat",
	});
	useGlobalChatStore.getState().startRun({
		runId: "run",
		conversationId: "chat",
		label: "Native run",
		selection: {
			runId: "run",
			provider: "bits",
			selectedModelId: "model",
			reasoningEffort: "",
		},
		message: null,
		sourceAttachments: [],
	});
});

test("nested specialist text stays out of the parent reply while its steps render", () => {
	const present = createNativeStreamPresentation("run");
	expect(present("Main reply")).toBe(false);
	expect(
		present(
			JSON.stringify({
				type: "native_specialist_event",
				requestId: "child",
				parentRequestId: "parent",
				frame:
					'Child reply<plan_step>{"id":"step","title":"Inspect board","status":"progress"}</plan_step>',
			}),
		),
	).toBe(true);
	expect(useGlobalChatStore.getState().runs.run.subPlanSteps[0]?.title).toBe(
		"↳ Inspect board",
	);
	present(
		JSON.stringify({
			type: "native_specialist_event",
			requestId: "child",
			frame: "",
			stage: "finished",
			status: "error",
		}),
	);
	expect(useGlobalChatStore.getState().runs.run.subPlanSteps[0]?.status).toBe(
		"failed",
	);
});

test("native frames from a completed run cannot modify another conversation", () => {
	const present = createNativeStreamPresentation("gone");
	expect(
		present(
			JSON.stringify({
				type: "native_specialist_event",
				requestId: "child",
				frame: '<plan_step>{"id":"x","title":"Wrong turn"}</plan_step>',
			}),
		),
	).toBe(true);
	expect(useGlobalChatStore.getState().runs.run.subPlanSteps).toEqual([]);
});

test("research subtools publish every source live and preserve context through completion", () => {
	const present = createNativeStreamPresentation("run");
	const send = (frame: string, stage?: string) =>
		present(
			JSON.stringify({
				type: "native_specialist_event",
				requestId: "research:agent",
				parentRequestId: "research",
				toolName: "research_agent",
				frame,
				stage,
				status: "completed",
			}),
		);
	send(
		`<tool_start>${JSON.stringify({
			tool_call_id: "search",
			tool: "internet_search",
			arguments_preview: JSON.stringify({ query: "Panamera comfort" }),
		})}</tool_start>`,
	);
	let steps = useGlobalChatStore.getState().runs.run.subPlanSteps;
	expect(steps).toHaveLength(1);
	expect(steps[0].status).toBe("progress");
	expect(steps[0].detail).toMatchObject({ queries: ["Panamera comfort"] });
	send(
		`<tool_end>${JSON.stringify({
			tool_call_id: "search",
			tool: "internet_search",
			status: "completed",
			result_preview: JSON.stringify({
				results: Array.from({ length: 8 }, (_, index) => ({
					url: `https://reviews.example/review-${index}`,
					title: `Review ${index}`,
				})),
			}),
		})}</tool_end>`,
	);
	send("", "finished");
	steps = useGlobalChatStore.getState().runs.run.subPlanSteps;
	expect(steps).toHaveLength(1);
	expect(steps[0].status).toBe("done");
	expect(steps[0].detail).toMatchObject({ queries: ["Panamera comfort"] });
	if (steps[0].detail?.kind !== "tool_activity")
		throw new Error("Missing research activity");
	expect(steps[0].detail.sources).toHaveLength(8);
	expect(steps[0].detail.sources?.at(-1)?.title).toBe("Review 7");
});
