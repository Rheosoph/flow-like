import { describe, expect, it } from "bun:test";
import { consumeHostedStream } from "./hosted-stream";

function stream(chunks: string[]) {
	const encoder = new TextEncoder();
	return new ReadableStream<Uint8Array>({
		start(controller) {
			for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
			controller.close();
		},
	});
}

describe("hosted execution stream", () => {
	it("preserves token whitespace and terminal-batch events across split CRLF frames", async () => {
		const events: unknown[] = [];
		let runId: string | undefined;
		await consumeHostedStream(
			stream([
				': heartbeat\r\n\r\ndata: {"event_type":"run_initiated","payload":{"run_id":"run-1"}}\r',
				'\n\r\ndata: {"event_type":"stream_text","payload":"  hello  "}\r\n\r\ndata: {"event_type":"completed"}\r\n\r\ndata: {"event_type":"usage","payload":4}\r\n\r\n',
			]),
			(batch) => events.push(...batch),
			(id) => {
				runId = id;
			},
		);
		expect(runId).toBe("run-1");
		expect(events).toHaveLength(4);
		expect(events[1]).toEqual({
			event_type: "stream_text",
			payload: "  hello  ",
		});
	});
	it("reports failed runs and interrupted streams", async () => {
		await expect(
			consumeHostedStream(
				stream([
					'data: {"event_type":"completed","payload":{"status":"failed"}}\n\n',
				]),
			),
		).rejects.toThrow("status: failed");
		await expect(
			consumeHostedStream(
				stream([
					'data: {"event_type":"completed","payload":{"status":"cancelled"}}\n\n',
				]),
			),
		).rejects.toThrow("status: cancelled");
		await expect(
			consumeHostedStream(
				stream([
					'data: {"event_type":"error","payload":{"message":"Budget exhausted"}}\n\n',
				]),
			),
		).rejects.toThrow("Budget exhausted");
		await expect(
			consumeHostedStream(
				stream(['data: {"event_type":"stream_text","payload":"partial"}\n\n']),
			),
		).rejects.toThrow("before the workflow completed");
	});
});
