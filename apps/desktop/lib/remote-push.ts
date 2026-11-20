import type {
	IPushNotificationsConfig,
	IPushTargetStatus,
	IUserState,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";
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
const REMOTE_PUSH_PROMPT_DECLINED_STORAGE_KEY =
	"flow-like-remote-push-prompt-declined";

export const REMOTE_PUSH_PREFERENCE_EVENT =
	"flow-like:remote-push-preference-changed";
export const REMOTE_PUSH_REGISTRATION_EVENT =
	"flow-like:remote-push-registered";

const TOKEN_REQUEST_TIMEOUT_MS = 25_000;
let pendingTokenRequest: Promise<string> | undefined;

function getRemotePushToken(
	getPluginToken: () => Promise<string>,
): Promise<string> {
	if (pendingTokenRequest) return pendingTokenRequest;

	// Settings and startup can request a token together. Share the native request
	// and bound the wait so an unavailable push service cannot leave settings busy.
	const request = new Promise<string>((resolve, reject) => {
		const timeout = setTimeout(
			() =>
				reject(
					new Error(
						"Push registration timed out. Check your connection and try again.",
					),
				),
			TOKEN_REQUEST_TIMEOUT_MS,
		);
		Promise.resolve()
			.then(() =>
				isIOSDevice()
					? invoke<string>("get_remote_push_token")
					: getPluginToken(),
			)
			.then(resolve, reject)
			.finally(() => clearTimeout(timeout));
	});
	pendingTokenRequest = request.finally(() => {
		pendingTokenRequest = undefined;
	});
	return pendingTokenRequest;
}

export async function loadRemotePushPlugin(): Promise<RemotePushApi | null> {
	if (!isTauriRuntime()) return null;
	try {
		const mod = await import("tauri-plugin-remote-push-api");
		return {
			getToken: () => getRemotePushToken(mod.getToken),
			requestPermission: mod.requestPermission,
			onNotificationReceived: mod.onNotificationReceived,
			onNotificationTapped: mod.onNotificationTapped,
			onTokenRefresh: mod.onTokenRefresh,
		};
	} catch {
		return null;
	}
}

export async function getRemotePushPermission(): Promise<boolean | null> {
	// Query native settings so returning from iOS Settings does not use the
	// notification plugin's cached WebView permission value.
	return invoke<boolean | null>("plugin:notification|is_permission_granted");
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

	const native = isTauriRuntime() ? await loadNativeDeviceId() : null;
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
	return (
		window.localStorage.getItem(REMOTE_PUSH_ENABLED_STORAGE_KEY) !== "false"
	);
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

export type RemotePushPromptReason = "permission" | "disabled";

export interface RemotePushPromptState {
	/** OS permission before this launch could ask: false = refused, null = never asked. */
	permissionAtLaunch: boolean | null;
	preferenceEnabled: boolean;
	status: IPushTargetStatus | null;
}

export function remotePushPromptReason({
	permissionAtLaunch,
	preferenceEnabled,
	status,
}: RemotePushPromptState): RemotePushPromptReason | null {
	if (permissionAtLaunch === false) return "permission";
	if (!preferenceEnabled) return "disabled";
	if (status?.registered && !status.push_enabled) return "disabled";
	return null;
}

export function hasDeclinedRemotePushPrompt(): boolean {
	try {
		return (
			window.localStorage.getItem(REMOTE_PUSH_PROMPT_DECLINED_STORAGE_KEY) ===
			"true"
		);
	} catch {
		return false;
	}
}

export function declineRemotePushPrompt(): void {
	try {
		window.localStorage.setItem(
			REMOTE_PUSH_PROMPT_DECLINED_STORAGE_KEY,
			"true",
		);
	} catch {
		// Without storage the prompt may return next launch; nothing else breaks.
	}
}

export function openNotificationSettings(): Promise<void> {
	return invoke<void>("native_open_notification_settings");
}

export class RemotePushPermissionDeniedError extends Error {
	constructor() {
		super(
			"Allow notifications for Flow Like in your device Settings, then try again.",
		);
		this.name = "RemotePushPermissionDeniedError";
	}
}

export interface EnableRemotePushOptions {
	userState: IUserState;
	pushConfig: IPushNotificationsConfig | undefined;
	deviceId: string;
	source: "settings" | "prompt";
}

function requireRemotePushPlatform(
	pushConfig: IPushNotificationsConfig | undefined,
) {
	const platform = detectPushPlatform();
	if (!platform) {
		throw new Error("Push platform could not be detected.");
	}
	if (!canUseRemotePushForPlatform(pushConfig, platform)) {
		throw new Error("Remote push is not enabled for this platform.");
	}
	return platform;
}

async function acquireGrantedPushToken() {
	const remotePushApi = await loadRemotePushPlugin();
	if (!remotePushApi) {
		throw new Error("Remote push plugin is not available.");
	}
	const permission = await remotePushApi.requestPermission();
	if (!permission.granted) {
		throw new RemotePushPermissionDeniedError();
	}
	const token = await remotePushApi.getToken();
	if (!token) {
		throw new Error("Remote push returned an empty token.");
	}
	return token;
}

/** Asks for the OS permission, registers this device and switches server delivery on. */
export async function enableRemotePush({
	userState,
	pushConfig,
	deviceId,
	source,
}: EnableRemotePushOptions): Promise<IPushTargetStatus> {
	const platform = requireRemotePushPlatform(pushConfig);
	const token = await acquireGrantedPushToken();

	await userState.registerPushTarget({
		device_id: deviceId,
		platform,
		token,
		device_name:
			typeof navigator !== "undefined" ? navigator.userAgent : undefined,
		channel_id: pushConfig?.channel_id ?? undefined,
		metadata: {
			source,
			platform,
			provider: pushConfig?.provider,
		},
	});

	const status = await userState.setPushTargetEnabled(deviceId, true);
	setRemotePushPreference(true);
	return status;
}
