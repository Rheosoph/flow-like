// @vitest-environment happy-dom

import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { RemotePushRegistrationOptions } from "../remote-push-registration";

const mocks = vi.hoisted(() => ({
	preference: true,
	auth: {
		isAuthenticated: true,
		user: { profile: { sub: "test-user" }, access_token: "access-token" },
	},
	backend: { profile: { hub: "https://hub.example.test" } },
	hub: {
		hub: {
			push_notifications: {
				enabled: true,
				provider: "fcm",
				allow_mobile: true,
			},
		},
	},
	queryClient: { refetchQueries: vi.fn() },
	router: { push: vi.fn() },
	invoke: vi.fn(),
	listen: vi.fn(),
	fetcher: vi.fn(),
	tapUnregister: vi.fn(),
	startRegistration: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	useBackend: () => mocks.backend,
	useHub: () => mocks.hub,
}));
vi.mock("@flow-like/flow-like-ui/lib/client-navigation", () => ({
	useClientRouter: () => mocks.router,
}));
vi.mock(
	"@flow-like/flow-like-ui/components/notifications/notification-icon",
	() => ({ NotificationIcon: () => null }),
);
vi.mock("@flow-like/flow-like-ui/lib/notification-icon", () => ({
	remoteNotificationIcon: () => undefined,
}));
vi.mock("@tanstack/react-query", () => ({
	useQueryClient: () => mocks.queryClient,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/plugin-notification", () => ({
	isPermissionGranted: async () => true,
	requestPermission: async () => "granted",
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("sonner", () => ({ toast: { info: vi.fn() } }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher }));
vi.mock("../notification-presentation", () => ({
	presentLocalNotification: vi.fn(),
	shouldShowRemotePushToast: () => false,
}));
vi.mock("../notifications-db", () => ({ addLocalNotification: vi.fn() }));
vi.mock("../remote-push", () => ({
	REMOTE_PUSH_PREFERENCE_EVENT: "test:push-preference",
	REMOTE_PUSH_REGISTRATION_EVENT: "test:push-registered",
	canUseRemotePushForPlatform: () => true,
	detectPushPlatform: () => "IOS",
	getPushDeviceId: async () => "test-device",
	isRemotePushPreferenceEnabled: () => mocks.preference,
	loadRemotePushPlugin: async () => ({
		onNotificationTapped: async () => ({ unregister: mocks.tapUnregister }),
	}),
}));
vi.mock("../remote-push-registration", () => ({
	startRemotePushRegistration: mocks.startRegistration,
}));

import NotificationProvider from "../../components/notification-provider";

let root: Root;
let mounted: boolean;
let registrations: {
	options: RemotePushRegistrationOptions;
	refresh: ReturnType<typeof vi.fn>;
	stop: ReturnType<typeof vi.fn>;
}[];

beforeEach(() => {
	vi.clearAllMocks();
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	mocks.preference = true;
	mocks.invoke.mockResolvedValue(null);
	mocks.listen.mockResolvedValue(() => {});
	mocks.fetcher.mockResolvedValue({ id: "push-target", success: true });
	registrations = [];
	mocks.startRegistration.mockImplementation(
		(options: RemotePushRegistrationOptions) => {
			const registration = { options, refresh: vi.fn(), stop: vi.fn() };
			registrations.push(registration);
			return registration;
		},
	);
	root = createRoot(document.createElement("div"));
	mounted = true;
	vi.spyOn(console, "log").mockImplementation(() => {});
});

afterEach(async () => {
	if (mounted) await act(async () => root.unmount());
	vi.restoreAllMocks();
});

async function mount() {
	await act(async () => {
		root.render(createElement(NotificationProvider));
	});
	expect(registrations).toHaveLength(1);
}

function setPreference(enabled: boolean) {
	mocks.preference = enabled;
	window.dispatchEvent(new Event("test:push-preference"));
}

function lastRegistrationSignal(): AbortSignal {
	const options = mocks.fetcher.mock.lastCall?.[2] as
		| { signal: AbortSignal }
		| undefined;
	if (!options) throw new Error("Expected a push registration request");
	return options.signal;
}

test("restarts registration after a disable failure rolls back in one React batch", async () => {
	await mount();
	const original = registrations[0];
	await original.options.register("original-token");
	const originalSignal = lastRegistrationSignal();
	expect(originalSignal.aborted).toBe(false);

	await act(async () => {
		setPreference(false);
		expect(original.stop).toHaveBeenCalledTimes(1);
		expect(originalSignal.aborted).toBe(true);
		await Promise.reject(new Error("Disable request failed")).catch(() => {
			setPreference(true);
		});
	});

	expect(registrations).toHaveLength(2);
	const restarted = registrations[1];
	await restarted.options.register("current-token");
	expect(lastRegistrationSignal().aborted).toBe(false);
	expect(lastRegistrationSignal()).not.toBe(originalSignal);
	window.dispatchEvent(new Event("focus"));
	expect(restarted.refresh).toHaveBeenCalledTimes(1);
	expect(original.refresh).not.toHaveBeenCalled();
});

test("unmount stops registration, aborts its request and removes refresh listeners", async () => {
	const clearInterval = vi.spyOn(window, "clearInterval");
	await mount();
	const registration = registrations[0];
	mocks.fetcher.mockImplementationOnce(
		(_profile, _path, options: { signal: AbortSignal }) =>
			new Promise((_resolve, reject) => {
				options.signal.addEventListener("abort", () => {
					reject(new Error("Registration cancelled"));
				});
			}),
	);
	const request = registration.options.register("token");
	const cancelled = expect(request).rejects.toThrow("Registration cancelled");
	const signal = lastRegistrationSignal();

	await act(async () => root.unmount());
	await cancelled;
	mounted = false;
	expect(registration.stop).toHaveBeenCalledTimes(1);
	expect(signal.aborted).toBe(true);
	expect(mocks.tapUnregister).toHaveBeenCalledTimes(1);
	expect(clearInterval).toHaveBeenCalledTimes(1);
	window.dispatchEvent(new Event("focus"));
	window.dispatchEvent(new Event("online"));
	document.dispatchEvent(new Event("visibilitychange"));
	expect(registration.refresh).not.toHaveBeenCalled();
});
