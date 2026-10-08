import { isTauri } from "../platform";
import type { ArtifactTransferStatus } from "./artifacts";
import type { ModelPush, PushFile } from "./model-push";
import { nativeArtifactUploadError } from "./native-errors";
import { snapshotSource } from "./project-export";
import type { DeviceAccountScope, LocalDeviceVault } from "./storage";
import {
	type TunnelArtifactUpload,
	type TunnelModelAssetPush,
	modelAssetReply,
} from "./tunnel-data";

/*
 * The desktop's native device client (plan §11 E3): local ports, deploy
 * uploads and model pushes run in the desktop app over its own tunnel, so no
 * byte crosses the webview. The keys the device area unlocks are opened there
 * too. Locking a device locks it there as well; when the area lets go on its
 * own (idle lock, page hide), keys kept for model access or used by a run
 * stay. The web keeps the TypeScript tunnel.
 */

/** The desktop app's commands; tests replace it. */
export interface NativeBridge {
	invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
	/** A channel the desktop app streams `T`s through. */
	channel<T>(onMessage: (message: T) => void): Promise<unknown>;
	/** Resolves to the function that stops listening to the app event. */
	listen?<T>(event: string, onEvent: (payload: T) => void): Promise<() => void>;
}

export const tauriBridge: NativeBridge = {
	async invoke<T>(command: string, args?: Record<string, unknown>) {
		const { invoke } = await import("@tauri-apps/api/core");
		return invoke<T>(command, args);
	},
	async channel<T>(onMessage: (message: T) => void) {
		const { Channel } = await import("@tauri-apps/api/core");
		const channel = new Channel<T>();
		channel.onmessage = onMessage;
		return channel;
	},
	async listen<T>(event: string, onEvent: (payload: T) => void) {
		const { listen } = await import("@tauri-apps/api/event");
		return listen<T>(event, (message) => onEvent(message.payload));
	},
};

const messageOf = (error: unknown) =>
	error instanceof Error ? error.message : String(error);

/** The password and the encrypted vault with its public fields; the opened keys never cross. */
export function nativeUnlockRequest(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
	password: string,
	keepUnlocked: boolean,
) {
	return {
		deviceId: vault.deviceId,
		password,
		controllerVault: Array.from(vault.controllerVault),
		manifestJws: vault.manifestJws,
		grantId: vault.grantId,
		...(vault.ownerControllerKey
			? { ownerControllerKey: vault.ownerControllerKey }
			: {}),
		apiOrigin: scope.apiOrigin,
		account: scope.account,
		keepUnlocked,
	};
}

/**
 * How the device area lets go of a device's keys in the desktop app.
 * `lock`: the user locked it, so the desktop drops its keys too, kept or not.
 * `release`: the area let go on its own (idle lock, page hide); keys kept for
 * model access or used by a run stay until they idle.
 */
export type NativeLockMode = "lock" | "release";

/** A device whose keys the desktop app holds (`device_models_held`). */
export interface NativeHeldKeys {
	deviceId: string;
	deviceName?: string | null;
	apiOrigin: string;
	account: string;
	/** Kept for model access until the user locks it, signs out or quits. */
	kept: boolean;
	/** The device area holds them too. */
	area: boolean;
}

export const NATIVE_HELD_EVENT = "device-models-held";
export const NATIVE_LOCKED_ALL_EVENT = "device-keys-locked-all";

export interface NativeKeyWatcher {
	/** Every device whose keys the desktop holds, now and after each change. */
	held(devices: readonly NativeHeldKeys[]): void;
	/** The tray's "Lock All Devices" locked every device. */
	lockedAll(): void;
}

/** What the key sessions of the device area tell the desktop app. */
export interface NativeKeyMirror {
	/** Never throws; `ready` reports a failed unlock. */
	unlocked(
		scope: DeviceAccountScope,
		vault: LocalDeviceVault,
		password: string,
		keepUnlocked: boolean,
	): void;
	locked(deviceId: string, mode: NativeLockMode): void;
	kept(deviceId: string, keepUnlocked: boolean): void;
	/** Locks every device the desktop holds keys for, also those a run's prompt unlocked. */
	lockAll(): void;
	/** Returns the stop; a no-op off the desktop. */
	watch(watcher: NativeKeyWatcher): () => void;
}

export interface NativeKeys extends NativeKeyMirror {
	/** Settles once the desktop applied every key change of the device; rejects when its last unlock failed. */
	ready(deviceId: string): Promise<void>;
}

/** Listens first, then reads the current list, so no change is missed. */
function watchHeld(bridge: NativeBridge, watcher: NativeKeyWatcher) {
	if (!bridge.listen) return () => undefined;
	let stopped = false;
	let fresh = false;
	const stops: (() => void)[] = [];
	const keep = (stop: () => void) => {
		if (stopped) stop();
		else stops.push(stop);
	};
	Promise.all([
		bridge
			.listen<NativeHeldKeys[]>(NATIVE_HELD_EVENT, (held) => {
				fresh = true;
				if (!stopped) watcher.held(held);
			})
			.then(keep),
		bridge
			.listen<unknown>(NATIVE_LOCKED_ALL_EVENT, () => {
				if (!stopped) watcher.lockedAll();
			})
			.then(keep),
	])
		.then(() => bridge.invoke<NativeHeldKeys[]>("device_models_held"))
		.then((held) => {
			if (!stopped && !fresh) watcher.held(held);
		})
		.catch(() => undefined);
	return () => {
		stopped = true;
		for (const stop of stops.splice(0)) stop();
	};
}

export function createNativeKeys(
	bridge: NativeBridge,
	enabled: () => boolean,
): NativeKeys {
	const queues = new Map<string, Promise<void>>();
	const failures = new Map<string, unknown>();
	/** The last Lock all; changes of devices it has not seen wait for it. */
	let barrier: Promise<void> = Promise.resolve();
	const then = (deviceId: string, step: () => Promise<unknown>) => {
		const next = (queues.get(deviceId) ?? barrier).then(step);
		queues.set(
			deviceId,
			next.then(
				() => undefined,
				() => undefined,
			),
		);
	};
	return {
		unlocked(scope, vault, password, keepUnlocked) {
			if (!enabled()) return;
			const unlock = nativeUnlockRequest(scope, vault, password, keepUnlocked);
			then(vault.deviceId, async () => {
				failures.delete(vault.deviceId);
				await bridge
					.invoke("device_keys_unlock", { unlock })
					.catch((error: unknown) => failures.set(vault.deviceId, error));
			});
		},
		locked(deviceId, mode) {
			if (!enabled()) return;
			then(deviceId, async () => {
				failures.delete(deviceId);
				await bridge.invoke("device_keys_lock", { deviceId, mode });
			});
		},
		kept(deviceId, keepUnlocked) {
			if (!enabled()) return;
			then(deviceId, () =>
				bridge.invoke("device_keys_keep", { deviceId, keep: keepUnlocked }),
			);
		},
		lockAll() {
			if (!enabled()) return;
			const all = Promise.all([barrier, ...queues.values()])
				.then(() => bridge.invoke("device_keys_lock_all"))
				.then(
					() => failures.clear(),
					() => failures.clear(),
				);
			barrier = all;
			for (const deviceId of queues.keys()) queues.set(deviceId, all);
		},
		watch(watcher) {
			return enabled() ? watchHeld(bridge, watcher) : () => undefined;
		},
		async ready(deviceId) {
			await (queues.get(deviceId) ?? barrier);
			if (failures.has(deviceId))
				throw new Error(
					`The desktop app could not open the keys of device ${deviceId}: ${messageOf(failures.get(deviceId))}`,
				);
		},
	};
}

export const nativeKeys = createNativeKeys(tauriBridge, isTauri);

export const nativeKeysReady = (deviceId: string) => nativeKeys.ready(deviceId);

/** Runs one native transfer; aborting `signal` cancels it in the desktop app. */
async function transfer<T>(
	bridge: NativeBridge,
	signal: AbortSignal | undefined,
	run: (transfer: string) => Promise<T>,
): Promise<T> {
	signal?.throwIfAborted();
	const id = crypto.randomUUID();
	const cancel = () =>
		void bridge
			.invoke("device_transfer_cancel", { transfer: id })
			.catch(() => undefined);
	signal?.addEventListener("abort", cancel, { once: true });
	try {
		return await run(id);
	} catch (error) {
		throw signal?.aborted ? (signal.reason ?? error) : error;
	} finally {
		signal?.removeEventListener("abort", cancel);
	}
}

/** The desktop holds the device's keys, or `fallback` carries the transfer when it could not open them. */
async function nativeOr<T>(
	keys: Pick<NativeKeys, "ready">,
	deviceId: string,
	fallback: (() => Promise<T>) | undefined,
	native: () => Promise<T>,
): Promise<T> {
	try {
		await keys.ready(deviceId);
	} catch (error) {
		if (fallback) return fallback();
		throw error;
	}
	return native();
}

export type ArtifactUpload = (
	input: TunnelArtifactUpload,
) => Promise<ArtifactTransferStatus>;

/**
 * Files of the desktop's prepared export go from its snapshot to the device
 * in the desktop app; files made in this window (the manifest, approved
 * metadata) and every file on the web go through `upload`.
 */
export function desktopArtifactUpload(
	deviceId: string,
	upload: ArtifactUpload,
	bridge: NativeBridge | undefined = isTauri() ? tauriBridge : undefined,
	keys: Pick<NativeKeys, "ready"> = nativeKeys,
): ArtifactUpload {
	return (input) => {
		const source = snapshotSource(input.file);
		if (!bridge || !source || input.fileIndex === null) return upload(input);
		const fileIndex = input.fileIndex;
		return nativeOr(
			keys,
			deviceId,
			() => upload(input),
			() =>
				transfer(bridge, input.signal, (id) =>
					bridge
						.invoke<ArtifactTransferStatus>("device_upload_artifact", {
							transfer: id,
							upload: {
								deviceId,
								exportId: source.exportId,
								path: source.path,
								projectId: input.projectId,
								transferId: input.transferId,
								fileIndex,
								offset: input.offset,
							},
						})
						.catch((error: unknown) => {
							throw nativeArtifactUploadError(error);
						}),
				),
		);
	};
}

const bitStoreSource = (file: TunnelModelAssetPush["file"]) =>
	(file as Partial<PushFile> | undefined)?.local;

const BIT_STORE_ONLY =
	"The desktop app sends model files from its own Bit store only; download the model on this computer first.";

/**
 * Pushes on the desktop: the probe and files of this computer's Bit store go
 * from the desktop app; a downloaded file goes through the live session's
 * push, when it offers one.
 */
export function desktopModelPush(
	deviceId: string,
	live: ModelPush | undefined,
	bridge: NativeBridge = tauriBridge,
	keys: Pick<NativeKeys, "ready"> = nativeKeys,
): ModelPush {
	const push: ModelPush = async (input) => {
		const local = bitStoreSource(input.file);
		if (input.file && !local) {
			if (live) return live(input);
			throw new Error(BIT_STORE_ONLY);
		}
		return nativeOr(keys, deviceId, live && (() => live(input)), async () => {
			const progress = await bridge.channel<number>((bytes) =>
				input.onProgress?.(bytes),
			);
			const value = await transfer(bridge, input.signal, (id) =>
				bridge.invoke<Record<string, unknown>>("device_push_model_asset", {
					transfer: id,
					progress,
					push: {
						deviceId,
						jobId: input.jobId,
						offset: input.offset,
						...(local && input.file
							? {
									file: {
										hash: local.hash,
										fileName: local.fileName,
										size: input.file.size,
									},
								}
							: {}),
					},
				}),
			);
			return modelAssetReply(value, input.jobId);
		});
	};
	push.refuses = (file) =>
		bitStoreSource(file) !== undefined || live !== undefined
			? undefined
			: BIT_STORE_ONLY;
	return push;
}
