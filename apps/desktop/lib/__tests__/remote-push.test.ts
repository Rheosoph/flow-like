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

function stubWindow() {
	const store = new Map<string, string>();
	const dispatchEvent = vi.fn();
	vi.stubGlobal("window", {
		localStorage: {
			getItem: (key: string) => store.get(key) ?? null,
			setItem: (key: string, value: string) => store.set(key, value),
		},
		dispatchEvent,
	});
	return { store, dispatchEvent };
}

const enabledStatus = {
	device_id: "device",
	registered: true,
	push_enabled: true,
	eligible: true,
	stale: false,
};

const PROMPT_CASES = [
	[
		"a refusal from an earlier launch",
		{ permissionAtLaunch: false, preferenceEnabled: true, status: null },
		"permission",
	],
	[
		"a refusal while this device's switch is off",
		{ permissionAtLaunch: false, preferenceEnabled: false, status: null },
		"permission",
	],
	[
		"this device's switch being off",
		{ permissionAtLaunch: true, preferenceEnabled: false, status: null },
		"disabled",
	],
	[
		"a registered target the server keeps off",
		{
			permissionAtLaunch: true,
			preferenceEnabled: true,
			status: { ...enabledStatus, push_enabled: false },
		},
		"disabled",
	],
	[
		"a permission the OS has not asked for yet",
		{ permissionAtLaunch: null, preferenceEnabled: true, status: null },
		null,
	],
	[
		"an enabled target",
		{
			permissionAtLaunch: true,
			preferenceEnabled: true,
			status: enabledStatus,
		},
		null,
	],
	[
		"a device that is not registered yet",
		{
			permissionAtLaunch: true,
			preferenceEnabled: true,
			status: { ...enabledStatus, registered: false, push_enabled: false },
		},
		null,
	],
] as const;

describe("remote push re-prompt", () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it.each(PROMPT_CASES)(
		"for %s gives reason %s",
		async (_, state, expected) => {
			const { remotePushPromptReason } = await import("../remote-push");
			expect(remotePushPromptReason(state)).toBe(expected);
		},
	);

	it("remembers a declined prompt", async () => {
		stubWindow();
		const { declineRemotePushPrompt, hasDeclinedRemotePushPrompt } =
			await import("../remote-push");
		expect(hasDeclinedRemotePushPrompt()).toBe(false);
		declineRemotePushPrompt();
		expect(hasDeclinedRemotePushPrompt()).toBe(true);
	});

	it("treats missing storage as not declined", async () => {
		const { declineRemotePushPrompt, hasDeclinedRemotePushPrompt } =
			await import("../remote-push");
		expect(() => declineRemotePushPrompt()).not.toThrow();
		expect(hasDeclinedRemotePushPrompt()).toBe(false);
	});

	it("opens the system notification settings natively", async () => {
		mocks.invoke.mockResolvedValue(undefined);
		const { openNotificationSettings } = await import("../remote-push");
		await openNotificationSettings();
		expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
			"native_open_notification_settings",
		);
	});
});

describe("enableRemotePush", () => {
	const pushConfig = {
		enabled: true,
		provider: "fcm",
		allow_mobile: true,
		channel_id: "default",
	} as never;

	function userState() {
		return {
			registerPushTarget: vi.fn().mockResolvedValue({ success: true }),
			setPushTargetEnabled: vi.fn().mockResolvedValue(enabledStatus),
		};
	}

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it("registers the token, switches delivery on and saves the preference", async () => {
		const { store, dispatchEvent } = stubWindow();
		mocks.requestPermission.mockResolvedValue({ granted: true });
		mocks.invoke.mockResolvedValue("ios-token");
		const state = userState();
		const { enableRemotePush } = await import("../remote-push");

		await expect(
			enableRemotePush({
				userState: state as never,
				pushConfig,
				deviceId: "device-1",
				source: "prompt",
			}),
		).resolves.toBe(enabledStatus);

		expect(state.registerPushTarget).toHaveBeenCalledWith(
			expect.objectContaining({
				device_id: "device-1",
				platform: "IOS",
				token: "ios-token",
				channel_id: "default",
				metadata: expect.objectContaining({ source: "prompt" }),
			}),
		);
		expect(state.setPushTargetEnabled).toHaveBeenCalledWith("device-1", true);
		expect(store.get("flow-like-remote-push-enabled")).toBe("true");
		expect(dispatchEvent).toHaveBeenCalledTimes(1);
	});

	it("stops with a permission error before touching the server", async () => {
		stubWindow();
		mocks.requestPermission.mockResolvedValue({ granted: false });
		const state = userState();
		const { enableRemotePush, RemotePushPermissionDeniedError } = await import(
			"../remote-push"
		);

		await expect(
			enableRemotePush({
				userState: state as never,
				pushConfig,
				deviceId: "device-1",
				source: "prompt",
			}),
		).rejects.toBeInstanceOf(RemotePushPermissionDeniedError);
		expect(state.registerPushTarget).not.toHaveBeenCalled();
		expect(state.setPushTargetEnabled).not.toHaveBeenCalled();
	});
});
