import { beforeEach, expect, test } from "bun:test";
import { createNativeStreamPresentation } from "./native-stream-presentation";
import { useGlobalChatStore } from "./global-chat-store";

beforeEach(() => {
	useGlobalChatStore.setState({
		runs: {},
		inlineAppSurfaces: [],
		messages: [],
		activeConversationId: "chat",
	});
	useGlobalChatStore
		.getState()
		.startRun({
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
