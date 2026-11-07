import { describe, expect, it, vi } from "vitest";
import { createEmotionDispatcher } from "./emotion-dispatch";
import type { EmotionWorkerRequest, EmotionWorkerResponse } from "./protocol";
import { expressionScores } from "./scoring";

function port() {
	const messages: EmotionWorkerResponse[] = [];
	return {
		messages,
		postMessage: (message: EmotionWorkerResponse) => messages.push(message),
		onmessage: null as
			| ((event: MessageEvent<EmotionWorkerRequest>) => void)
			| null,
	};
}

describe("emotion dispatcher", () => {
	it("keeps colliding request ids on the tab that sent them", async () => {
		const first = port();
		const second = port();
		const dispatcher = createEmotionDispatcher(async (input, reply) => {
			reply({
				type: "result",
				id: input.id,
				scores: expressionScores({ joy: input.draft === "left" ? 0.2 : 0.9 }),
			});
		});
		dispatcher.attach(first);
		dispatcher.attach(second);
		first.onmessage?.({
			data: { id: 1, draft: "left", messages: [] },
		} as MessageEvent<EmotionWorkerRequest>);
		second.onmessage?.({
			data: { id: 1, draft: "right", messages: [] },
		} as MessageEvent<EmotionWorkerRequest>);
		await vi.waitFor(() => {
			expect(first.messages).toHaveLength(1);
			expect(second.messages).toHaveLength(1);
		});
		expect(first.messages[0]).toMatchObject({
			type: "result",
			id: 1,
			scores: expect.objectContaining({ happy: 0.2 }),
		});
		expect(second.messages[0]).toMatchObject({
			type: "result",
			id: 1,
			scores: expect.objectContaining({ happy: 0.9 }),
		});
	});

	it("broadcasts load progress and drops a port that can no longer receive", async () => {
		const alive = port();
		const dead = port();
		dead.postMessage = () => {
			throw new Error("port closed");
		};
		const dispatcher = createEmotionDispatcher(async (_input, reply) => {
			dispatcher.broadcast({ type: "progress", progress: 0.5 });
			reply({ type: "ready" });
		});
		dispatcher.attach(alive);
		dispatcher.attach(dead);
		alive.onmessage?.({
			data: { id: 0, warmup: true, draft: "", messages: [] },
		} as MessageEvent<EmotionWorkerRequest>);
		await vi.waitFor(() => expect(alive.messages).toHaveLength(2));
		expect(alive.messages.map((message) => message.type)).toEqual([
			"progress",
			"ready",
		]);
	});
});
