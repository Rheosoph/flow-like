import { withPassword } from "../../../../lib/device-management/crypto";
import { changeDevicePassword } from "../../../../lib/device-management/password";
import {
	type ControllerBackupFile,
	openControllerBackup,
	readAccountBackupStatus,
	readControllerBackupFile,
	restoreAccountRecovery,
	retryPendingAccountBackup,
	saveAccountRecovery,
	sealedControllerBackup,
} from "../../../../lib/device-management/recovery";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	addDeviceVault,
	readDeviceVault,
	replaceEndpointVault,
} from "../../../../lib/device-management/storage";
import type { DeviceCrypto } from "../../../../lib/device-management/types";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { KeyFlowError } from "./key-errors";

/* The key flows of N9 and the device Keys tab. Every function runs inside `useDeviceAction().run({ call })`. */

export const PASSWORD_MIN_BYTES = 12;
export const PASSWORD_MAX_BYTES = 4096;
export const KEY_FILE_MAX_BYTES = 1024 * 1024;

const DEVICE_ID =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function keyFileName(deviceId: string): string {
	return `flow-like-controller-${deviceId}.json`;
}

const statusOf = (error: unknown) =>
	(error as { status?: unknown } | null)?.status;

/** Opens the stored keys once with the typed password, so nothing is sealed with a mistyped one. */
async function assertPassword(
	crypto: Pick<DeviceCrypto, "unlockControllerVault">,
	vault: LocalDeviceVault,
	password: string,
): Promise<void> {
	try {
		await withPassword(password, (bytes) => {
			const controller = crypto.unlockControllerVault(
				vault.deviceId,
				bytes,
				vault.controllerVault,
			);
			try {
				controller.close();
			} finally {
				controller.free();
			}
		});
	} catch {
		throw new KeyFlowError("wrong_password");
	}
}

function recoveryInput(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
	signal?: AbortSignal,
) {
	const { api, profile, scope } = workspace.deps;
	return { api, profile, scope, deviceId, password, signal };
}

/** Seals this computer's keys with the device password and stores them on the account; returns the version. */
export function saveAccountBackup(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
	signal?: AbortSignal,
): Promise<number> {
	return workspace.keys.withVaultLease(deviceId, async (lease) => {
		const crypto = await workspace.deps.crypto();
		await assertPassword(crypto, lease.vault, password);
		try {
			return await saveAccountRecovery({
				...recoveryInput(workspace, deviceId, password, signal),
				crypto,
				lease,
			});
		} catch (error) {
			if (statusOf(error) === 429) throw new KeyFlowError("limit");
			throw error;
		}
	});
}

/** Publishes the upload that was sealed earlier; needs no password. */
export function retryAccountBackup(
	workspace: DeviceWorkspace,
	deviceId: string,
	signal?: AbortSignal,
): Promise<number> {
	const { api, profile, scope } = workspace.deps;
	return workspace.keys.withVaultLease(deviceId, async (lease) => {
		try {
			return await retryPendingAccountBackup(
				api,
				profile,
				scope,
				deviceId,
				lease,
				signal,
			);
		} catch (error) {
			if (statusOf(error) === 429) throw new KeyFlowError("limit");
			throw error;
		}
	});
}

/**
 * "Check account backup": opens the account's copy with the device password
 * and adopts its version number. The keys and the password on this computer
 * stay as they are.
 */
export async function verifyAccountBackup(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
	signal?: AbortSignal,
): Promise<number> {
	await workspace.keys.withVaultLease(deviceId, async (lease) => {
		const crypto = await workspace.deps.crypto();
		await restoreAccountRecovery({
			...recoveryInput(workspace, deviceId, password, signal),
			crypto,
			lease,
		});
	});
	await workspace.local.reload().catch(() => undefined);
	return workspace.local.summary().backups[deviceId]?.localRevision ?? 0;
}

/** Brings the keys back from the account backup; they start locked. Returns the restored version. */
export async function restoreFromAccount(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
	signal?: AbortSignal,
): Promise<number> {
	const crypto = await workspace.deps.crypto();
	await restoreAccountRecovery({
		...recoveryInput(workspace, deviceId, password, signal),
		crypto,
	});
	await workspace.local.reload().catch(() => undefined);
	return workspace.local.summary().backups[deviceId]?.localRevision ?? 0;
}

/** The account's version for one device without opening it; 0 when there is none. */
export async function readAccountBackupRevision(
	workspace: DeviceWorkspace,
	deviceId: string,
	signal?: AbortSignal,
): Promise<number> {
	const { api, profile } = workspace.deps;
	return (
		(await readAccountBackupStatus(api, profile, deviceId, signal))?.revision ??
		0
	);
}

export interface ParsedKeyFile {
	deviceId: string;
	file: ControllerBackupFile;
}

/** Reads a key backup file far enough to know its device; sealed files still need their password. */
export function parseKeyFile(
	text: string,
	scope: DeviceAccountScope,
): ParsedKeyFile {
	if (new TextEncoder().encode(text).length > KEY_FILE_MAX_BYTES)
		throw new KeyFlowError("too_large");
	let value: unknown;
	try {
		value = JSON.parse(text);
	} catch {
		throw new KeyFlowError("not_a_backup");
	}
	const deviceId = (value as { deviceId?: unknown } | null)?.deviceId;
	if (typeof deviceId !== "string" || !DEVICE_ID.test(deviceId))
		throw new KeyFlowError("not_a_backup");
	return { deviceId, file: readControllerBackupFile(text, scope, deviceId) };
}

export type ImportOutcome = "imported" | "same";

/** Checks the file, then keeps its keys on this computer, locked. */
export async function importKeyFile(
	workspace: DeviceWorkspace,
	parsed: ParsedKeyFile,
	password: string,
): Promise<ImportOutcome> {
	const { scope } = workspace.deps;
	const { deviceId, file } = parsed;
	const vault = file.sealed
		? await openControllerBackup(
				scope,
				deviceId,
				file,
				password,
				await workspace.deps.crypto(),
			)
		: file.vault;
	const existing = await readDeviceVault(scope, deviceId);
	if (existing) {
		if (
			existing.controllerPublic.controller_key.x ===
			vault.controllerPublic.controller_key.x
		)
			return "same";
		throw new KeyFlowError("exists");
	}
	await addDeviceVault(scope, vault);
	await workspace.local.reload().catch(() => undefined);
	return "imported";
}

/** A sealed backup file of the keys on this computer, opened only by the device password. */
export function sealKeyFile(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
): Promise<Blob> {
	return workspace.keys.withVaultLease(deviceId, async (lease) => {
		const crypto = await workspace.deps.crypto();
		await assertPassword(crypto, lease.vault, password);
		return sealedControllerBackup(
			workspace.deps.scope,
			lease.vault,
			password,
			crypto,
		);
	});
}

export interface PasswordChange {
	/** The backup file sealed with the new password; absent when it could not be prepared. */
	file?: Blob;
}

/**
 * Re-encrypts this computer's copy of the keys; works while the device is
 * unlocked. The new backup file is sealed in the same step, so the password
 * is not needed again for it.
 */
export function changePassword(
	workspace: DeviceWorkspace,
	deviceId: string,
	current: string,
	next: string,
	signal?: AbortSignal,
): Promise<PasswordChange> {
	const { scope } = workspace.deps;
	return workspace.keys.withVaultLease(deviceId, async (lease) => {
		const crypto = await workspace.deps.crypto();
		await assertPassword(crypto, lease.vault, current);
		const vault = await changeDevicePassword(
			scope,
			lease.vault,
			current,
			next,
			crypto,
			signal,
			lease,
		);
		const file = await sealedControllerBackup(scope, vault, next, crypto).catch(
			() => undefined,
		);
		return file ? { file } : {};
	});
}

/** Locks the device here first, then removes its keys, trust pins and backup watermark. */
export async function deleteLocalKeys(
	workspace: DeviceWorkspace,
	deviceId: string,
): Promise<void> {
	workspace.keys.lock(deviceId);
	await workspace.local.deleteKeys(deviceId);
}

/**
 * IA S23: a fresh identity for shared live metrics. The approved keys stay;
 * every group this computer read needs the owner's approval again, and the
 * device locks here so the next unlock connects with the new identity.
 */
export async function resetMetricIdentity(
	workspace: DeviceWorkspace,
	deviceId: string,
	password: string,
): Promise<void> {
	await workspace.keys.withVaultLease(deviceId, async ({ vault }) => {
		const crypto = await workspace.deps.crypto();
		await withPassword(password, async (bytes) => {
			let controller: ReturnType<DeviceCrypto["unlockControllerVault"]>;
			try {
				controller = crypto.unlockControllerVault(
					deviceId,
					bytes,
					vault.controllerVault,
				);
			} catch {
				throw new KeyFlowError("wrong_password");
			}
			try {
				const fresh = controller.freshEndpointVault(bytes);
				await replaceEndpointVault(workspace.deps.scope, vault, {
					...vault,
					controllerPublic: fresh.public_bundle,
					controllerVault: Uint8Array.from(fresh.vault),
					requiresFreshEndpoint: false,
				});
			} finally {
				try {
					controller.close();
				} finally {
					controller.free();
				}
			}
		});
	});
	workspace.keys.lock(deviceId);
	await workspace.local.reload().catch(() => undefined);
}
