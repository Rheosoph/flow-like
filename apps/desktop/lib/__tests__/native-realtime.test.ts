import { afterEach, describe, expect, test, vi } from "vitest";

const { invoke, channels } = vi.hoisted(() => ({
	invoke: vi.fn(),
	channels: [] as Array<{ onmessage: (message: unknown) => void }>,
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke,
	Channel: class {
		onmessage: (message: unknown) => void = () => {};
		constructor() {
			channels.push(this);
		}
	},
}));

import { watchNativeRealtime } from "../native-realtime";

const options = () => ({
	profileId: "profile-a",
	subject: "user-a",
	issuer: "https://issuer.test",
	request: { notifications: {}, runs: [], timeout_ms: 20_000 },
	onMessage: vi.fn(),
});

afterEach(() => {
	vi.useRealTimers();
	invoke.mockReset();
	channels.length = 0;
});

describe("native realtime subscriptions", () => {
	test.each([
		"The authentication session changed",
		"Sign in to subscribe to realtime updates",
	])("stops registration when native reports %s", async (reason) => {
		vi.useFakeTimers();
		invoke.mockRejectedValue(reason);
		const input = options();
		const close = watchNativeRealtime(input);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(input.onMessage).toHaveBeenCalledWith({
			type: "state",
			state: "signed_out",
		});
		expect(
			invoke.mock.calls.filter(([command]) => command === "realtime_subscribe"),
		).toHaveLength(1);
		close();
	});
	const update = {
		type: "update",
		delivery_id: "delivery-one",
		response: { protocol_version: 1, runs: [], errors: [] },
	};
	const acknowledgments = () =>
		invoke.mock.calls.filter(([name]) => name === "realtime_acknowledge");

	test("retries failed consumers and acknowledges only successful processing", async () => {
		vi.useFakeTimers();
		invoke.mockResolvedValue(undefined);
		const input = options();
		input.onMessage.mockRejectedValueOnce(new Error("Consumer failed"));
		const close = watchNativeRealtime(input);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		expect(input.onMessage).toHaveBeenCalledOnce();
		expect(acknowledgments()).toHaveLength(0);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		expect(input.onMessage).toHaveBeenCalledTimes(2);
		expect(acknowledgments()).toHaveLength(1);
		expect(acknowledgments()[0][1]).toEqual({
			subscriptionId: invoke.mock.calls[0][1].subscriptionId,
			deliveryId: "delivery-one",
		});
		close();
	});

	test("retries a lost acknowledgment without processing the accepted batch twice", async () => {
		vi.useFakeTimers();
		let fail = true;
		invoke.mockImplementation((command) => {
			if (command === "realtime_acknowledge" && fail) {
				fail = false;
				return Promise.reject(new Error("IPC interrupted"));
			}
			return Promise.resolve();
		});
		const input = options();
		const close = watchNativeRealtime(input);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		expect(input.onMessage).toHaveBeenCalledOnce();
		expect(acknowledgments()).toHaveLength(2);
		close();
	});

	test("keeps one consumer in flight and never acknowledges it after cleanup", async () => {
		vi.useFakeTimers();
		invoke.mockResolvedValue(undefined);
		const input = options();
		let finish!: () => void;
		input.onMessage.mockImplementation(
			() =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		);
		const close = watchNativeRealtime(input);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		channels[0].onmessage(update);
		channels[0].onmessage({ ...update, delivery_id: "another" });
		await vi.advanceTimersByTimeAsync(0);
		expect(input.onMessage).toHaveBeenCalledOnce();
		close();
		finish();
		await vi.advanceTimersByTimeAsync(0);
		expect(acknowledgments()).toHaveLength(0);
	});

	test("hands over a reply while a batch is in flight and never acknowledges it", async () => {
		vi.useFakeTimers();
		invoke.mockResolvedValue(undefined);
		const input = options();
		input.onMessage.mockImplementationOnce(() => new Promise<void>(() => {}));
		const close = watchNativeRealtime(input);
		channels[0].onmessage(update);
		await vi.advanceTimersByTimeAsync(0);
		const reply = {
			type: "reply",
			reply: { conversation_id: "c-1", message_id: "re-1", text: "Hello" },
		};
		input.onMessage.mockImplementationOnce(() => {
			throw new Error("Consumer failed");
		});
		channels[0].onmessage(reply);
		channels[0].onmessage(reply);
		await vi.advanceTimersByTimeAsync(0);
		expect(input.onMessage.mock.calls.slice(1)).toEqual([[reply], [reply]]);
		expect(acknowledgments()).toHaveLength(0);
		close();
		channels[0].onmessage(reply);
		expect(input.onMessage).toHaveBeenCalledTimes(3);
	});

	test("supplies expected identity and stops accepting callbacks after cleanup", async () => {
		invoke.mockResolvedValue(undefined);
		const input = options();
		const close = watchNativeRealtime(input);
		expect(invoke).toHaveBeenCalledWith(
			"realtime_subscribe",
			expect.objectContaining({
				profileId: "profile-a",
				expectedSubject: "user-a",
				expectedIssuer: "https://issuer.test",
				transport: "configured",
			}),
		);
		const message = { type: "state", state: "connected" };
		const deliver = channels[0].onmessage;
		deliver(message);
		expect(input.onMessage).toHaveBeenCalledOnce();
		close();
		deliver(message);
		expect(input.onMessage).toHaveBeenCalledOnce();
		await Promise.resolve();
	});

	test("preserves an explicit HTTP preference for the native connection owner", async () => {
		invoke.mockResolvedValue(undefined);
		const close = watchNativeRealtime({ ...options(), transport: "http" });
		expect(invoke).toHaveBeenCalledWith(
			"realtime_subscribe",
			expect.objectContaining({ transport: "http" }),
		);
		close();
		await Promise.resolve();
	});

	test("removes a subscription that finishes registering after its window detached", async () => {
		let registered!: () => void;
		invoke.mockImplementation((command) =>
			command === "realtime_subscribe"
				? new Promise<void>((resolve) => {
						registered = resolve;
					})
				: Promise.resolve(),
		);
		const close = watchNativeRealtime(options());
		close();
		expect(
			invoke.mock.calls.filter(([name]) => name === "realtime_unsubscribe"),
		).toHaveLength(1);
		registered();
		await vi.waitFor(() =>
			expect(
				invoke.mock.calls.filter(([name]) => name === "realtime_unsubscribe"),
			).toHaveLength(2),
		);
	});

	test("retries startup failures but cancels the retry on cleanup", async () => {
		vi.useFakeTimers();
		invoke.mockRejectedValue(new Error("Native session is loading"));
		const input = options();
		const close = watchNativeRealtime(input);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(
			invoke.mock.calls.filter(([name]) => name === "realtime_subscribe"),
		).toHaveLength(2);
		close();
		await vi.advanceTimersByTimeAsync(60_000);
		expect(
			invoke.mock.calls.filter(([name]) => name === "realtime_subscribe"),
		).toHaveLength(2);
	});

	test("older native builds do not cause repeated unsupported IPC calls", async () => {
		vi.useFakeTimers();
		invoke.mockRejectedValue(new Error("Command realtime_subscribe not found"));
		const input = options();
		const close = watchNativeRealtime(input);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(input.onMessage).toHaveBeenCalledWith({
			type: "state",
			state: "unsupported",
		});
		expect(
			invoke.mock.calls.filter(([name]) => name === "realtime_subscribe"),
		).toHaveLength(1);
		close();
	});
});
