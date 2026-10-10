import { afterEach, describe, expect, test } from "bun:test";
import {
	AIChatPlugin,
	AIPlugin,
	CopilotPlugin,
	triggerCopilotSuggestion,
} from "@platejs/ai/react";
import { NodeApi } from "platejs";
import { createPlateEditor } from "platejs/react";
import { type IHistoryMessage, IRole } from "../../../lib/schema/llm/history";
import type { IResponse } from "../../../lib/schema/llm/response";
import {
	type IBackendState,
	useBackendStore,
} from "../../../state/backend-state";
import type { IAIState } from "../../../state/backend-state/ai-state";
import { createCopilotKit } from "./copilot-kit";

const originalBackend = useBackendStore.getState().backend;
afterEach(() => useBackendStore.setState({ backend: originalBackend }));

const response = (text: string | null): IResponse => ({
	choices: [
		{
			index: 0,
			finish_reason: "stop",
			message: { role: "assistant", content: text },
		},
	],
	usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
});

const setup = (chatComplete: IAIState["chatComplete"]) => {
	useBackendStore.setState({
		backend: {
			aiState: { chatComplete },
			profile: { id: "profile-1" },
		} as IBackendState,
	});
	const editor = createPlateEditor({
		plugins: [AIPlugin, AIChatPlugin, ...createCopilotKit("app-1")],
		value: [{ id: "paragraph", type: "p", children: [{ text: "A test " }] }],
	});
	editor.tf.select({ path: [0, 0], offset: 7 });
	return editor;
};

describe("inline editor autocomplete", () => {
	test("uses the real completion, preserves app attribution, and accepts into the paragraph", async () => {
		const calls: Array<{ messages: IHistoryMessage[]; appId?: string }> = [];
		const editor = setup(async (messages, appId) => {
			calls.push({ messages, appId });
			return response("with **useful** results.");
		});

		await triggerCopilotSuggestion(editor);

		expect(calls).toHaveLength(1);
		expect(calls[0].appId).toBe("app-1");
		expect(calls[0].messages[0].role).toBe(IRole.System);
		expect(calls[0].messages[1]).toMatchObject({ role: IRole.User });
		expect(calls[0].messages[1].content).toContain("A test");
		expect(editor.getOption(CopilotPlugin, "suggestionText")).toBe(
			"with useful results.",
		);
		expect(editor.getOption(CopilotPlugin, "error")).toBeNull();
		editor.getTransforms(CopilotPlugin).copilot.accept();
		expect(NodeApi.string(editor)).toBe("A test with useful results.");
	});

	test("keeps the document and ghost text empty on backend failure", async () => {
		const error = new Error("No available profile model");
		const editor = setup(async () => {
			throw error;
		});

		await triggerCopilotSuggestion(editor);

		expect(editor.getOption(CopilotPlugin, "error")).toBe(error);
		expect(editor.getOption(CopilotPlugin, "suggestionText")).toBeNull();
		expect(NodeApi.string(editor)).toBe("A test ");
	});

	test.each([null, "", "0", " 0\n", " \n"])(
		"does not suggest empty or sentinel output %j",
		async (text) => {
			const editor = setup(async () => response(text));
			await triggerCopilotSuggestion(editor);
			expect(editor.getOption(CopilotPlugin, "suggestionText")).toBeNull();
			expect(editor.getOption(CopilotPlugin, "error")).toBeNull();
		},
	);

	test.each(["edit", "cursor", "stop", "reject", "backend", "profile"])(
		"ignores a late response after %s",
		async (change) => {
			let finish!: (result: IResponse) => void;
			const editor = setup(
				() =>
					new Promise((resolve) => {
						finish = resolve;
					}),
			);
			const pending = triggerCopilotSuggestion(editor);
			expect(editor.getOption(CopilotPlugin, "isLoading")).toBe(true);
			if (change === "edit") editor.tf.insertText("changed ");
			if (change === "cursor") editor.tf.select({ path: [0, 0], offset: 2 });
			if (change === "stop") editor.getApi(CopilotPlugin).copilot.stop();
			if (change === "reject") editor.getApi(CopilotPlugin).copilot.reject();
			if (change === "backend") useBackendStore.setState({ backend: null });
			if (change === "profile") {
				const profile = useBackendStore.getState().backend?.profile;
				if (profile) profile.id = "profile-2";
			}
			finish(response("old suggestion."));
			await pending;

			expect(editor.getOption(CopilotPlugin, "suggestionText")).toBeNull();
			expect(editor.getOption(CopilotPlugin, "error")).toBeNull();
			expect(editor.getOption(CopilotPlugin, "isLoading")).toBe(false);
		},
	);

	test("can suggest for new text while the cancelled backend call is still pending", async () => {
		let finishFirst!: (result: IResponse) => void;
		let calls = 0;
		const editor = setup(() => {
			calls += 1;
			if (calls > 1) return Promise.resolve(response("new suggestion."));
			return new Promise((resolve) => {
				finishFirst = resolve;
			});
		});
		const first = triggerCopilotSuggestion(editor);
		editor.tf.insertText("with updated context ");
		// Cancellation settles without waiting for the native model invocation.
		await first;
		await triggerCopilotSuggestion(editor);
		expect(calls).toBe(2);
		expect(editor.getOption(CopilotPlugin, "suggestionText")).toBe(
			"new suggestion.",
		);
		finishFirst(response("old suggestion."));
		await Promise.resolve();
		expect(editor.getOption(CopilotPlugin, "suggestionText")).toBe(
			"new suggestion.",
		);
	});
});
