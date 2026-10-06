import { expect, test } from "bun:test";
import type { ArtifactBlob, ArtifactTransferStatus } from "./artifacts";
import { bitStoreFile } from "./model-push";
import type { ModelAssetStatus } from "./models";
import {
	type NativeBridge,
	createNativeKeys,
	desktopArtifactUpload,
	desktopModelPush,
} from "./native-client";
import { type ExportCommands, prepareDesktopProject } from "./project-export";
import type { DeviceAccountScope, LocalDeviceVault } from "./storage";
import type { TunnelArtifactUpload, TunnelUploadFile } from "./tunnel-data";

const DEVICE = "device-1";
const EXPORT = "a0000000-0000-4000-8000-000000000000";
const TRANSFER = "b0000000-0000-4000-8000-000000000000";
const JOB = "0b7e5c1e-4d7c-4bb0-9a43-6b5a7d1f2e3c";
const HASH = "b".repeat(64);
const scope: DeviceAccountScope = {
	issuer: "issuer",
	account: "user-1",
	apiOrigin: "https://api.flow-like.test",
	profileId: "profile",
};
const OWNER_KEY = { kty: "OKP", crv: "Ed25519", x: "A".repeat(43) } as const;

function vault(owner = true): LocalDeviceVault {
	return {
		deviceId: DEVICE,
		grantId: owner ? "owner" : "reader-grant",
		manifestJws: "header.payload.signature",
		ownerControllerKey: owner ? undefined : OWNER_KEY,
		controllerPublic: {} as LocalDeviceVault["controllerPublic"],
		controllerVault: new Uint8Array([1, 2, 3]),
	};
}

interface Call {
	command: string;
	args?: Record<string, unknown>;
}

class FakeBridge implements NativeBridge {
	calls: Call[] = [];
	answers = new Map<string, (args?: Record<string, unknown>) => unknown>();
	channels: ((message: number) => void)[] = [];
	listeners = new Map<string, Set<(payload: unknown) => void>>();
	async invoke<T>(command: string, args?: Record<string, unknown>) {
		this.calls.push({ command, args });
		return (await this.answers.get(command)?.(args)) as T;
	}
	async channel<T>(onMessage: (message: T) => void) {
		this.channels.push(onMessage as unknown as (message: number) => void);
		return { channel: this.channels.length };
	}
	async listen<T>(event: string, onEvent: (payload: T) => void) {
		const listeners = this.listeners.get(event) ?? new Set();
		const listener = onEvent as (payload: unknown) => void;
		listeners.add(listener);
		this.listeners.set(event, listeners);
		return () => {
			listeners.delete(listener);
		};
	}
	emit(event: string, payload: unknown) {
		for (const listener of this.listeners.get(event) ?? []) listener(payload);
	}
	commands() {
		return this.calls.map((call) => call.command);
	}
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const ready = { ready: async () => undefined };
const refused = {
	ready: async () => {
		throw new Error("The desktop app could not open the keys.");
	},
};

test("off the desktop the key sessions tell the desktop app nothing", async () => {
	const bridge = new FakeBridge();
	const keys = createNativeKeys(bridge, () => false);
	keys.unlocked(scope, vault(), "password", false);
	keys.kept(DEVICE, true);
	keys.locked(DEVICE, "lock");
	keys.lockAll();
	keys.watch({ held: () => undefined, lockedAll: () => undefined })();
	await keys.ready(DEVICE);
	expect(bridge.calls).toHaveLength(0);
	expect(bridge.listeners.size).toBe(0);
});

test("the desktop app gets the password and vault, and later changes wait for it", async () => {
	const bridge = new FakeBridge();
	let finishUnlock!: () => void;
	bridge.answers.set(
		"device_keys_unlock",
		() =>
			new Promise<void>((resolve) => {
				finishUnlock = resolve;
			}),
	);
	const keys = createNativeKeys(bridge, () => true);

	keys.unlocked(scope, vault(false), "password", false);
	keys.kept(DEVICE, true);
	keys.locked(DEVICE, "release");
	await Promise.resolve();
	expect(bridge.commands()).toEqual(["device_keys_unlock"]);
	expect(bridge.calls[0]?.args).toEqual({
		unlock: {
			deviceId: DEVICE,
			password: "password",
			controllerVault: [1, 2, 3],
			manifestJws: "header.payload.signature",
			grantId: "reader-grant",
			ownerControllerKey: OWNER_KEY,
			apiOrigin: scope.apiOrigin,
			account: "user-1",
			keepUnlocked: false,
		},
	});

	finishUnlock();
	await keys.ready(DEVICE);
	expect(bridge.calls.slice(1)).toEqual([
		{ command: "device_keys_keep", args: { deviceId: DEVICE, keep: true } },
		{
			command: "device_keys_lock",
			args: { deviceId: DEVICE, mode: "release" },
		},
	]);
});

test("Lock all waits for every queued change, and later changes wait for it", async () => {
	const bridge = new FakeBridge();
	let finishUnlock!: () => void;
	bridge.answers.set(
		"device_keys_unlock",
		() =>
			new Promise<void>((resolve) => {
				finishUnlock = resolve;
			}),
	);
	const keys = createNativeKeys(bridge, () => true);
	keys.unlocked(scope, vault(), "password", true);
	keys.lockAll();
	keys.kept("device-2", true);
	await flush();
	expect(bridge.commands()).toEqual(["device_keys_unlock"]);

	finishUnlock();
	await keys.ready(DEVICE);
	await keys.ready("device-2");
	expect(bridge.commands()).toEqual([
		"device_keys_unlock",
		"device_keys_lock_all",
		"device_keys_keep",
	]);
});

test("a watch hears every change of the held keys after reading them, and the tray's Lock all", async () => {
	const bridge = new FakeBridge();
	const first = {
		deviceId: DEVICE,
		apiOrigin: scope.apiOrigin,
		account: scope.account,
		kept: true,
		area: false,
	};
	bridge.answers.set("device_models_held", () => [first]);
	const keys = createNativeKeys(bridge, () => true);
	const seen: string[] = [];
	const stop = keys.watch({
		held: (devices) =>
			seen.push(devices.map((device) => device.deviceId).join(",")),
		lockedAll: () => seen.push("locked all"),
	});
	await flush();
	expect(bridge.commands()).toEqual(["device_models_held"]);
	bridge.emit("device-models-held", [first, { ...first, deviceId: "d2" }]);
	bridge.emit("device-keys-locked-all", null);
	stop();
	bridge.emit("device-models-held", []);
	expect(seen).toEqual([DEVICE, `${DEVICE},d2`, "locked all"]);
});

test("a failed unlock in the desktop app is reported until the device locks", async () => {
	const bridge = new FakeBridge();
	bridge.answers.set("device_keys_unlock", () => {
		throw new Error("hub_unreachable: GET identity failed");
	});
	const keys = createNativeKeys(bridge, () => true);
	keys.unlocked(scope, vault(), "password", false);
	await expect(keys.ready(DEVICE)).rejects.toThrow(
		`The desktop app could not open the keys of device ${DEVICE}: hub_unreachable: GET identity failed`,
	);
	expect(bridge.calls[0]?.args?.unlock).not.toHaveProperty(
		"ownerControllerKey",
	);

	keys.locked(DEVICE, "lock");
	await keys.ready(DEVICE);
	await keys.ready("device-2");
});

async function snapshotFiles() {
	const files = new Map([
		["apps/project/manifest.app", new TextEncoder().encode("manifest")],
		["apps/project/upload/data", new Uint8Array(4096).fill(7)],
	]);
	const commands: ExportCommands = {
		prepare: async (project) => ({
			export_id: EXPORT,
			project_id: project,
			files: [...files].map(([path, bytes]) => ({ path, size: bytes.length })),
			assets: { bit_pins: [], package_pins: [] },
		}),
		read: async (_, path, offset, length) =>
			(files.get(path) ?? new Uint8Array()).slice(offset, offset + length)
				.buffer,
		release: async () => undefined,
	};
	const prepared = await prepareDesktopProject("project", commands);
	const fileOf = (suffix: string) => {
		const file = prepared.artifact.files.find((entry) =>
			entry.path.endsWith(suffix),
		)?.file;
		if (!file) throw new Error(`Missing snapshot file ${suffix}`);
		return file;
	};
	return { data: fileOf("/data"), manifest: fileOf("/manifest.app") };
}

const STATUS = { transfer_id: TRANSFER } as ArtifactTransferStatus;

function uploadOf(
	file: ArtifactBlob,
	fileIndex: number | null = 1,
): TunnelArtifactUpload {
	return {
		projectId: "project",
		transferId: TRANSFER,
		fileIndex,
		offset: 12,
		file,
	};
}

test("snapshot files upload from the desktop app; other files through the live session", async () => {
	const { data } = await snapshotFiles();
	const bridge = new FakeBridge();
	bridge.answers.set("device_upload_artifact", () => STATUS);
	const live: TunnelArtifactUpload[] = [];
	const upload = desktopArtifactUpload(
		DEVICE,
		async (input) => {
			live.push(input);
			return STATUS;
		},
		bridge,
		ready,
	);

	expect(await upload(uploadOf(data))).toBe(STATUS);
	const call = bridge.calls[0];
	expect(call?.command).toBe("device_upload_artifact");
	expect(call?.args?.upload).toEqual({
		deviceId: DEVICE,
		exportId: EXPORT,
		path: "apps/project/upload/data",
		projectId: "project",
		transferId: TRANSFER,
		fileIndex: 1,
		offset: 12,
	});
	expect(typeof call?.args?.transfer).toBe("string");

	await upload(uploadOf(new Blob([new Uint8Array(20)]), 0));
	await upload(uploadOf(data.slice(0, 100)));
	await upload(uploadOf(data, null));
	expect(live).toHaveLength(3);
	expect(bridge.calls).toHaveLength(1);
});

test("without the desktop's keys or off the desktop, uploads use the live session", async () => {
	const { manifest } = await snapshotFiles();
	let live = 0;
	const fallback = async () => {
		live++;
		return STATUS;
	};
	const bridge = new FakeBridge();
	await desktopArtifactUpload(
		DEVICE,
		fallback,
		bridge,
		refused,
	)(uploadOf(manifest));
	await desktopArtifactUpload(
		DEVICE,
		fallback,
		undefined,
		ready,
	)(uploadOf(manifest));
	expect(live).toBe(2);
	expect(bridge.calls).toHaveLength(0);
});

test("aborting a native upload cancels its transfer in the desktop app", async () => {
	const { data } = await snapshotFiles();
	const bridge = new FakeBridge();
	let fail!: (error: Error) => void;
	bridge.answers.set(
		"device_upload_artifact",
		() =>
			new Promise((_, reject) => {
				fail = reject;
			}),
	);
	const abort = new AbortController();
	const upload = desktopArtifactUpload(
		DEVICE,
		async () => STATUS,
		bridge,
		ready,
	)({ ...uploadOf(data), signal: abort.signal });
	await new Promise((resolve) => setTimeout(resolve, 0));
	const reason = new Error("Deploy cancelled.");
	abort.abort(reason);
	expect(bridge.commands()).toEqual([
		"device_upload_artifact",
		"device_transfer_cancel",
	]);
	expect(bridge.calls[1]?.args).toEqual({
		transfer: bridge.calls[0]?.args?.transfer,
	});
	fail(new Error("the transfer was cancelled"));
	await expect(upload).rejects.toBe(reason);
});

const read = async () => new ArrayBuffer(0);
/** A file this window downloads: no place in a Bit store. */
const DOWNLOAD: TunnelUploadFile = {
	size: 10,
	slice() {
		return { arrayBuffer: read };
	},
};
const status = (jobId: string, state: string, bytes?: number) =>
	({
		digest: { algorithm: "sha256", hex: "a".repeat(64) },
		job_id: jobId,
		state,
		...(bytes === undefined ? {} : { bytes }),
	}) as unknown as ModelAssetStatus;

test("Bit store files and probes push from the desktop app with progress", async () => {
	const bridge = new FakeBridge();
	bridge.answers.set("device_push_model_asset", (args) => {
		bridge.channels.at(-1)?.(512);
		const push = args?.push as { offset: number };
		return push.offset === 0
			? status(JOB, "awaiting_push", 256)
			: status(JOB, "present");
	});
	const push = desktopModelPush(DEVICE, undefined, bridge, ready);

	const probe = await push({ jobId: JOB, offset: 0 });
	expect(probe.state).toBe("awaiting_push");
	expect(bridge.calls[0]?.args?.push).toEqual({
		deviceId: DEVICE,
		jobId: JOB,
		offset: 0,
	});

	const seen: number[] = [];
	const sent = await push({
		jobId: JOB,
		offset: 256,
		file: bitStoreFile(read, HASH, "model.gguf", 1024),
		onProgress: (bytes) => seen.push(bytes),
	});
	expect(sent.state).toBe("present");
	expect(bridge.calls[1]?.args?.push).toEqual({
		deviceId: DEVICE,
		jobId: JOB,
		offset: 256,
		file: { hash: HASH, fileName: "model.gguf", size: 1024 },
	});
	expect(bridge.calls[1]?.args?.progress).toEqual({ channel: 2 });
	expect(seen).toEqual([512]);
});

test("a downloaded file, or a push without the desktop's keys, goes through the live push", async () => {
	const bridge = new FakeBridge();
	let live = 0;
	const viaLive = async () => {
		live++;
		return status(JOB, "present");
	};
	const downloaded = { jobId: JOB, offset: 0, file: DOWNLOAD };
	await desktopModelPush(DEVICE, viaLive, bridge, ready)(downloaded);
	await desktopModelPush(
		DEVICE,
		viaLive,
		bridge,
		refused,
	)({ jobId: JOB, offset: 0 });
	expect(live).toBe(2);
	expect(bridge.calls).toHaveLength(0);
	const nativeOnly = desktopModelPush(DEVICE, undefined, bridge, ready);
	await expect(nativeOnly(downloaded)).rejects.toThrow("own Bit store");
	expect(nativeOnly.refuses?.(DOWNLOAD)).toContain("own Bit store");
	const stored = bitStoreFile(read, HASH, "model.gguf", 1024);
	expect(nativeOnly.refuses?.(stored)).toBeUndefined();
	expect(
		desktopModelPush(DEVICE, viaLive, bridge, ready).refuses?.(DOWNLOAD),
	).toBeUndefined();
});

test("the desktop's answer for another job is refused", async () => {
	const bridge = new FakeBridge();
	bridge.answers.set("device_push_model_asset", () =>
		status("other", "present"),
	);
	const push = desktopModelPush(DEVICE, undefined, bridge, ready);
	await expect(push({ jobId: JOB, offset: 0 })).rejects.toThrow(
		"does not match",
	);
});
