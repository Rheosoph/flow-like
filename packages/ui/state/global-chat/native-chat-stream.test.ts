import { beforeEach, expect, mock, test } from "bun:test";
import {
	type NativeActionRequest,
	type NativeActionResult,
	assertNativeActionCurrent,
	nativeChatOutcome,
	runNativeAction,
} from "../../lib/native-action-result";
import { IRole } from "../../lib/schema/llm/history";
import { useGlobalChatStore } from "./global-chat-store";
import {
	driveGlobalChatStream,
	makeGlobalChatMessage,
	tauriStart,
} from "./global-chat-stream";

const invoke = mock(async (_command: string, _args: unknown) => undefined);
mock.module("@tauri-apps/api/core", () => ({
	Channel: class {
		onmessage?: (message: string) => void;
	},
	invoke,
}));

beforeEach(() => {
	invoke.mockClear();
	useGlobalChatStore.setState({
		activeConversationId: "native-conversation",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		nativeDrafts: [],
	});
});

async function deliver(
	start: (onChunk: (chunk: string) => void) => Promise<unknown>,
) {
	const request: NativeActionRequest = {
		id: "native-request",
		scope: "workspace-a",
		responseMode: "text",
		responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	};
	const complete = mock(
		async (_command: string, _args: { result: NativeActionResult }) => {},
	);
	const responseMessage = makeGlobalChatMessage(
		IRole.Assistant,
		"",
		"native-conversation",
	);
	await runNativeAction(request, {
		getCurrentScope: () => request.scope,
		invoke: complete,
		run: async () => {
			await driveGlobalChatStream({
				responseMessage,
				agentSelection: {
					provider: "bits",
					selectedModelId: "model",
					reasoningEffort: "",
				},
				start,
			});
			return nativeChatOutcome(responseMessage);
		},
	});
	return complete.mock.calls[0][1].result;
}

test("real FlowPilot stream returns only completed answer through native completion", async () => {
	const answer = "Your order arrives Tuesday.";
	const result = await deliver(async (onChunk) => {
		onChunk(
			'<plan_step>{"id":"private-plan","title":"Internal reasoning"}</plan_step>',
		);
		onChunk(
			'<tool_start>{"tool_call_id":"private-tool","tool":"lookup","summary":"Private diagnostic"}</tool_start>',
		);
		onChunk("Your order ");
		onChunk("arrives Tuesday.");
		onChunk(
			'<tool_end>{"tool_call_id":"private-tool","status":"success"}</tool_end>',
		);
		return { message: "Final fallback" };
	});
	expect(result).toMatchObject({ status: "success", text: answer });
	expect(JSON.stringify(result)).not.toContain("private");
	expect(useGlobalChatStore.getState().messages[0].inner.content).toBe(answer);
});

test("nonstreaming backend result is returned to Shortcuts", async () => {
	const result = await deliver(async () => ({ message: "Saved the file." }));
	expect(result).toMatchObject({ status: "success", text: "Saved the file." });
});

test("transport failure after partial text returns a native error", async () => {
	const result = await deliver(async (onChunk) => {
		onChunk("An incomplete response");
		throw new Error("Network unavailable");
	});
	expect(result.status).toBe("error");
	expect(result.text).toBeUndefined();
	expect(result.error).toBeTruthy();
});

test("identity is checked after the lazy Tauri import before sending the question", async () => {
	const request = { id: "lazy", scope: "workspace-a" };
	let currentScope = request.scope;
	const pending = tauriStart(
		"global_chat",
		{ userPrompt: "Private question" },
		() => assertNativeActionCurrent(request, currentScope),
	)(() => {});
	currentScope = "workspace-b";
	await expect(pending).rejects.toThrow("different account");
	expect(invoke).not.toHaveBeenCalled();
});
