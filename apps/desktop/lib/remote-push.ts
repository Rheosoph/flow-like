import { invoke } from "@tauri-apps/api/core";
import type { IPushNotificationsConfig } from "@flow-like/flow-like-ui";
import { isAndroidDevice, isIOSDevice, isTauriRuntime } from "./platform";

export type PushTargetPlatform = "IOS" | "ANDROID" | "DESKTOP";

export type RemotePushPayload = {
	title?: string;
	body?: string;
	data: Record<string, unknown>;
	badge?: number;
	sound?: string;
	channelId?: string;
	category?: string;
};

export type RemotePushListener = {
	unregister: () => Promise<void> | void;
};

export type RemotePushApi = {
	getToken: () => Promise<string>;
	requestPermission: () => Promise<{ granted: boolean }>;
	onNotificationReceived: (
		handler: (notification: RemotePushPayload) => void,
	) => Promise<RemotePushListener>;
	onNotificationTapped: (
		handler: (notification: RemotePushPayload) => void,
	) => Promise<RemotePushListener>;
	onTokenRefresh: (
		handler: (token: string) => void,
	) => Promise<RemotePushListener>;
};

const DEVICE_ID_STORAGE_KEY = "flow-like-push-device-id";
const DEVICE_ID_FILE = "push-device-id.txt";
const REMOTE_PUSH_ENABLED_STORAGE_KEY = "flow-like-remote-push-enabled";

export const REMOTE_PUSH_PREFERENCE_EVENT =
	"flow-like:remote-push-preference-changed";

export async function loadRemotePushPlugin(): Promise<RemotePushApi | null> {
	try {
		const mod = await import("tauri-plugin-remote-push-api");
		return {
			getToken: mod.getToken,
			requestPermission: mod.requestPermission,
			onNotificationReceived: mod.onNotificationReceived,
			onNotificationTapped: mod.onNotificationTapped,
			onTokenRefresh: mod.onTokenRefresh,
		};
	} catch {
		return null;
	}
}

async function loadPersistentDeviceId(): Promise<string | null> {
	try {
		const { readTextFile, BaseDirectory } = await import(
			"@tauri-apps/plugin-fs"
		);
		const id = await readTextFile(DEVICE_ID_FILE, {
			baseDir: BaseDirectory.AppData,
		});
		return id?.trim() || null;
	} catch {
		return null;
	}
}

async function savePersistentDeviceId(id: string): Promise<void> {
	try {
		const { writeTextFile, mkdir, BaseDirectory } = await import(
			"@tauri-apps/plugin-fs"
		);
		await mkdir("", { baseDir: BaseDirectory.AppData, recursive: true }).catch(
			() => {},
		);
		await writeTextFile(DEVICE_ID_FILE, id, {
			baseDir: BaseDirectory.AppData,
		});
	} catch {
		// FS is unavailable in browser-like shells; localStorage is still used.
	}
}

async function loadNativeDeviceId(): Promise<string | null> {
	try {
		const id = await invoke<string | null>("get_stable_device_id");
		return typeof id === "string" && id.trim().length > 0 ? id.trim() : null;
	} catch {
		return null;
	}
}

export async function getPushDeviceId(): Promise<string> {
	if (typeof window === "undefined") {
		return "server-device";
	}

	const native = await loadNativeDeviceId();
	if (native) {
		window.localStorage.setItem(DEVICE_ID_STORAGE_KEY, native);
		await savePersistentDeviceId(native);
		return native;
	}

	const persisted = await loadPersistentDeviceId();
	if (persisted) {
		window.localStorage.setItem(DEVICE_ID_STORAGE_KEY, persisted);
		return persisted;
	}

	const existing = window.localStorage.getItem(DEVICE_ID_STORAGE_KEY);
	if (existing) {
		await savePersistentDeviceId(existing);
		return existing;
	}

	const created = crypto.randomUUID();
	window.localStorage.setItem(DEVICE_ID_STORAGE_KEY, created);
	await savePersistentDeviceId(created);
	return created;
}

export function detectPushPlatform(): PushTargetPlatform | null {
	if (isAndroidDevice()) {
		return "ANDROID";
	}
	if (isIOSDevice()) {
		return "IOS";
	}
	if (isTauriRuntime()) {
		return "DESKTOP";
	}

	return null;
}

export function canUseRemotePushForPlatform(
	pushConfig: IPushNotificationsConfig | undefined,
	platform: PushTargetPlatform | null,
): boolean {
	if (!pushConfig?.enabled || pushConfig.provider !== "fcm" || !platform) {
		return false;
	}

	if (platform === "DESKTOP") {
		return pushConfig.allow_desktop === true;
	}

	return pushConfig.allow_mobile === true;
}

export function isRemotePushPreferenceEnabled(): boolean {
	if (typeof window === "undefined") {
		return true;
	}
	return window.localStorage.getItem(REMOTE_PUSH_ENABLED_STORAGE_KEY) !== "false";
}

export function setRemotePushPreference(enabled: boolean): void {
	if (typeof window === "undefined") {
		return;
	}

	window.localStorage.setItem(
		REMOTE_PUSH_ENABLED_STORAGE_KEY,
		enabled ? "true" : "false",
	);
	window.dispatchEvent(
		new CustomEvent(REMOTE_PUSH_PREFERENCE_EVENT, {
			detail: { enabled },
		}),
	);
}
