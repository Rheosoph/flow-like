import { isTransportFailure } from "../../../../lib/api-error";
import { toHubError } from "../../../../lib/device-management/hub/endpoints";
import type { HubErrorCode } from "../../../../lib/device-management/model/types";
import {
	DeviceLockHeldError,
	DeviceLockUnsupportedError,
	DeviceVaultExistsError,
} from "../../../../lib/device-management/storage";
import { KeySessionError } from "../../../../lib/device-management/workspace/keys";
import type { KeyError } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";

/** Why a key flow stopped. The wording is ours; lib and crypto exception texts never reach the screen. */
export type KeyFailureCode =
	| "wrong_password"
	| "wrong_backup_password"
	| "limit"
	| "conflict"
	| "pending_first"
	| "older_than_seen"
	| "different_keys"
	| "no_backup"
	| "not_allowed"
	| "no_keys"
	| "held_elsewhere"
	| "lock_unsupported"
	| "crypto_unavailable"
	| "changed_elsewhere"
	| "exists"
	| "other_device"
	| "too_large"
	| "not_a_backup"
	| "legacy_shared"
	| "same_password"
	| "storage"
	| "hub"
	| "cancelled"
	| "unknown";

export interface KeyFailure {
	code: KeyFailureCode;
	/** With `hub`: which hub failure. */
	hub?: HubErrorCode;
}

/** Thrown by the key flows themselves once they know why a step failed. */
export class KeyFlowError extends Error {
	constructor(readonly code: KeyFailureCode) {
		super(`Key flow stopped: ${code}.`);
		this.name = "KeyFlowError";
	}
}

const KEY_SESSION: Record<KeyError["code"], KeyFailureCode> = {
	wrong_password: "wrong_password",
	no_vault: "no_keys",
	held_elsewhere: "held_elsewhere",
	lock_unsupported: "lock_unsupported",
	crypto_unavailable: "crypto_unavailable",
	identity_mismatch: "different_keys",
	authority_mismatch: "different_keys",
	storage: "storage",
};

/** Every sentence `DM/{recovery,storage,password,crypto}.ts` throws on these paths. */
const LIB_MESSAGES: readonly (readonly [string, KeyFailureCode])[] = [
	["Restore this device's local keys before", "no_keys"],
	["The pending backup has not reached the account yet", "pending_first"],
	["The downloaded account backup is older", "older_than_seen"],
	[
		"An account backup is pending or the downloaded revision",
		"older_than_seen",
	],
	["The recovery controller does not match", "different_keys"],
	["This app has a different controller", "different_keys"],
	["The sealed backup for device", "different_keys"],
	["The unlocked vault does not match", "different_keys"],
	["The account backup changed in another app tab", "changed_elsewhere"],
	["The pending account backup changed", "changed_elsewhere"],
	["The restored controller vault changed", "changed_elsewhere"],
	["The replacement endpoint changed", "changed_elsewhere"],
	["Controller backups must be smaller", "too_large"],
	["This older shared-access backup", "legacy_shared"],
	["This backup belongs to another device or hub", "other_device"],
	["Invalid encrypted", "not_a_backup"],
	["Choose a different password", "same_password"],
	["Device cryptography could not be loaded", "crypto_unavailable"],
	["Device management requires a browser session", "crypto_unavailable"],
];

const STATUS: Readonly<Record<number, KeyFailureCode>> = {
	403: "not_allowed",
	404: "no_backup",
	409: "conflict",
	410: "no_backup",
};

function statusOf(error: unknown): number | undefined {
	const status = (error as { status?: unknown } | null)?.status;
	return typeof status === "number" ? status : undefined;
}

function hubFailure(error: unknown, status: number | undefined): KeyFailure {
	if (status !== undefined && STATUS[status]) return { code: STATUS[status] };
	return { code: "hub", hub: toHubError(error).code };
}

function libFailure(error: unknown): KeyFailureCode | undefined {
	if (!(error instanceof Error)) return undefined;
	return LIB_MESSAGES.find(([start]) => error.message.startsWith(start))?.[1];
}

type ErrorClass = abstract new (...args: never[]) => Error;

const CLASS_CODES: readonly (readonly [ErrorClass, KeyFailureCode])[] = [
	[DeviceVaultExistsError, "exists"],
	[DeviceLockHeldError, "held_elsewhere"],
	[DeviceLockUnsupportedError, "lock_unsupported"],
	[SyntaxError, "not_a_backup"],
];

/** Errors that say what happened by their class. */
function classFailure(error: unknown): KeyFailureCode | undefined {
	if (error instanceof KeyFlowError) return error.code;
	if (error instanceof KeySessionError) return KEY_SESSION[error.keyError.code];
	if (error instanceof DOMException && error.name === "AbortError")
		return "cancelled";
	return CLASS_CODES.find(([type]) => error instanceof type)?.[1];
}

/**
 * Files an error of a key flow. `fallback` names the step that was running
 * when nothing else explains it: opening a vault or a backup fails with a
 * crypto exception whose text is never shown.
 */
export function classifyKeyFailure(
	error: unknown,
	fallback: KeyFailureCode = "unknown",
): KeyFailure {
	const byClass = classFailure(error);
	if (byClass) return { code: byClass };
	const status = statusOf(error);
	if (status !== undefined || isTransportFailure(error))
		return hubFailure(error, status);
	return { code: libFailure(error) ?? fallback };
}

type Copy = (t: DevicesT, device: string) => string;

const COPY: Record<Exclude<KeyFailureCode, "hub" | "cancelled">, Copy> = {
	wrong_password: (t, device) =>
		t(
			"devices:keys.error.wrongPassword",
			"That password doesn't open the keys for {{device}} on this computer.",
			{ device },
		),
	wrong_backup_password: (t) =>
		t(
			"devices:keys.error.wrongBackupPassword",
			"That password doesn't open this backup, or the backup was changed. Nothing was restored.",
		),
	limit: (t) =>
		t(
			"devices:keys.error.limit",
			"Your account has no free backup slot. The keys stay only on this computer for now.",
		),
	conflict: (t) =>
		t(
			"devices:keys.error.conflict",
			"Another computer saved a newer backup first. Check the account backup to adopt its version, then save again.",
		),
	pending_first: (t) =>
		t(
			"devices:keys.error.pendingFirst",
			"The latest backup from this computer hasn't reached your account yet. Retry the upload first.",
		),
	older_than_seen: (t) =>
		t(
			"devices:keys.error.olderThanSeen",
			"Your account returned an older backup than this computer has seen. Nothing was changed; try again in a moment.",
		),
	different_keys: (t, device) =>
		t(
			"devices:keys.error.differentKeys",
			"That backup holds other keys for {{device}} than this computer. Nothing was changed.",
			{ device },
		),
	no_backup: (t, device) =>
		t(
			"devices:keys.error.noBackup",
			"Your account has no backup of the keys for {{device}}.",
			{ device },
		),
	not_allowed: (t, device) =>
		t(
			"devices:keys.error.notAllowed",
			"The hub refused this: your access to {{device}} doesn't cover it any more.",
			{ device },
		),
	no_keys: (t, device) =>
		t(
			"devices:keys.error.noKeys",
			"This computer has no keys for {{device}}.",
			{ device },
		),
	held_elsewhere: (t, device) =>
		t(
			"devices:keys.error.heldElsewhere",
			"{{device}} is unlocked in another window. Lock it there first.",
			{ device },
		),
	lock_unsupported: (t) =>
		t(
			"devices:keys.error.lockUnsupported",
			"This browser can't protect device keys. Use the desktop app or a current browser.",
		),
	crypto_unavailable: (t) =>
		t(
			"devices:keys.error.cryptoUnavailable",
			"The encryption module couldn't be loaded. Reload the page and try again.",
		),
	changed_elsewhere: (t, device) =>
		t(
			"devices:keys.error.changedElsewhere",
			"The keys for {{device}} changed in another window while this ran. Reload and try again.",
			{ device },
		),
	exists: (t, device) =>
		t(
			"devices:keys.error.exists",
			"Other keys for {{device}} already exist on this computer.",
			{ device },
		),
	other_device: (t) =>
		t(
			"devices:keys.error.otherDevice",
			"This backup belongs to another device or hub.",
		),
	too_large: (t) =>
		t(
			"devices:keys.error.tooLarge",
			"Backup files must be smaller than 1 MiB.",
		),
	not_a_backup: (t) =>
		t("devices:keys.error.notABackup", "This isn't a key backup file."),
	legacy_shared: (t) =>
		t(
			"devices:keys.error.legacyShared",
			"This older shared-access backup can't be checked. Restore from your account backup, or request access again.",
		),
	same_password: (t) =>
		t(
			"devices:keys.error.samePassword",
			"Choose a different password from the current one.",
		),
	storage: (t) =>
		t(
			"devices:keys.error.storage",
			"This computer's storage refused the change. Nothing was saved.",
		),
	unknown: (t) =>
		t(
			"devices:keys.error.unknown",
			"It didn't finish. Nothing was changed on this computer.",
		),
};

/**
 * The sentence for a failed key flow; empty for a cancelled one. `hubCopy`
 * words the hub's part (the area's `hubErrorCopy`), so this module stays free
 * of React.
 */
export function keyFailureCopy(
	t: DevicesT,
	failure: KeyFailure,
	device: string,
	hubCopy?: (code: HubErrorCode) => string,
): string {
	if (failure.code === "cancelled") return "";
	if (failure.code === "hub")
		return t(
			"devices:keys.error.hub",
			"{{why}} Nothing was changed on this computer; try again in a moment.",
			{
				why:
					hubCopy?.(failure.hub ?? "network") ??
					t("devices:keys.error.hubUnreached", "The hub didn't answer."),
			},
		);
	return COPY[failure.code](t, device);
}
