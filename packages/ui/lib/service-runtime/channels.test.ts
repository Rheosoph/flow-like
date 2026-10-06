import { afterEach, describe, expect, test } from "bun:test";
import { cancelChannel, replyToChannel, steerChannel } from "../channel";
import type { IChannelHandle } from "../schema/channel";
import { createRuntimeChannels } from "./channels";

const cleanups: Array<() => void> = [];
afterEach(() => {
	for (const close of cleanups.splice(0)) close();
});
const handle = (token = "a".repeat(43)): IChannelHandle => ({
	channel_id: "run-1",
	request_id: "question-1",
	expires_at: Date.now() / 1000 + 300,
	transport: { type: "http", push_url: "/channels/incarnation/run-1", token },
	fallback: {
		type: "http",
		push_url: "https://hub.test/push",
		token: "hub-secret",
	},
});
function runtime() {
	const calls: Array<{ path: string; init: RequestInit }> = [];
	const channels = createRuntimeChannels((async (input, init) => {
		calls.push({ path: String(input), init: init ?? {} });
		return new Response(null, { status: 204 });
	}) as typeof fetch);
	cleanups.push(channels.close);
	return { channels, calls };
}

describe("runtime reply channels", () => {
	test("routes serialized replies, cancellation and steering through their originating session", async () => {
		const a = runtime();
		const b = runtime();
		const boundA = JSON.parse(
			JSON.stringify(a.channels.bind(handle())),
		) as IChannelHandle;
		const boundB = b.channels.bind(handle("b".repeat(43)));
		await replyToChannel(boundA, { approved: true });
		await cancelChannel(boundB);
		await steerChannel(boundA, "continue");
		expect(a.calls).toHaveLength(2);
		expect(b.calls).toHaveLength(1);
		expect(a.calls[0].path).toBe("/channels/incarnation/run-1");
		expect(new Headers(a.calls[0].init.headers).get("authorization")).toBe(
			`Bearer ${"a".repeat(43)}`,
		);
		expect(JSON.parse(String(a.calls[0].init.body))).toMatchObject({
			channel_id: "run-1",
			request_id: "question-1",
			kind: "reply",
			value: { approved: true },
		});
		expect(boundA.fallback).toBeUndefined();
	});

	test("closed handles cannot use hub fallback or another session's transport", async () => {
		const a = runtime();
		const b = runtime();
		const boundA = a.channels.bind(handle());
		const boundB = b.channels.bind(handle("b".repeat(43)));
		a.channels.close();
		boundA.fallback = handle().fallback;
		await expect(replyToChannel(boundA, 1)).rejects.toThrow(
			/session has ended/,
		);
		const crossed = {
			...boundB,
			transport: { ...boundB.transport, token: "a".repeat(43) },
		} as IChannelHandle;
		await expect(replyToChannel(crossed, 1)).rejects.toThrow(
			/invalid or expired/,
		);
		expect(a.calls).toHaveLength(0);
		expect(b.calls).toHaveLength(0);
	});

	test("rejects foreign URLs, traversal, token shape and another run ID before registration", () => {
		const { channels } = runtime();
		for (const push_url of [
			"https://hub.test/push",
			"/channels/other-run",
			"/channels/../run-1",
			"/channels/%2e%2e/run-1",
			"//hub.test/run-1",
		]) {
			const input = handle();
			input.transport = { type: "http", token: "a".repeat(43), push_url };
			expect(() => channels.bind({ nested: [input] })).toThrow(/invalid/);
		}
		expect(() => channels.bind(handle("bad"))).toThrow(/invalid/);
	});

	test("closing the runtime cancels an in-flight interactive reply", async () => {
		let started!: () => void;
		const ready = new Promise<void>((resolve) => {
			started = resolve;
		});
		const channels = createRuntimeChannels((async (_, init) => {
			started();
			return new Promise<Response>((_, reject) => {
				init?.signal?.addEventListener(
					"abort",
					() => reject(new Error("reply aborted")),
					{ once: true },
				);
			});
		}) as typeof fetch);
		cleanups.push(channels.close);
		const pending = replyToChannel(channels.bind(handle()), 1);
		await ready;
		channels.close();
		await expect(pending).rejects.toThrow(/aborted/);
	});
});
