import { afterEach, describe, expect, it, vi } from "vitest";
import type { RemotePushApi, RemotePushPayload } from "../remote-push";
import { startRemotePushRegistration } from "../remote-push-registration";

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((resolvePromise, rejectPromise) => {
		resolve = resolvePromise;
		reject = rejectPromise;
	});
	return { promise, resolve, reject };
}

async function flush() {
	for (let i = 0; i < 20; i++) await Promise.resolve();
}

function setup() {
	vi.useFakeTimers();
	let tokenHandler: (token: string) => void = () => {};
	let notificationHandler: (notification: RemotePushPayload) => void = () => {};
	const tokenListener = { unregister: vi.fn() };
	const notificationListener = { unregister: vi.fn() };
	const api = {
		requestPermission: vi.fn(async () => ({ granted: true })),
		getToken: vi.fn(async () => "original-token"),
		onTokenRefresh: vi.fn(async (handler: (token: string) => void) => {
			tokenHandler = handler;
			return tokenListener;
		}),
		onNotificationReceived: vi.fn(
			async (handler: (notification: RemotePushPayload) => void) => {
				notificationHandler = handler;
				return notificationListener;
			},
		),
		onNotificationTapped: vi.fn(async () => ({ unregister() {} })),
	} satisfies RemotePushApi;
	const register = vi.fn(async (_token: string) => {});
	const onNotification = vi.fn();
	const onError = vi.fn();
	return {
		api,
		register,
		onNotification,
		onError,
		tokenListener,
		notificationListener,
		emitToken: (token: string) => tokenHandler(token),
		emitNotification: (notification: RemotePushPayload) =>
			notificationHandler(notification),
		start: () =>
			startRemotePushRegistration({
				api,
				register,
				onNotification,
				onError,
				retryBaseDelayMs: 100,
				retryMaxDelayMs: 400,
			}),
	};
}

afterEach(() => {
	vi.useRealTimers();
});

describe("remote push registration", () => {
	it("subscribes before requesting permission or acquiring a token", async () => {
		const test = setup();
		const lifecycle = test.start();
		await flush();
		expect(test.api.onTokenRefresh.mock.invocationCallOrder[0]).toBeLessThan(
			test.api.requestPermission.mock.invocationCallOrder[0],
		);
		expect(
			test.api.onNotificationReceived.mock.invocationCallOrder[0],
		).toBeLessThan(test.api.requestPermission.mock.invocationCallOrder[0]);
		expect(test.register).toHaveBeenCalledExactlyOnceWith("original-token");
		lifecycle.stop();
	});

	it("keeps listeners active after token acquisition fails and accepts recovery", async () => {
		const test = setup();
		test.api.getToken.mockRejectedValueOnce(new Error("Token not ready"));
		const lifecycle = test.start();
		await flush();
		expect(test.onError).toHaveBeenCalledTimes(1);
		const notification = { title: "Received", data: {} };
		test.emitNotification(notification);
		test.emitToken("recovered-token");
		await flush();
		expect(test.onNotification).toHaveBeenCalledWith(notification);
		expect(test.register).toHaveBeenCalledExactlyOnceWith("recovered-token");
		await vi.advanceTimersByTimeAsync(1_000);
		expect(test.api.getToken).toHaveBeenCalledTimes(1);
		expect(test.register).toHaveBeenCalledTimes(1);
		lifecycle.stop();
	});

	it("retries failed token acquisition", async () => {
		const test = setup();
		test.api.getToken.mockRejectedValueOnce(new Error("Token not ready"));
		const lifecycle = test.start();
		await flush();
		await vi.advanceTimersByTimeAsync(100);
		expect(test.api.getToken).toHaveBeenCalledTimes(2);
		expect(test.register).toHaveBeenCalledExactlyOnceWith("original-token");
		lifecycle.stop();
	});

	it("serializes registrations and coalesces token rotations during a POST", async () => {
		const test = setup();
		const firstRegistration = deferred<void>();
		test.register.mockImplementationOnce(() => firstRegistration.promise);
		const lifecycle = test.start();
		await flush();
		test.emitToken("intermediate-token");
		test.emitToken("latest-token");
		await flush();
		expect(test.register).toHaveBeenCalledTimes(1);
		firstRegistration.resolve();
		await flush();
		expect(test.register.mock.calls).toEqual([
			["original-token"],
			["latest-token"],
		]);
		lifecycle.stop();
	});

	it("uses a refresh event instead of an older in-flight token result", async () => {
		const test = setup();
		const token = deferred<string>();
		test.api.getToken.mockReturnValueOnce(token.promise);
		const lifecycle = test.start();
		await flush();
		test.emitToken("latest-token");
		token.resolve("stale-token");
		await flush();
		expect(test.register).toHaveBeenCalledExactlyOnceWith("latest-token");
		lifecycle.stop();
	});

	it("accepts a fresh token acquired after an earlier permission-time event", async () => {
		const test = setup();
		const permission = deferred<{ granted: boolean }>();
		test.api.requestPermission.mockReturnValueOnce(permission.promise);
		test.api.getToken.mockResolvedValue("fresh-native-token");
		const lifecycle = test.start();
		await flush();
		test.emitToken("earlier-plugin-token");
		permission.resolve({ granted: true });
		await flush();
		expect(test.register).toHaveBeenCalledExactlyOnceWith("fresh-native-token");
		expect(test.register).not.toHaveBeenCalledWith("earlier-plugin-token");
		lifecycle.stop();
	});

	it("retries registration with capped backoff without reacquiring the token", async () => {
		const test = setup();
		test.register.mockRejectedValue(new Error("Network unavailable"));
		const lifecycle = test.start();
		await flush();
		expect(test.register).toHaveBeenCalledTimes(1);
		for (const [delay, expectedCalls] of [
			[100, 2],
			[200, 3],
			[400, 4],
			[400, 5],
		]) {
			await vi.advanceTimersByTimeAsync(delay - 1);
			expect(test.register).toHaveBeenCalledTimes(expectedCalls - 1);
			await vi.advanceTimersByTimeAsync(1);
			expect(test.register).toHaveBeenCalledTimes(expectedCalls);
		}
		expect(test.api.getToken).toHaveBeenCalledTimes(1);
		lifecycle.stop();
		await vi.advanceTimersByTimeAsync(1_000);
		expect(test.register).toHaveBeenCalledTimes(5);
	});

	it("refreshes server registration even when the token has not changed", async () => {
		const test = setup();
		const lifecycle = test.start();
		await flush();
		lifecycle.refresh();
		await flush();
		expect(test.api.getToken).toHaveBeenCalledTimes(2);
		expect(test.register.mock.calls).toEqual([
			["original-token"],
			["original-token"],
		]);
		lifecycle.stop();
	});

	it("does not retry denied permission until an explicit refresh", async () => {
		const test = setup();
		test.api.requestPermission.mockResolvedValueOnce({ granted: false });
		const lifecycle = test.start();
		await flush();
		test.emitToken("token-while-denied");
		await vi.advanceTimersByTimeAsync(10_000);
		expect(test.api.requestPermission).toHaveBeenCalledTimes(1);
		expect(test.api.getToken).not.toHaveBeenCalled();
		expect(test.register).not.toHaveBeenCalled();
		lifecycle.refresh();
		await flush();
		expect(test.register).toHaveBeenCalledExactlyOnceWith("original-token");
		lifecycle.stop();
	});

	it("unregisters a listener that resolves after stop", async () => {
		const test = setup();
		const subscription = deferred<typeof test.tokenListener>();
		test.api.onTokenRefresh.mockReturnValueOnce(subscription.promise);
		const lifecycle = test.start();
		lifecycle.stop();
		subscription.resolve(test.tokenListener);
		await flush();
		expect(test.tokenListener.unregister).toHaveBeenCalledTimes(1);
		expect(test.api.onNotificationReceived).not.toHaveBeenCalled();
		expect(test.api.requestPermission).not.toHaveBeenCalled();
	});

	it("ignores token results and callbacks after stop", async () => {
		const test = setup();
		const token = deferred<string>();
		test.api.getToken.mockReturnValueOnce(token.promise);
		const lifecycle = test.start();
		await flush();
		lifecycle.stop();
		token.resolve("late-token");
		test.emitToken("late-refresh");
		test.emitNotification({ data: {} });
		lifecycle.refresh();
		await flush();
		expect(test.register).not.toHaveBeenCalled();
		expect(test.onNotification).not.toHaveBeenCalled();
		expect(test.tokenListener.unregister).toHaveBeenCalledTimes(1);
		expect(test.notificationListener.unregister).toHaveBeenCalledTimes(1);
	});

	it("retries a failed subscription without duplicating existing listeners", async () => {
		const test = setup();
		test.api.onNotificationReceived.mockRejectedValueOnce(
			new Error("Listener unavailable"),
		);
		const lifecycle = test.start();
		await flush();
		expect(test.api.getToken).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(100);
		expect(test.api.onTokenRefresh).toHaveBeenCalledTimes(1);
		expect(test.api.onNotificationReceived).toHaveBeenCalledTimes(2);
		expect(test.register).toHaveBeenCalledExactlyOnceWith("original-token");
		lifecycle.stop();
	});
});
