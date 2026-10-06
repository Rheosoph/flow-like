import { getApiOrigin } from "../../../../../lib/api-url";
import {
	type DeviceAccountScope,
	type LocalDeviceVault,
	unsignedManifest,
} from "../../../../../lib/device-management/storage";
import type { Ed25519PublicKey } from "../../../../../lib/device-management/types";
import type { IProfile } from "../../../../../types";

/** A run wants a model on a locked device (the desktop connector's `UnlockPrompt`). */
export interface ModelUnlockPrompt {
	id: string;
	deviceId: string;
	deviceName?: string | null;
	modelName: string;
	runId?: string | null;
	runName?: string | null;
	/** Unix milliseconds: without an answer by then, the run goes on without the device. */
	expiresAt: number;
}

export const MODEL_UNLOCK_REQUESTED = "device-model-unlock-requested";
export const MODEL_UNLOCK_CLOSED = "device-model-unlock-closed";

/**
 * What crosses to the desktop connector: the password and the encrypted
 * controller vault with its public fields. Opened keys never leave it.
 */
export interface DeviceUnlockRequest {
	deviceId: string;
	password: string;
	controllerVault: number[];
	manifestJws: string;
	grantId: string;
	ownerControllerKey?: Ed25519PublicKey;
	apiOrigin: string;
	account: string;
	keepUnlocked: boolean;
}

/** Why the prompt can't offer a password: no account, no keys here, or keys awaiting a fresh endpoint. */
export type ModelUnlockBlock = "signed_out" | "no_vault" | "fresh_endpoint";

/** The desktop's answer to a refused unlock. */
export type ModelUnlockFailure =
	| "wrong_password"
	| "authority_mismatch"
	| "device_unavailable"
	| "hub_unreachable"
	| "signed_out"
	| "failed";

export type VaultLookup =
	| { kind: "ready"; scope: DeviceAccountScope; vault: LocalDeviceVault }
	| { kind: "blocked"; block: ModelUnlockBlock };

export interface SignedInAccount {
	issuer: string;
	account: string;
}

export type ReadVault = (
	scope: DeviceAccountScope,
	deviceId: string,
) => Promise<LocalDeviceVault | undefined>;

/** The account scope the device area stores vaults under (`useHostAccount`). */
export function accountScope(
	signedIn: SignedInAccount,
	profile: IProfile | undefined,
): DeviceAccountScope {
	return {
		issuer: signedIn.issuer,
		account: signedIn.account,
		apiOrigin: getApiOrigin(profile),
		profileId: profile?.id ?? "default",
	};
}

export async function lookupVault(
	read: ReadVault,
	signedIn: SignedInAccount | undefined,
	profile: () => Promise<IProfile | undefined>,
	deviceId: string,
): Promise<VaultLookup> {
	if (!signedIn?.account) return { kind: "blocked", block: "signed_out" };
	const scope = accountScope(signedIn, await profile());
	const vault = await read(scope, deviceId);
	if (!vault) return { kind: "blocked", block: "no_vault" };
	if (vault.requiresFreshEndpoint)
		return { kind: "blocked", block: "fresh_endpoint" };
	return { kind: "ready", scope, vault };
}

export function deviceUnlockRequest(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
	password: string,
	keepUnlocked: boolean,
): DeviceUnlockRequest {
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

/** The name the device was set up with, until the connector learns the current one. */
export function vaultDeviceName(vault: LocalDeviceVault) {
	try {
		return unsignedManifest(vault.manifestJws).name.trim() || undefined;
	} catch {
		return undefined;
	}
}

const FAILURES = new Set<string>([
	"wrong_password",
	"authority_mismatch",
	"device_unavailable",
	"hub_unreachable",
	"signed_out",
]);

/** The desktop refuses an unlock with `<code>: <message>`. */
export function unlockFailure(error: unknown): ModelUnlockFailure {
	const text = error instanceof Error ? error.message : String(error);
	const code = text.split(":", 1)[0]?.trim() ?? "";
	return FAILURES.has(code) ? (code as ModelUnlockFailure) : "failed";
}

/** Adds prompts not queued yet, oldest first. */
export function withPrompts(
	queue: readonly ModelUnlockPrompt[],
	prompts: readonly ModelUnlockPrompt[],
): ModelUnlockPrompt[] {
	const known = new Set(queue.map((prompt) => prompt.id));
	return [...queue, ...prompts.filter((prompt) => !known.has(prompt.id))];
}

export function withoutPrompt(
	queue: readonly ModelUnlockPrompt[],
	promptId: string,
): ModelUnlockPrompt[] {
	return queue.filter((prompt) => prompt.id !== promptId);
}
