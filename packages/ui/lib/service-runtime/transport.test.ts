import { describe, expect, test } from "bun:test";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import {
	consumeServiceStream,
	createServiceRequest,
	readServiceJson,
} from "./transport";

const bytes = (value: string) => new TextEncoder().encode(value);
const bodyOf = (text: string) => {
	const body = new Response(text).body;
	if (!body) throw new Error("Missing fixture response body");
	return body;
};

describe("shared service runtime transport", () => {
	test("bounds JSON responses and cancels an unfinished metadata body", async () => {
		let cancelled = false;
		const body = new ReadableStream<Uint8Array>({
			start(controller) {
				controller.enqueue(bytes('{"data":"123456789"}'));
			},
			cancel() {
				cancelled = true;
			},
		});
		await expect(
			readServiceJson(new Response(body), undefined, 10),
		).rejects.toThrow("size limit");
		expect(cancelled).toBe(true);
		const controller = new AbortController();
		const pending = readServiceJson(
			new Response(new ReadableStream()),
			controller.signal,
		);
		controller.abort();
		await expect(pending).rejects.toMatchObject({ name: "AbortError" });
	});

	test("maps rich events in wire order and stops after explicit completion", async () => {
		const events: IIntercomEvent[] = [];
		const mapped: unknown[] = [];
		const body = bodyOf(
			'event: a2ui\ndata: {"step":1}\n\nevent: chat_stream_partial\ndata: {"step":2}\n\nevent: done\ndata: {"completed":true}\n\nevent: a2ui\ndata: {"step":99}\n\n',
		);
		await consumeServiceStream(
			body,
			(batch) => events.push(...batch),
			undefined,
			{
				mapValue: async (value) => {
					await Promise.resolve();
					mapped.push(value);
					return value;
				},
			},
		);
		expect(mapped).toEqual([{ step: 1 }, { step: 2 }, { completed: true }]);
		expect(events.map((event) => event.event_type)).toEqual([
			"a2ui",
			"chat_stream_partial",
			"completed",
		]);
	});

	test("rejects changing run identity and cleans up mapper errors", async () => {
		await expect(
			consumeServiceStream(
				bodyOf(
					'event: run_initiated\ndata: {"run_id":"one"}\n\nevent: run_initiated\ndata: {"run_id":"two"}\n\n',
				),
			),
		).rejects.toThrow("invalid run identifier");
		let cancelled = false;
		const stream = new ReadableStream<Uint8Array>({
			start(controller) {
				controller.enqueue(bytes("event: a2ui\ndata: {}\n\n"));
			},
			cancel() {
				cancelled = true;
			},
		});
		await expect(
			consumeServiceStream(stream, undefined, undefined, {
				mapValue: async () => {
					throw new Error("Resource expired");
				},
			}),
		).rejects.toThrow("Resource expired");
		expect(cancelled).toBe(true);
	});

	test("scopes bearer access to asset capabilities and app uploads", async () => {
		const calls: string[] = [];
		const request = createServiceRequest("t".repeat(32), async (path, init) => {
			calls.push(String(path));
			expect(new Headers(init?.headers).get("Authorization")).toBe(
				`Bearer ${"t".repeat(32)}`,
			);
			return new Response();
		});
		await request(`/ui/assets/${"a".repeat(32)}`);
		await request(`/ui/assets/${"b".repeat(32)}/${"a".repeat(32)}`);
		await request("/ui/assets?store=upload&path=media%2Fimage.png");
		for (const path of [
			"/ui/assets?url=file:///secret",
			"/ui/assets?store=storage&path=secret",
			"/ui/assets?store=upload&path=media&store=storage",
			"/ui/assets/../secret",
			"/ui/assets/short",
			`/ui/assets/${"b".repeat(32)}/${"a".repeat(32)}/extra`,
			`/ui/assets/${"x".repeat(32)}/${"a".repeat(32)}`,
			"/ui/assets?path=media",
			"/ui/assets?store=upload&path=media&token=x",
		])
			await expect(request(path)).rejects.toThrow("Unsupported");
		expect(calls).toHaveLength(3);
	});
});
