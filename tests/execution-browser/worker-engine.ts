import {
	chatDb,
	decodeMessageRow,
} from "../../packages/ui/components/interfaces/chat-default/chat-db";
import { getExecutionRuntime } from "../../packages/ui/lib/execution/runtime-client";
import { IRole } from "../../packages/ui/lib/schema/llm/history";
import type { FixtureExecutionEngine } from "./execution-context";

export async function createWorkerFixtureEngine(scope: string) {
	const client = await getExecutionRuntime(scope);
	const outcomes: Record<string, string> = {};
	const engine: FixtureExecutionEngine = {
		getTasks: () => client.getTasks(),
		subscribeToGlobalUpdates: (listener) => client.subscribe(listener),
		cancelTask: (taskId) => client.cancel(taskId),
		dismissTask: (taskId) => {
			void client.dismiss(taskId);
		},
		replyToTask: async (taskId, requestId, value) => {
			const request = client
				.getTask(taskId)
				?.pendingInteractions.find((item) => item.requestId === requestId);
			if (!request?.channel)
				throw new Error("The question has no reply channel");
			await client.submitReply(request.channel, value);
		},
	};
	return {
		engine,
		client,
		outcomes,
		async start(
			taskId: string,
			options: { delayRunId?: boolean; title?: string } = {},
		) {
			await chatDb.sessions.put({
				id: taskId,
				appId: "fixture-app",
				summarization: options.title || "Background research",
				createdAt: Date.now(),
				updatedAt: Date.now(),
			});
			void client
				.execute(
					{
						taskId,
						streamId: taskId,
						scope,
						appId: "fixture-app",
						eventId: "fixture-chat",
						title: options.title || "Background research",
						path: `/chat/${taskId}`,
						interfaceType: "chat",
						startedAt: new Date().toISOString(),
						chat: {
							responseMessage: {
								id: `reply-${taskId}`,
								appId: "fixture-app",
								sessionId: taskId,
								inner: { role: IRole.Assistant, content: "" },
								timestamp: Date.now(),
								files: [],
								actions: [],
								tools: [],
							},
						},
					},
					{
						url: `${location.origin}/execution-fixture/start`,
						headers: { "content-type": "application/json" },
						body: { taskId, delayRunId: options.delayRunId },
						cancelUrl: `${location.origin}/execution-fixture/cancel/{runId}`,
						timeoutMs: 10_000,
					},
				)
				.then(
					() => {
						outcomes[taskId] = "completed";
					},
					(error) => {
						outcomes[taskId] = error.message;
					},
				);
		},
		async history(sessionId: string) {
			return (
				await chatDb.messages.where("sessionId").equals(sessionId).toArray()
			).map(decodeMessageRow);
		},
	};
}
