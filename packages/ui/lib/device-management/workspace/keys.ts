import { ApiResponseError, isMissingResourceError } from "../../api-error";
import { parseDeviceRows } from "../../devices";
import { withPassword } from "../crypto";
import { identityFingerprint } from "../fingerprint";
import { type PreflightInput, runPreflight } from "../model/preflight";
import { relationshipOf } from "../model/presence";
import type {
	DeviceIdentity,
	DeviceRow,
	HubDeviceSupport,
	Relationship,
} from "../model/types";
import { saveAccountRecovery } from "../recovery";
import {
	type DeviceAccountScope,
	DeviceLockHeldError,
	DeviceLockUnsupportedError,
	type LocalDeviceVault,
	accountStorageKey,
	acquireDeviceLock,
	assertVaultAuthority,
	deviceApiBase,
	pinDeviceIdentity,
	queryDeviceLock,
	readDeviceVault,
	replaceRestoredVault,
} from "../storage";
import {
	type BrowserController,
	type DeviceCrypto,
	type DeviceReceipt,
	type HeldSignerController,
	supportsHeldSigner,
} from "../types";
import type {
	ActivityTracker,
	FleetPort,
	HubPort,
	KeyError,
	KeyHubError,
	KeySessionManager,
	KeySessionSnapshot,
	KeyState,
	LivePort,
	LocalPort,
	LocalSummary,
	OwnerSigner,
	StepDetailCode,
	UnlockManyOutcome,
	UnlockOptions,
	UnlockStep,
	UnlockStepId,
	VaultLease,
	WorkspaceDeps,
} from "./types";

export const IDLE_LOCK_MS = 30 * 60_000;
const TOUCH_RESOLUTION_MS = 1_000;

export class KeySessionError extends Error {
	constructor(readonly keyError: KeyError) {
		super(`Key session failed: ${keyError.code}.`);
		this.name = "KeySessionError";
	}
}

/**
 * This key session does not hold the invitation key (the user asked to be
 * prompted again, or the crypto bundle predates it), so the signature needs
 * the password.
 */
export class OwnerPasswordRequiredError extends Error {
	constructor() {
		super("Enter the device password to sign this access change.");
		this.name = "OwnerPasswordRequiredError";
	}
}

const ASK_PASSWORD_KEY = "flow-like/devices/ask-password-for-access-changes/";

/** "Ask for my password again for access changes" (IA §6.4.1): per account on this computer, off by default. */
export function readAskPasswordForAccessChanges(
	scope: DeviceAccountScope,
): boolean {
	try {
		return (
			globalThis.localStorage?.getItem(
				ASK_PASSWORD_KEY + accountStorageKey(scope),
			) === "1"
		);
	} catch {
		return false;
	}
}

/** False when this browser refused to store the choice. */
export function writeAskPasswordForAccessChanges(
	scope: DeviceAccountScope,
	ask: boolean,
): boolean {
	try {
		const storage = globalThis.localStorage;
		if (!storage) return false;
		const key = ASK_PASSWORD_KEY + accountStorageKey(scope);
		if (ask) storage.setItem(key, "1");
		else storage.removeItem(key);
		return true;
	} catch {
		return false;
	}
}

/** Hub, account and clock facts for D1–D3, D8 and D9. */
export interface KeyPreflightFacts {
	/** Hub-corrected unix seconds. */
	now: number;
	hub: HubDeviceSupport;
	auth: PreflightInput["auth"];
	device?: DeviceRow;
	relationship: Relationship;
	clock: PreflightInput["clock"];
}

export interface KeySessionPorts {
	local: LocalPort;
	live: LivePort;
	fleet: FleetPort;
	hub: HubPort;
	/** With `list`, an active tracked operation counts as use for the idle lock. */
	activity: Pick<ActivityTracker, "start" | "update" | "finish"> &
		Partial<Pick<ActivityTracker, "list">>;
	/** W2 wires react-query and the clock; the default reads the hub device list. */
	facts?: (deviceId: string) => Promise<KeyPreflightFacts>;
	/** Greyed "Locked · last read …", captured right before the keys are cleared. */
	lockedSummary?: (deviceId: string) => KeySessionSnapshot["lockedSummary"];
}

/** Storage, timer and page seams; tests replace them. */
export interface KeySessionIo {
	readDeviceVault: typeof readDeviceVault;
	acquireDeviceLock: typeof acquireDeviceLock;
	queryDeviceLock: typeof queryDeviceLock;
	replaceRestoredVault: typeof replaceRestoredVault;
	pinDeviceIdentity: typeof pinDeviceIdentity;
	saveAccountRecovery: typeof saveAccountRecovery;
	readAskPassword: typeof readAskPasswordForAccessChanges;
	writeAskPassword: typeof writeAskPasswordForAccessChanges;
	setTimer(run: () => void, ms: number): unknown;
	clearTimer(handle: unknown): void;
	onPageHide(listener: () => void): () => void;
}

const DEFAULT_IO: KeySessionIo = {
	readDeviceVault,
	acquireDeviceLock,
	queryDeviceLock,
	replaceRestoredVault,
	pinDeviceIdentity,
	saveAccountRecovery,
	readAskPassword: readAskPasswordForAccessChanges,
	writeAskPassword: writeAskPasswordForAccessChanges,
	setTimer: (run, ms) => setTimeout(run, ms),
	clearTimer: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
	onPageHide(listener) {
		if (typeof window === "undefined") return () => undefined;
		window.addEventListener("pagehide", listener);
		return () => window.removeEventListener("pagehide", listener);
	},
};

export type KeySessionRuntime = KeySessionManager & {
	/** Views, user calls and active operations count as use for the idle lock. */
	touch(deviceId: string): void;
	/** "Ask for my password again for access changes" (IA §6.4.1); off by default. */
	askPasswordForAccessChanges(): boolean;
	/** Turning it on drops every invitation key this window holds, at once. */
	setAskPasswordForAccessChanges(ask: boolean): void;
	/** Lock all and stop listening for page hide. */
	dispose(): void;
};

interface Session {
	deviceId: string;
	state: Exclude<KeyState, "none" | "stale">;
	vault?: LocalDeviceVault;
	controller?: BrowserController;
	/** The controller holds the owner's invitation key, so signatures need no password. */
	signerHeld: boolean;
	receipt?: DeviceReceipt;
	release?: () => void;
	lockToken?: object;
	liveDemand?: () => void;
	unlockedAt?: number;
	lastUsedAt?: number;
	keepUnlocked: boolean;
	timer?: unknown;
	lockedSummary?: KeySessionSnapshot["lockedSummary"];
	lastError?: KeySessionSnapshot["lastError"];
	generation: number;
	unlocking?: Promise<void>;
	leases: Promise<unknown>;
}

const KEY_STEP_DETAIL: Record<KeyError["code"], StepDetailCode> = {
	wrong_password: "wrong_password",
	no_vault: "no_vault",
	held_elsewhere: "held_elsewhere",
	lock_unsupported: "lock_unsupported",
	crypto_unavailable: "crypto_unavailable",
	identity_mismatch: "identity_mismatch",
	authority_mismatch: "authority_mismatch",
	storage: "storage_error",
};

const FAILED_STATE: Partial<Record<KeyError["code"], Session["state"]>> = {
	identity_mismatch: "blocked",
	held_elsewhere: "held_elsewhere",
};

type Progress = (
	id: UnlockStepId,
	state: UnlockStep["state"],
	detail?: StepDetailCode,
) => void;

interface UnlockRun {
	value: Session;
	generation: number;
	/** False when "Use here" already took the lock for this session. */
	lockedHere: boolean;
	options: UnlockOptions;
	step: UnlockStepId;
	/** Opened but not yet committed; closed and freed on any failure. */
	controller?: BrowserController;
	signerHeld: boolean;
	/** Set when the hub fails the identity read; the run still rejects with the hub's error. */
	hubError?: KeyHubError;
}

const MANY_OUTCOMES = new Set<KeyError["code"]>([
	"wrong_password",
	"no_vault",
	"held_elsewhere",
]);

/** `free` drops the controller with everything it holds, even when an earlier step throws. */
function closeController(controller: BrowserController | undefined) {
	if (!controller) return;
	try {
		controller.detachInvitation?.();
		controller.close();
	} finally {
		controller.free();
	}
}

function lockedWhileUnlocking() {
	return new DOMException(
		"The device was locked while unlocking.",
		"AbortError",
	);
}

function isAbort(error: unknown) {
	return error instanceof DOMException && error.name === "AbortError";
}

function keyErrorOf(error: unknown): KeyError | undefined {
	if (error instanceof KeySessionError) return error.keyError;
	if (error instanceof DeviceLockHeldError) return { code: "held_elsewhere" };
	if (error instanceof DeviceLockUnsupportedError)
		return { code: "lock_unsupported" };
	return undefined;
}

function failureDetail(error: unknown): StepDetailCode {
	const keyError = keyErrorOf(error);
	if (keyError) return KEY_STEP_DETAIL[keyError.code];
	return isAbort(error) ? "cancelled" : "http_error";
}

function hubErrorOf(error: unknown): KeyHubError {
	const status = (error as { status?: unknown } | null)?.status;
	return typeof status === "number" ? { code: "hub", status } : { code: "hub" };
}

/** `LocalVaultSummary.identityPinnedAt` is stored in milliseconds; copy and `KeyError` take seconds. */
const pinSeconds = (pinnedAtMs: number | undefined) =>
	pinnedAtMs === undefined ? undefined : Math.floor(pinnedAtMs / 1000);

function storageError(error: unknown): KeySessionError {
	return new KeySessionError({
		code: "storage",
		detail: error instanceof Error ? error.message : String(error),
	});
}

function safeFingerprint(identity: DeviceIdentity): string {
	try {
		return identityFingerprint(identity);
	} catch {
		return "";
	}
}

function isStale(facts: KeyPreflightFacts): boolean {
	const { device, relationship, now } = facts;
	if (!device) return false;
	const expiresAt = device.access_expires_at;
	return (
		device.status === "revoked" ||
		relationship === "none" ||
		(relationship === "shared" && expiresAt != null && expiresAt <= now)
	);
}

export function createKeySessionManager(
	deps: WorkspaceDeps,
	ports: KeySessionPorts,
	overrides: Partial<KeySessionIo> = {},
): KeySessionRuntime {
	const io: KeySessionIo = { ...DEFAULT_IO, ...overrides };
	const now = () => deps.now?.() ?? Date.now();
	const sessions = new Map<string, Session>();
	const stale = new Set<string>();
	const listeners = new Set<() => void>();
	let cache = new Map<string, KeySessionSnapshot>();
	let cachedList: KeySessionSnapshot[] | undefined;
	let cachedFor: LocalSummary | undefined;
	/** A choice this browser could not store still holds for this window. */
	let unsavedAsk: boolean | undefined;

	function emit() {
		cache = new Map();
		cachedList = undefined;
		for (const listener of listeners) listener();
	}

	/** Read at every use, so a change made in another window applies to the next signature. */
	function askPassword(): boolean {
		return unsavedAsk ?? io.readAskPassword(deps.scope);
	}

	function session(deviceId: string): Session {
		let value = sessions.get(deviceId);
		if (!value) {
			value = {
				deviceId,
				state: "locked",
				signerHeld: false,
				keepUnlocked: false,
				generation: 0,
				leases: Promise.resolve(),
			};
			sessions.set(deviceId, value);
		}
		return value;
	}

	function unlocked(deviceId: string): Session | undefined {
		const value = sessions.get(deviceId);
		return value?.state === "unlocked" && value.controller ? value : undefined;
	}

	function holdsSigner(deviceId: string): boolean {
		return unlocked(deviceId)?.signerHeld === true;
	}

	function reloadLocal() {
		void ports.local.reload().catch(() => undefined);
	}

	function stateOf(deviceId: string, hasVault: boolean): KeyState {
		const value = sessions.get(deviceId);
		const state = value?.state ?? "locked";
		if (!hasVault && !value?.controller && state !== "unlocking") return "none";
		if (state === "locked" && stale.has(deviceId)) return "stale";
		return state;
	}

	function idleLocksAt(value: Session, state: KeyState) {
		if (state !== "unlocked" || value.keepUnlocked) return undefined;
		return value.lastUsedAt === undefined
			? undefined
			: value.lastUsedAt + IDLE_LOCK_MS;
	}

	function build(deviceId: string, summary: LocalSummary): KeySessionSnapshot {
		const value = sessions.get(deviceId);
		const local = summary.vaults.find((row) => row.deviceId === deviceId);
		const grantId = value?.vault?.grantId ?? local?.grantId ?? "";
		const state = stateOf(deviceId, local !== undefined);
		const restored =
			value?.vault?.requiresFreshEndpoint ?? local?.requiresFreshEndpoint;
		const locksAt = value && idleLocksAt(value, state);
		const canSign = holdsSigner(deviceId);
		return {
			deviceId,
			state,
			role: grantId === "owner" ? "owner" : "shared",
			grantId,
			canSign,
			unlockedAt: value?.unlockedAt,
			lastUsedAt: value?.lastUsedAt,
			idleLocksAt: locksAt,
			keepUnlocked: value?.keepUnlocked ?? false,
			restoredNeedsFreshEndpoint: restored === true,
			lockedSummary: value?.lockedSummary,
			lastError: value?.lastError,
		};
	}

	function currentSummary(): LocalSummary {
		const summary = ports.local.summary();
		if (summary !== cachedFor) {
			cachedFor = summary;
			cache = new Map();
			cachedList = undefined;
		}
		return summary;
	}

	function snapshot(deviceId: string): KeySessionSnapshot {
		const summary = currentSummary();
		let value = cache.get(deviceId);
		if (!value) {
			value = build(deviceId, summary);
			cache.set(deviceId, value);
		}
		return value;
	}

	function list(): KeySessionSnapshot[] {
		const summary = currentSummary();
		if (!cachedList) {
			const ids = new Set(summary.vaults.map((row) => row.deviceId));
			for (const value of sessions.values())
				if (value.controller || value.state !== "locked")
					ids.add(value.deviceId);
			cachedList = [...ids].map(snapshot);
		}
		return cachedList;
	}

	function activeOperation(deviceId: string): boolean {
		return (
			ports.activity
				.list?.()
				.some(
					(item) =>
						item.target.deviceId === deviceId && item.state === "active",
				) ?? false
		);
	}

	function schedule(value: Session) {
		if (value.timer !== undefined) io.clearTimer(value.timer);
		value.timer = undefined;
		if (value.state !== "unlocked" || value.keepUnlocked) return;
		const generation = value.generation;
		const due = (value.lastUsedAt ?? now()) + IDLE_LOCK_MS;
		value.timer = io.setTimer(
			() => idle(value, generation),
			Math.max(0, due - now()),
		);
	}

	function idle(value: Session, generation: number) {
		if (value.generation !== generation || value.state !== "unlocked") return;
		value.timer = undefined;
		if (value.keepUnlocked) return;
		if (activeOperation(value.deviceId)) {
			touch(value.deviceId);
			return;
		}
		if (now() < (value.lastUsedAt ?? 0) + IDLE_LOCK_MS) schedule(value);
		else lockSession(value, "idle");
	}

	function touch(deviceId: string) {
		const value = unlocked(deviceId);
		if (!value) return;
		const at = now();
		if (at - (value.lastUsedAt ?? 0) < TOUCH_RESOLUTION_MS) return;
		value.lastUsedAt = at;
		schedule(value);
		emit();
	}

	function lockSession(value: Session, reason: "lock" | "idle" | "lost") {
		const hadKeys = Boolean(value.controller) || value.state === "unlocking";
		const summary = value.controller
			? ports.lockedSummary?.(value.deviceId)
			: undefined;
		value.generation++;
		if (value.timer !== undefined) io.clearTimer(value.timer);
		value.timer = undefined;
		value.liveDemand?.();
		value.liveDemand = undefined;
		if (hadKeys) ports.live.close(value.deviceId);
		closeController(value.controller);
		value.controller = undefined;
		value.signerHeld = false;
		value.receipt = undefined;
		value.vault = undefined;
		value.unlocking = undefined;
		value.unlockedAt = undefined;
		value.lastUsedAt = undefined;
		if (reason !== "lost") value.release?.();
		value.release = undefined;
		value.lockToken = undefined;
		if (summary) value.lockedSummary = summary;
		if (reason === "lost") {
			value.state = "held_elsewhere";
			value.lastError = { code: "held_elsewhere" };
		} else if (value.state !== "blocked") {
			value.state = "locked";
			value.lastError = undefined;
		}
		emit();
	}

	/** The session keeps this lock until lock, idle lock, page hide or a steal by another window. */
	async function holdLock(value: Session, steal: boolean) {
		const token = {};
		value.lockToken = token;
		let release: () => void;
		try {
			release = await io.acquireDeviceLock(deps.scope, value.deviceId, {
				steal,
				onLost: () => {
					if (value.lockToken === token) lockSession(value, "lost");
				},
			});
		} catch (error) {
			if (value.lockToken === token) value.lockToken = undefined;
			throw new KeySessionError(
				keyErrorOf(error) ?? storageError(error).keyError,
			);
		}
		if (value.lockToken !== token) {
			release();
			throw lockedWhileUnlocking();
		}
		value.release = release;
	}

	function identityMismatch(
		deviceId: string,
		reported: DeviceIdentity,
	): KeySessionError {
		const pinned = ports.local
			.summary()
			.vaults.find((row) => row.deviceId === deviceId);
		return new KeySessionError({
			code: "identity_mismatch",
			pinnedAt: pinSeconds(pinned?.identityPinnedAt) ?? 0,
			fingerprint: pinned?.identityFingerprint ?? "",
			reported: safeFingerprint(reported),
		});
	}

	async function verifyIdentity(
		crypto: DeviceCrypto,
		controller: BrowserController,
		vault: LocalDeviceVault,
		alive: () => void,
		onHubFailure: (error: unknown) => void,
	): Promise<DeviceReceipt> {
		const deviceId = vault.deviceId;
		const publicKey = controller.publicBundle();
		if (
			publicKey.device_id !== deviceId ||
			publicKey.controller_key.x !== vault.controllerPublic.controller_key.x
		)
			throw new KeySessionError({ code: "authority_mismatch" });
		const receipt = await ports.hub
			.fetch<DeviceReceipt>(`devices/${encodeURIComponent(deviceId)}/identity`)
			.catch((error: unknown) => {
				if (!isAbort(error)) onHubFailure(error);
				throw error;
			});
		alive();
		try {
			const accepted = crypto.verifyDeviceReceipt(
				receipt,
				vault.manifestJws,
				vault.ownerControllerKey ?? publicKey.controller_key,
			);
			if (
				receipt.device_id !== deviceId ||
				accepted.device_id !== deviceId ||
				accepted.api_base_url !== deviceApiBase(deps.scope)
			)
				throw new Error(
					"The signed identity belongs to another device or hub.",
				);
			assertVaultAuthority(deps.scope, vault, accepted);
		} catch {
			throw new KeySessionError({ code: "authority_mismatch" });
		}
		if (ports.local.identityCheck(deviceId, receipt.identity) === "mismatch")
			throw identityMismatch(deviceId, receipt.identity);
		try {
			await io.pinDeviceIdentity(deps.scope, deviceId, receipt);
		} catch (error) {
			throw error instanceof Error && error.message.includes("differ from")
				? identityMismatch(deviceId, receipt.identity)
				: storageError(error);
		}
		reloadLocal();
		return receipt;
	}

	async function openController(
		crypto: DeviceCrypto,
		vault: LocalDeviceVault,
		password: string,
		progress: Progress,
	): Promise<{ controller: BrowserController; vault: LocalDeviceVault }> {
		return withPassword(password, async (bytes) => {
			let controller: BrowserController;
			try {
				controller = crypto.unlockControllerVault(
					vault.deviceId,
					bytes,
					vault.controllerVault,
				);
			} catch {
				throw new KeySessionError({ code: "wrong_password" });
			}
			if (!vault.requiresFreshEndpoint) return { controller, vault };
			try {
				progress("unlocking_keys", "done");
				progress("rotating_endpoint", "active");
				const fresh = controller.freshEndpointVault(bytes);
				const next: LocalDeviceVault = {
					...vault,
					controllerPublic: fresh.public_bundle,
					controllerVault: Uint8Array.from(fresh.vault),
					requiresFreshEndpoint: false,
				};
				await io
					.replaceRestoredVault(deps.scope, vault, next)
					.catch((error: unknown) => {
						throw storageError(error);
					});
				return { controller, vault: next };
			} catch (error) {
				closeController(controller);
				throw error;
			}
		});
	}

	/**
	 * Owners hold their invitation key from unlock on, unless they asked to be
	 * prompted again. A failed attach keeps the unlock; the signer then asks
	 * for the password.
	 */
	async function holdSignerAtUnlock(
		controller: BrowserController,
		vault: LocalDeviceVault,
		password: string,
	): Promise<boolean> {
		const invitation = vault.invitationVault;
		const holder = supportsHeldSigner(controller) ? controller : undefined;
		if (vault.grantId !== "owner" || !invitation || !holder || askPassword())
			return false;
		return withPassword(password, (bytes) => {
			try {
				holder.attachInvitation(bytes, invitation);
				return true;
			} catch {
				return false;
			}
		});
	}

	function dropSigner(value: Session): boolean {
		if (!value.signerHeld) return false;
		value.signerHeld = false;
		value.controller?.detachInvitation?.();
		return true;
	}

	async function backupWithPassword(
		value: Session,
		password: string,
		crypto: DeviceCrypto,
		progress: Progress,
	) {
		progress("saving_backup", "active");
		const activity = ports.activity.start({
			kind: "account_backup",
			target: { deviceId: value.deviceId },
			state: "active",
			label: { code: "account_backup" },
			progress: "indeterminate",
			actions: [],
			startedBy: "you",
		});
		try {
			await manager.withVaultLease(value.deviceId, (lease) =>
				io.saveAccountRecovery({
					api: deps.api,
					profile: deps.profile,
					scope: deps.scope,
					deviceId: value.deviceId,
					password,
					lease,
					crypto,
				}),
			);
			ports.activity.finish(activity, "done");
			progress("saving_backup", "done", "backup_saved");
		} catch (error) {
			ports.activity.finish(activity, "failed", { code: "failed" });
			progress(
				"saving_backup",
				"failed",
				error instanceof ApiResponseError && error.status === 429
					? "backup_limit"
					: "backup_local_only",
			);
		}
	}

	async function readEncryptedStatus(deviceId: string, progress: Progress) {
		progress("reading_encrypted_status", "active");
		try {
			await ports.fleet.refresh(deviceId);
			progress("reading_encrypted_status", "done");
		} catch {
			progress("reading_encrypted_status", "failed");
		}
	}

	function reporter(run: UnlockRun): Progress {
		return (id, state, detail) => {
			if (state === "active") run.step = id;
			run.options.onProgress?.({
				id,
				state,
				...(detail && { detail: { code: detail } }),
			});
		};
	}

	function aliveCheck(run: UnlockRun) {
		return () => {
			if (run.value.generation !== run.generation) throw lockedWhileUnlocking();
			run.options.signal?.throwIfAborted();
		};
	}

	async function openKeys(run: UnlockRun, password: string) {
		const { value } = run;
		const progress = reporter(run);
		const alive = aliveCheck(run);
		progress("unlocking_keys", "active");
		const readVault = async () => {
			const stored = await io
				.readDeviceVault(deps.scope, value.deviceId)
				.catch((error: unknown) => {
					throw storageError(error);
				});
			if (!stored) throw new KeySessionError({ code: "no_vault" });
			return stored;
		};
		// A device without keys takes no lock, and this read lets a lock this
		// window released a moment ago drain before it is requested again.
		await readVault();
		alive();
		if (run.lockedHere) await holdLock(value, false);
		alive();
		// Until the lock is held, another window may still rewrap or rotate the vault.
		const stored = await readVault();
		alive();
		const crypto = await deps.crypto().catch(() => {
			throw new KeySessionError({ code: "crypto_unavailable" });
		});
		alive();
		const opened = await openController(crypto, stored, password, progress);
		run.controller = opened.controller;
		if (opened.vault === stored) {
			progress("unlocking_keys", "done");
			progress("rotating_endpoint", "skipped");
		} else progress("rotating_endpoint", "done", "fresh_endpoint");
		alive();
		progress("checking_identity", "active");
		const receipt = await verifyIdentity(
			crypto,
			opened.controller,
			opened.vault,
			alive,
			(error) => {
				run.hubError = hubErrorOf(error);
			},
		);
		alive();
		run.signerHeld = await holdSignerAtUnlock(
			opened.controller,
			opened.vault,
			password,
		);
		alive();
		progress("checking_identity", "done");
		return { crypto, vault: opened.vault, receipt };
	}

	function commitUnlock(
		run: UnlockRun,
		vault: LocalDeviceVault,
		receipt: DeviceReceipt,
	) {
		const { value, options } = run;
		value.controller = run.controller;
		run.controller = undefined;
		value.signerHeld = run.signerHeld;
		value.vault = vault;
		value.receipt = receipt;
		value.state = "unlocked";
		value.unlockedAt = now();
		value.lastUsedAt = value.unlockedAt;
		value.lockedSummary = undefined;
		value.keepUnlocked = options.keepUnlocked ?? value.keepUnlocked;
		schedule(value);
		emit();
	}

	function failUnlock(run: UnlockRun, error: unknown) {
		const { value } = run;
		closeController(run.controller);
		run.controller = undefined;
		const keyError = keyErrorOf(error);
		reporter(run)(run.step, "failed", failureDetail(error));
		if (value.generation !== run.generation) return;
		if (run.lockedHere) {
			value.release?.();
			value.release = undefined;
			value.lockToken = undefined;
		}
		value.state = (keyError && FAILED_STATE[keyError.code]) ?? "locked";
		value.lastError = keyError ?? run.hubError;
		emit();
	}

	async function afterUnlock(
		run: UnlockRun,
		password: string,
		crypto: DeviceCrypto,
	) {
		const { value, options } = run;
		const progress = reporter(run);
		if (options.backupToAccount)
			await backupWithPassword(value, password, crypto, progress);
		else progress("saving_backup", "skipped", "not_requested");
		if (value.generation !== run.generation) return;
		if (!options.connectLive)
			return readEncryptedStatus(value.deviceId, progress);
		value.liveDemand ??= ports.live.acquire(value.deviceId, "view");
		void ports.fleet.refresh(value.deviceId).catch(() => undefined);
	}

	async function runUnlock(
		value: Session,
		password: string,
		options: UnlockOptions,
	) {
		const run: UnlockRun = {
			value,
			generation: ++value.generation,
			lockedHere: !value.release,
			options,
			step: "unlocking_keys",
			signerHeld: false,
		};
		value.state = "unlocking";
		value.lastError = undefined;
		emit();
		let opened: Awaited<ReturnType<typeof openKeys>>;
		try {
			opened = await openKeys(run, password);
			commitUnlock(run, opened.vault, opened.receipt);
		} catch (error) {
			failUnlock(run, error);
			throw error;
		}
		await afterUnlock(run, password, opened.crypto);
	}

	async function heldLease<T>(
		value: Session,
		run: (lease: VaultLease) => Promise<T>,
	): Promise<T> {
		if (!value.release) return transientLease(value.deviceId, run);
		const vault =
			value.vault ?? (await io.readDeviceVault(deps.scope, value.deviceId));
		if (!vault) throw new KeySessionError({ code: "no_vault" });
		touch(value.deviceId);
		try {
			return await run({
				vault,
				replace(next) {
					if (value.vault && next.deviceId === value.deviceId) {
						value.vault = next;
						emit();
					}
				},
			});
		} finally {
			reloadLocal();
		}
	}

	async function transientLease<T>(
		deviceId: string,
		run: (lease: VaultLease) => Promise<T>,
	): Promise<T> {
		let release: () => void;
		try {
			release = await io.acquireDeviceLock(deps.scope, deviceId);
		} catch (error) {
			throw new KeySessionError(
				keyErrorOf(error) ?? storageError(error).keyError,
			);
		}
		try {
			const vault = await io.readDeviceVault(deps.scope, deviceId);
			if (!vault) throw new KeySessionError({ code: "no_vault" });
			return await run({ vault, replace: () => undefined });
		} finally {
			release();
			reloadLocal();
		}
	}

	function heldSigner(value: Session): HeldSignerController | undefined {
		const controller = value.controller;
		return value.signerHeld && controller && supportsHeldSigner(controller)
			? controller
			: undefined;
	}

	/**
	 * A typed password also starts holding the key, so later changes in this
	 * key session ask for nothing. Throws on a wrong password.
	 */
	function holdSignerFromPassword(
		value: Session,
		generation: number,
		bytes: Uint8Array,
		invitation: Uint8Array,
	): HeldSignerController | undefined {
		const controller = value.controller;
		if (
			value.generation !== generation ||
			!controller ||
			!supportsHeldSigner(controller)
		)
			return undefined;
		controller.attachInvitation(bytes, invitation);
		value.signerHeld = true;
		emit();
		return controller;
	}

	function signer(deviceId: string): OwnerSigner {
		const sign = async (
			password: string | undefined,
			held: (controller: HeldSignerController) => string,
			typed: (
				crypto: DeviceCrypto,
				bytes: Uint8Array,
				invitation: Uint8Array,
			) => string,
		) => {
			const value = unlocked(deviceId);
			const invitation = value?.vault?.invitationVault;
			if (!value || !invitation)
				throw new KeySessionError({ code: "no_vault" });
			const ask = askPassword();
			if (ask && dropSigner(value)) emit();
			const holder = heldSigner(value);
			if (holder) {
				touch(deviceId);
				return held(holder);
			}
			if (password === undefined) throw new OwnerPasswordRequiredError();
			const generation = value.generation;
			const crypto = await deps.crypto();
			touch(deviceId);
			return withPassword(password, (bytes) => {
				const attached = ask
					? undefined
					: holdSignerFromPassword(value, generation, bytes, invitation);
				return attached ? held(attached) : typed(crypto, bytes, invitation);
			});
		};
		return {
			signPolicy: (policy, password) =>
				sign(
					password,
					(controller) => controller.signManagementPolicyHeld(policy),
					(crypto, bytes, invitation) =>
						crypto.signManagementPolicy(policy, bytes, invitation),
				),
			signTelemetryRoster: (roster, password) =>
				sign(
					password,
					(controller) => controller.signTelemetryRosterHeld(roster),
					(crypto, bytes, invitation) =>
						crypto.signTelemetryRoster(roster, bytes, invitation),
				),
			signArchiveRoster: (roster, password) =>
				sign(
					password,
					(controller) => controller.signArchiveRosterHeld(roster),
					(crypto, bytes, invitation) =>
						crypto.signArchiveRoster(roster, bytes, invitation),
				),
		};
	}

	async function hubFacts(deviceId: string): Promise<KeyPreflightFacts> {
		const base = {
			now: Math.floor(now() / 1000),
			auth: { signedIn: Boolean(deps.scope.account), tokenScopeAll: true },
			clock: {},
		};
		try {
			const device = parseDeviceRows(
				await ports.hub.fetch<unknown>("devices"),
			).find((row) => row.device_id === deviceId);
			return {
				...base,
				hub: { state: "on" },
				device,
				relationship: device
					? relationshipOf(device, deps.scope.account)
					: "unknown",
			};
		} catch (error) {
			return {
				...base,
				hub: { state: isMissingResourceError(error) ? "off" : "unreachable" },
				relationship: "unknown",
			};
		}
	}

	/** Identity and lock-holder findings from pre-flight become the visible state of a session without keys. */
	function applyPreflight(
		deviceId: string,
		lock: PreflightInput["lock"],
		identity: "match" | "mismatch" | "unpinned",
		reported: DeviceIdentity | undefined,
	) {
		const value = sessions.get(deviceId);
		if (value?.controller || value?.state === "unlocking") return;
		const finding: Pick<Session, "state" | "lastError"> | undefined =
			identity === "mismatch" && reported
				? {
						state: "blocked",
						lastError: identityMismatch(deviceId, reported).keyError,
					}
				: lock === "held_elsewhere"
					? { state: "held_elsewhere", lastError: { code: "held_elsewhere" } }
					: undefined;
		if (finding) {
			const target = session(deviceId);
			if (
				target.state === finding.state &&
				target.lastError?.code === finding.lastError?.code
			)
				return;
			target.state = finding.state;
			target.lastError = finding.lastError;
			emit();
		} else if (
			value?.state === "blocked" ||
			value?.state === "held_elsewhere"
		) {
			value.state = "locked";
			value.lastError = undefined;
			emit();
		}
	}

	const manager: KeySessionRuntime = {
		snapshot,
		list,
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		async preflight(deviceId) {
			const [facts, lock] = await Promise.all([
				(ports.facts ?? hubFacts)(deviceId),
				io.queryDeviceLock(deps.scope, deviceId),
			]);
			const wasStale = stale.has(deviceId);
			if (isStale(facts)) stale.add(deviceId);
			else stale.delete(deviceId);
			if (wasStale !== stale.has(deviceId)) emit();
			const summary = ports.local.summary();
			const pinned = summary.vaults.find((row) => row.deviceId === deviceId);
			const reported = facts.device?.identity;
			const check = reported
				? ports.local.identityCheck(deviceId, reported)
				: pinned?.identityPinnedAt !== undefined
					? "match"
					: "unpinned";
			applyPreflight(deviceId, lock, check, reported);
			const keys = snapshot(deviceId);
			return runPreflight({
				now: facts.now,
				deviceId,
				platform: deps.platform,
				hub: facts.hub,
				auth: facts.auth,
				device: facts.device,
				relationship: facts.relationship,
				keys: { state: keys.state, role: keys.role },
				local: summary,
				lock,
				identity: {
					check,
					pinnedAt: pinSeconds(pinned?.identityPinnedAt),
					fingerprint: pinned?.identityFingerprint,
				},
				clock: facts.clock,
			});
		},
		unlock(deviceId, password, options = {}) {
			const value = session(deviceId);
			if (unlocked(deviceId)) {
				touch(deviceId);
				if (options.keepUnlocked !== undefined)
					manager.setKeepUnlocked(deviceId, options.keepUnlocked);
				if (options.connectLive)
					value.liveDemand ??= ports.live.acquire(deviceId, "view");
				return Promise.resolve();
			}
			if (value.unlocking) return value.unlocking;
			const run: Promise<void> = runUnlock(value, password, options).finally(
				() => {
					if (value.unlocking === run) value.unlocking = undefined;
				},
			);
			value.unlocking = run;
			return run;
		},
		async unlockMany(deviceIds, password, onEach, signal) {
			for (const deviceId of deviceIds) {
				if (signal?.aborted) return;
				let outcome: UnlockManyOutcome = "unlocked";
				try {
					await manager.unlock(deviceId, password, { signal });
				} catch (error) {
					if (signal?.aborted) return;
					const code = keyErrorOf(error)?.code;
					outcome =
						code && MANY_OUTCOMES.has(code)
							? (code as UnlockManyOutcome)
							: "error";
				}
				onEach(deviceId, outcome);
			}
		},
		async takeOver(deviceId) {
			const value = session(deviceId);
			if (value.release) return;
			try {
				await holdLock(value, true);
			} catch (error) {
				value.lastError = keyErrorOf(error);
				emit();
				throw error;
			}
			value.state = "locked";
			value.lastError = undefined;
			emit();
		},
		lock(deviceId) {
			const value = sessions.get(deviceId);
			if (value) lockSession(value, "lock");
		},
		lockAll() {
			for (const value of sessions.values())
				if (value.controller || value.release || value.unlocking)
					lockSession(value, "lock");
		},
		setKeepUnlocked(deviceId, keep) {
			const value = session(deviceId);
			value.keepUnlocked = keep;
			if (!keep && value.state === "unlocked") value.lastUsedAt = now();
			schedule(value);
			emit();
		},
		withVaultLease(deviceId, run) {
			const value = sessions.get(deviceId);
			if (!value?.release) return transientLease(deviceId, run);
			const result = value.leases.then(() => heldLease(value, run));
			value.leases = result.then(
				() => undefined,
				() => undefined,
			);
			return result;
		},
		controller: (deviceId) => unlocked(deviceId)?.controller,
		vault: (deviceId) => unlocked(deviceId)?.vault,
		receipt: (deviceId) => unlocked(deviceId)?.receipt,
		signer: (deviceId) => {
			const value = unlocked(deviceId);
			return value?.vault?.grantId === "owner" && value.vault.invitationVault
				? signer(deviceId)
				: undefined;
		},
		touch,
		askPasswordForAccessChanges: askPassword,
		setAskPasswordForAccessChanges(ask) {
			unsavedAsk = io.writeAskPassword(deps.scope, ask) ? undefined : ask;
			if (ask) for (const value of sessions.values()) dropSigner(value);
			emit();
		},
		dispose() {
			stopPageHide();
			manager.lockAll();
		},
	};
	const stopPageHide = io.onPageHide(() => manager.lockAll());
	return manager;
}
