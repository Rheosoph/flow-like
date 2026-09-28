import { z } from "zod";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	base64url,
	loadDeviceCrypto,
	unbase64url,
	withPassword,
} from "./crypto";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	acquireDeviceLock,
	completeAccountRecovery,
	controllerBackup,
	encryptedControllerBackup,
	isPublicKey,
	readAccountRecoveryState,
	readDeviceVault,
	restoreAccountRecoveryVault,
	stageAccountRecovery,
} from "./storage";
import type {
	AccountRecoveryContext,
	AccountRecoveryWrite,
	DeviceCrypto,
	Ed25519PublicKey,
} from "./types";

const reply = z
	.object({
		public_key: z
			.object({
				kty: z.literal("OKP"),
				crv: z.literal("Ed25519"),
				x: z.string().length(43),
			})
			.strict(),
		ciphertext: z.string().min(86).max(87_382),
		revision: z.number().int().min(1).max(Number.MAX_SAFE_INTEGER),
	})
	.strict();

export interface RecoveryInput {
	api: IApiState;
	profile: IProfile;
	scope: DeviceAccountScope;
	deviceId: string;
	password: string;
	signal?: AbortSignal;
	crypto?: Pick<DeviceCrypto, "sealAccountRecovery" | "openAccountRecovery">;
}

/** Account revisions count up from one, so local files never collide with a hub copy. */
const LOCAL_BACKUP_REVISION = Number.MAX_SAFE_INTEGER;
const MAX_BACKUP_BYTES = 1024 * 1024;

function context(
	scope: DeviceAccountScope,
	deviceId: string,
	key: Ed25519PublicKey,
	revision: number,
): AccountRecoveryContext {
	return {
		issuer: scope.issuer,
		account: scope.account,
		api_origin: scope.apiOrigin,
		device_id: deviceId,
		controller_key: key,
		revision,
	};
}
function path(deviceId: string) {
	return `devices/controller-vaults/${encodeURIComponent(deviceId)}`;
}
async function digest(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
	return base64url(
		new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
	);
}

async function publish(input: RecoveryInput, request: AccountRecoveryWrite) {
	const result = await input.api.fetch<{ revision: number }>(
		input.profile,
		path(input.deviceId),
		{
			method: "PUT",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(request),
			signal: input.signal,
		},
	);
	if (result.revision !== request.revision)
		throw new Error(
			"The account backup acknowledgement does not match its revision.",
		);
	await completeAccountRecovery(input.scope, input.deviceId, request);
}

export async function saveAccountRecovery(
	input: RecoveryInput,
): Promise<number> {
	input.signal?.throwIfAborted();
	const release = await acquireDeviceLock(input.scope, input.deviceId);
	try {
		const record = await readDeviceVault(input.scope, input.deviceId);
		if (!record)
			throw new Error(
				"Restore this device's local keys before creating an account backup.",
			);
		let state = await readAccountRecoveryState(input.scope, input.deviceId);
		// Finish an uncertain upload with the exact signed ciphertext, even if a
		// local password change has since prepared a newer envelope.
		if (state.pending) {
			await publish(input, state.pending.request);
			state = await readAccountRecoveryState(input.scope, input.deviceId);
		}
		const bytes = new Uint8Array(
			await encryptedControllerBackup(input.scope, record).arrayBuffer(),
		);
		const sourceDigest = await digest(bytes);
		const module = input.crypto ?? (await loadDeviceCrypto());
		const revision = state.revision + 1;
		if (!Number.isSafeInteger(revision))
			throw new Error("The account backup revision limit was reached.");
		const encrypted = await withPassword(input.password, (password) =>
			module.sealAccountRecovery(
				context(
					input.scope,
					input.deviceId,
					record.controllerPublic.controller_key,
					revision,
				),
				password,
				bytes,
			),
		);
		if (state.sourceDigest === sourceDigest) return state.revision;
		const request: AccountRecoveryWrite = {
			public_key: record.controllerPublic.controller_key,
			ciphertext: base64url(Uint8Array.from(encrypted.ciphertext)),
			revision,
			proof_jws: encrypted.proof_jws,
		};
		input.signal?.throwIfAborted();
		await stageAccountRecovery(
			input.scope,
			input.deviceId,
			sourceDigest,
			request,
		);
		await publish(input, request);
		return revision;
	} finally {
		release();
	}
}

/** Restore stable keys, then let management create a new MLS endpoint on unlock. */
export async function restoreAccountRecovery(
	input: RecoveryInput,
): Promise<LocalDeviceVault> {
	input.signal?.throwIfAborted();
	const release = await acquireDeviceLock(input.scope, input.deviceId);
	try {
		const response = reply.parse(
			await input.api.fetch<unknown>(input.profile, path(input.deviceId), {
				method: "GET",
				signal: input.signal,
			}),
		);
		const state = await readAccountRecoveryState(input.scope, input.deviceId);
		if (state.pending && response.revision < state.pending.request.revision)
			throw new Error(
				"The pending backup has not reached the account yet. Save the account backup again to finish its upload.",
			);
		if (state.revision > response.revision)
			throw new Error(
				"The downloaded account backup is older than this app has seen.",
			);
		unbase64url(response.public_key.x, 32);
		const module = input.crypto ?? (await loadDeviceCrypto());
		const decoded = await withPassword(input.password, (password) =>
			module.openAccountRecovery(
				context(
					input.scope,
					input.deviceId,
					response.public_key,
					response.revision,
				),
				password,
				unbase64url(response.ciphertext, 65_536),
			),
		);
		const restored = controllerBackup(
			JSON.stringify(decoded),
			input.scope,
			input.deviceId,
		);
		if (restored.controllerPublic.controller_key.x !== response.public_key.x)
			throw new Error(
				"The recovery controller does not match the account backup.",
			);
		input.signal?.throwIfAborted();
		const bytes = new Uint8Array(
			await encryptedControllerBackup(input.scope, restored).arrayBuffer(),
		);
		const sourceDigest = await digest(bytes);
		input.signal?.throwIfAborted();
		return await restoreAccountRecoveryVault(
			input.scope,
			restored,
			response.revision,
			sourceDigest,
			state.pending?.request,
		);
	} finally {
		release();
	}
}

/** The password seals every trust anchor, so a copied file cannot swap the manifest or owner key. */
export async function sealedControllerBackup(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
	password: string,
	crypto: Pick<DeviceCrypto, "sealAccountRecovery">,
): Promise<Blob> {
	const bytes = new Uint8Array(
		await encryptedControllerBackup(scope, vault).arrayBuffer(),
	);
	const key = vault.controllerPublic.controller_key;
	const sealed = await withPassword(password, (secret) =>
		crypto.sealAccountRecovery(
			context(scope, vault.deviceId, key, LOCAL_BACKUP_REVISION),
			secret,
			bytes,
		),
	);
	return new Blob(
		[
			JSON.stringify({
				version: 2,
				apiOrigin: scope.apiOrigin,
				deviceId: vault.deviceId,
				controllerKey: key,
				sealed: base64url(Uint8Array.from(sealed.ciphertext)),
			}),
		],
		{ type: "application/json" },
	);
}

export type ControllerBackupFile =
	| { sealed: false; vault: LocalDeviceVault }
	| { sealed: true; controllerKey: Ed25519PublicKey; ciphertext: Uint8Array };

export function readControllerBackupFile(
	text: string,
	scope: DeviceAccountScope,
	deviceId: string,
): ControllerBackupFile {
	if (new TextEncoder().encode(text).length > MAX_BACKUP_BYTES)
		throw new Error("Controller backups must be smaller than 1 MiB.");
	const value = JSON.parse(text) as Record<string, unknown> | null;
	if (value?.version !== 2) {
		const vault = controllerBackup(text, scope, deviceId);
		if (vault.grantId !== "owner")
			throw new Error(
				"This older shared-access backup cannot be verified. Restore it from your account backup, or request access from the device owner again.",
			);
		return { sealed: false, vault };
	}
	if (
		!value ||
		value.apiOrigin !== scope.apiOrigin ||
		value.deviceId !== deviceId ||
		!isPublicKey(value.controllerKey) ||
		typeof value.sealed !== "string"
	)
		throw new Error("This backup belongs to another device or hub.");
	return {
		sealed: true,
		controllerKey: value.controllerKey,
		ciphertext: unbase64url(value.sealed, 65_600),
	};
}

export async function openControllerBackup(
	scope: DeviceAccountScope,
	deviceId: string,
	file: Extract<ControllerBackupFile, { sealed: true }>,
	password: string,
	crypto: Pick<DeviceCrypto, "openAccountRecovery">,
): Promise<LocalDeviceVault> {
	const decoded = await withPassword(password, (secret) =>
		crypto.openAccountRecovery(
			context(scope, deviceId, file.controllerKey, LOCAL_BACKUP_REVISION),
			secret,
			file.ciphertext,
		),
	);
	const vault = controllerBackup(JSON.stringify(decoded), scope, deviceId);
	if (vault.controllerPublic.controller_key.x !== file.controllerKey.x)
		throw new Error(
			`The sealed backup for device ${deviceId} names a different controller than its envelope.`,
		);
	return vault;
}
