import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	isIOSDevice: vi.fn(),
	isAndroidDevice: vi.fn(),
	isTauriRuntime: vi.fn(),
	getToken: vi.fn(),
	requestPermission: vi.fn(),
	onNotificationReceived: vi.fn(),
	onNotificationTapped: vi.fn(),
	onTokenRefresh: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../platform", () => ({
	isIOSDevice: mocks.isIOSDevice,
	isAndroidDevice: mocks.isAndroidDevice,
	isTauriRuntime: mocks.isTauriRuntime,
}));
vi.mock("tauri-plugin-remote-push-api", () => ({
	getToken: mocks.getToken,
	requestPermission: mocks.requestPermission,
	onNotificationReceived: mocks.onNotificationReceived,
	onNotificationTapped: mocks.onNotificationTapped,
	onTokenRefresh: mocks.onTokenRefresh,
}));

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
	for (let i = 0; i < 10; i++) await Promise.resolve();
}

async function loadApi() {
	const { loadRemotePushPlugin } = await import("../remote-push");
	const api = await loadRemotePushPlugin();
	if (!api) throw new Error("Expected a remote push API");
	return api;
}

beforeEach(() => {
	vi.resetModules();
	vi.resetAllMocks();
	vi.useFakeTimers();
	mocks.isTauriRuntime.mockReturnValue(true);
	mocks.isIOSDevice.mockReturnValue(true);
	mocks.isAndroidDevice.mockReturnValue(false);
});

afterEach(() => {
	vi.clearAllTimers();
	vi.useRealTimers();
});

describe("remote push native bridge", () => {
	it("acquires iOS tokens through the app's native FCM command", async () => {
		mocks.invoke.mockResolvedValue("native-fcm-token");
		const api = await loadApi();
		await expect(api.getToken()).resolves.toBe("native-fcm-token");
		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
			"get_remote_push_token",
		);
		expect(mocks.getToken).not.toHaveBeenCalled();
		expect(vi.getTimerCount()).toBe(0);
	});

	it("keeps Android token acquisition on the plugin", async () => {
		mocks.isIOSDevice.mockReturnValue(false);
		mocks.isAndroidDevice.mockReturnValue(true);
		mocks.getToken.mockResolvedValue("android-fcm-token");
		const api = await loadApi();
		await expect(api.getToken()).resolves.toBe("android-fcm-token");
		expect(mocks.getToken).toHaveBeenCalledTimes(1);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	it("shares a native request between independently loaded API wrappers", async () => {
		const token = deferred<string>();
		mocks.invoke.mockReturnValue(token.promise);
		const startup = await loadApi();
		const settings = await loadApi();
		const first = startup.getToken();
		const second = settings.getToken();
		expect(second).toBe(first);
		await flush();
		expect(mocks.invoke).toHaveBeenCalledTimes(1);
		token.resolve("shared-token");
		await expect(Promise.all([first, second])).resolves.toEqual([
			"shared-token",
			"shared-token",
		]);
	});

	it("times out after 25 seconds and allows a new request", async () => {
		const firstNative = deferred<string>();
		mocks.invoke.mockReturnValueOnce(firstNative.promise);
		const api = await loadApi();
		const first = api.getToken();
		const rejected = expect(first).rejects.toThrow(
			"Push registration timed out",
		);
		let settled = false;
		void first.then(
			() => {
				settled = true;
			},
			() => {
				settled = true;
			},
		);
		await vi.advanceTimersByTimeAsync(24_999);
		expect(settled).toBe(false);
		await vi.advanceTimersByTimeAsync(1);
		await rejected;
		mocks.invoke.mockResolvedValueOnce("recovered-token");
		await expect(api.getToken()).resolves.toBe("recovered-token");
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
	});

	it.each(["resolve", "reject"] as const)(
		"ignores an old native request that later %ss while a new request is pending",
		async (completion) => {
			const oldNative = deferred<string>();
			const newNative = deferred<string>();
			mocks.invoke
				.mockReturnValueOnce(oldNative.promise)
				.mockReturnValueOnce(newNative.promise);
			const api = await loadApi();
			const oldRequest = api.getToken();
			const rejected = expect(oldRequest).rejects.toThrow("timed out");
			await vi.advanceTimersByTimeAsync(25_000);
			await rejected;
			const newRequest = api.getToken();
			await flush();
			if (completion === "resolve") oldNative.resolve("obsolete-token");
			else oldNative.reject(new Error("Old native failure"));
			await flush();
			expect(api.getToken()).toBe(newRequest);
			expect(mocks.invoke).toHaveBeenCalledTimes(2);
			expect(vi.getTimerCount()).toBe(1);
			newNative.resolve("current-token");
			await expect(newRequest).resolves.toBe("current-token");
			expect(vi.getTimerCount()).toBe(0);
		},
	);

	it("allows retry after the native command rejects", async () => {
		mocks.invoke
			.mockRejectedValueOnce(new Error("FCM registration unavailable"))
			.mockResolvedValueOnce("recovered-token");
		const api = await loadApi();
		await expect(api.getToken()).rejects.toThrow(
			"FCM registration unavailable",
		);
		await expect(api.getToken()).resolves.toBe("recovered-token");
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		expect(vi.getTimerCount()).toBe(0);
	});

	it.each([true, false, null])(
		"reads the current native permission value %s",
		async (permission) => {
			mocks.invoke.mockResolvedValue(permission);
			const { getRemotePushPermission } = await import("../remote-push");
			await expect(getRemotePushPermission()).resolves.toBe(permission);
			expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
				"plugin:notification|is_permission_granted",
			);
		},
	);

	it("does not expose the native push API outside Tauri", async () => {
		mocks.isTauriRuntime.mockReturnValue(false);
		const { loadRemotePushPlugin } = await import("../remote-push");
		await expect(loadRemotePushPlugin()).resolves.toBeNull();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
