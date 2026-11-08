import { describe, expect, it, vi } from "vitest";
import {
	type EmotionDispatchMessage,
	createEmotionDispatcher,
} from "./emotion-dispatch";
import type { EmotionWorkerResponse } from "./protocol";
import { expressionScores } from "./scoring";

function event(data: EmotionDispatchMessage) {
	return new MessageEvent<EmotionDispatchMessage>("message", { data });
}

function port() {
	const messages: EmotionWorkerResponse[] = [];
	return {
		messages,
		postMessage: (message: EmotionWorkerResponse) => messages.push(message),
		close: vi.fn(),
		onmessage: null as
			| ((event: MessageEvent<EmotionDispatchMessage>) => void)
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
		first.onmessage?.(event({ id: 1, draft: "left", messages: [] }));
		second.onmessage?.(event({ id: 1, draft: "right", messages: [] }));
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
		alive.onmessage?.(event({ id: 0, warmup: true, draft: "", messages: [] }));
		await vi.waitFor(() => expect(alive.messages).toHaveLength(2));
		expect(alive.messages.map((message) => message.type)).toEqual([
			"progress",
			"ready",
		]);
		expect(dead.close).toHaveBeenCalledOnce();
	});

	it("detaches closed tabs before broadcast and skips their queued inference", async () => {
		const first = port();
		const closed = port();
		let finishFirst!: () => void;
		const blocked = new Promise<void>((resolve) => {
			finishFirst = resolve;
		});
		const drafts: string[] = [];
		const dispatcher = createEmotionDispatcher(async (input, reply) => {
			drafts.push(input.draft);
			if (input.draft === "first") await blocked;
			reply({ type: "ready" });
		});
		dispatcher.attach(first);
		dispatcher.attach(closed);
		first.onmessage?.(event({ id: 1, draft: "first", messages: [] }));
		closed.onmessage?.(event({ id: 1, draft: "closed", messages: [] }));
		await vi.waitFor(() => expect(drafts).toEqual(["first"]));
		closed.onmessage?.(event({ type: "disconnect" }));
		dispatcher.broadcast({ type: "progress", progress: 0.5 });
		expect(closed.messages).toEqual([]);
		expect(closed.close).toHaveBeenCalledOnce();
		expect(closed.onmessage).toBeNull();
		first.onmessage?.(event({ id: 2, draft: "last", messages: [] }));
		finishFirst();
		await vi.waitFor(() => expect(drafts).toEqual(["first", "last"]));
		expect(closed.messages).toEqual([]);
	});

	it("continues queued requests after one inference rejects", async () => {
		const failed = port();
		const alive = port();
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		try {
			const dispatcher = createEmotionDispatcher(async (input, reply) => {
				if (input.draft === "fail") throw new Error("inference failed");
				reply({ type: "ready" });
			});
			dispatcher.attach(failed);
			dispatcher.attach(alive);
			failed.onmessage?.(event({ id: 1, draft: "fail", messages: [] }));
			alive.onmessage?.(event({ id: 1, draft: "alive", messages: [] }));
			await vi.waitFor(() =>
				expect(alive.messages).toEqual([{ type: "ready" }]),
			);
			expect(failed.messages).toEqual([{ type: "error", id: 1 }]);
		} finally {
			log.mockRestore();
		}
	});
});
