import { afterAll, beforeEach, expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import { ApiResponseError } from "../../api-error";
import { base64url } from "../crypto";
import { identityFingerprint } from "../fingerprint";
import {
	type LocalDeviceVault,
	accountStorageKey,
	queryDeviceLock,
} from "../storage";
import type {
	ArchiveRoster,
	BrowserController,
	DeviceCrypto,
	DeviceReceipt,
	ManagementPolicy,
	TelemetryRoster,
} from "../types";
import {
	IDLE_LOCK_MS,
	type KeyPreflightFacts,
	KeySessionError,
	type KeySessionIo,
	type KeySessionPorts,
	OwnerPasswordRequiredError,
	createKeySessionManager,
	readAskPasswordForAccessChanges,
	writeAskPasswordForAccessChanges,
} from "./keys";
import type {
	ActivityItem,
	LocalSummary,
	UnlockStep,
	WorkspaceDeps,
} from "./types";

const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const PASSWORD = "correct horse";
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const lockName = (deviceId: string) =>
	JSON.stringify([accountStorageKey(scope), deviceId, "unlock"]);

type Held = { reject: (error: unknown) => void };
function fakeLocks() {
	const held = new Map<string, Held>();
	let requests = 0;
	return {
		held,
		get requests() {
			return requests;
		},
		request(
			name: string,
			options: { ifAvailable?: boolean; steal?: boolean },
			run: (lock: object | null) => Promise<void>,
		) {
			requests++;
			return new Promise<void>((resolve, reject) => {
				const current = held.get(name);
				if (current && !options.steal) {
					run(null).then(resolve, reject);
					return;
				}
				if (current) {
					held.delete(name);
					current.reject(new DOMException("Lock stolen", "AbortError"));
				}
				const entry: Held = { reject };
				held.set(name, entry);
				run({ name }).then(() => {
					if (held.get(name) === entry) held.delete(name);
					resolve();
				}, reject);
			});
		},
		async query() {
			return { held: [...held.keys()].map((name) => ({ name })) };
		},
	};
}
function otherWindow(locks: ReturnType<typeof fakeLocks>, deviceId: string) {
	let lost: unknown;
	void locks
		.request(lockName(deviceId), { steal: true }, () => new Promise(() => {}))
		.catch((error) => {
			lost = error;
		});
	return () => lost;
}

const originalNavigator = Object.getOwnPropertyDescriptor(
	globalThis,
	"navigator",
);
let locks = fakeLocks();
beforeEach(() => {
	locks = fakeLocks();
	Object.defineProperty(globalThis, "navigator", {
		configurable: true,
		value: { locks },
	});
});
afterAll(() => {
	if (originalNavigator)
		Object.defineProperty(globalThis, "navigator", originalNavigator);
	else Reflect.deleteProperty(globalThis, "navigator");
});

const key = (fill: number) => ({
	kty: "OKP" as const,
	crv: "Ed25519" as const,
	x: base64url(new Uint8Array(32).fill(fill)),
});
function vault(
	deviceId: string,
	options: { owner?: boolean; restored?: boolean; secret?: string } = {},
): LocalDeviceVault {
	const owner = options.owner ?? true;
	return {
		deviceId,
		grantId: owner ? "owner" : "reader-grant",
		manifestJws: `manifest-${deviceId}`,
		ownerControllerKey: owner ? undefined : key(9),
		controllerPublic: {
			device_id: deviceId,
			endpoint_id: "endpoint",
			controller_key: key(1),
			archive_key: Array(32).fill(7),
			telemetry_member: { endpoint_id: "endpoint", signing_key: key(2) },
		},
		controllerVault: new TextEncoder().encode(options.secret ?? PASSWORD),
		invitationVault: owner ? new Uint8Array(80).fill(4) : undefined,
		requiresFreshEndpoint: options.restored,
	};
}
function identity(seed: number): DeviceReceipt["identity"] {
	return {
		auth_key: key(seed),
		telemetry_key: key(seed + 1),
		management_key: Array(32).fill(seed + 2),
	};
}
function receipt(deviceId: string): DeviceReceipt {
	return {
		enrollment_id: "enrollment",
		device_id: deviceId,
		owner_id: scope.account,
		name: deviceId,
		identity: identity(20),
		manifest_jws: "manifest",
		binding_jws: "binding",
		registered_at: 1,
		auth_epoch: 1,
	};
}

const WRONG_PASSWORD = "Incorrect password or damaged vault";

interface HeldSignerFake {
	passwords: Uint8Array[];
	/** Runs inside the attach, before the password is checked. */
	onAttach?: () => void;
}

function fakeController(stored: LocalDeviceVault, held?: HeldSignerFake) {
	const state = { closed: 0, freed: 0 };
	const signer = { attached: 0, detached: 0, holding: false };
	let publicBundle = stored.controllerPublic;
	const sign = (signature: string) => () => {
		if (!signer.holding)
			throw new Error("The owner invitation key is not held");
		return signature;
	};
	const controller = {
		publicBundle: () => publicBundle,
		freshEndpointVault(bytes: Uint8Array) {
			expect(new TextDecoder().decode(bytes)).toBe(PASSWORD);
			publicBundle = { ...publicBundle, endpoint_id: "fresh-endpoint" };
			return { public_bundle: publicBundle, vault: Array(80).fill(6) };
		},
		...(held && {
			attachInvitation(bytes: Uint8Array, invitation: Uint8Array) {
				held.passwords.push(bytes);
				held.onAttach?.();
				if (new TextDecoder().decode(bytes) !== PASSWORD)
					throw new Error(WRONG_PASSWORD);
				expect(invitation).toBe(stored.invitationVault as Uint8Array);
				signer.attached++;
				signer.holding = true;
			},
			detachInvitation() {
				if (signer.holding) signer.detached++;
				signer.holding = false;
			},
			signManagementPolicyHeld: sign("held-policy"),
			signTelemetryRosterHeld: sign("held-telemetry"),
			signArchiveRosterHeld: sign("held-archive"),
		}),
		close() {
			state.closed++;
			signer.holding = false;
		},
		free() {
			state.freed++;
		},
	} as unknown as BrowserController;
	return { controller, state, signer };
}

function harness(
	vaults: LocalDeviceVault[] = [vault("dev")],
	options: {
		platform?: "web" | "desktop";
		/** False simulates a crypto bundle built before the held owner key. */
		heldSigner?: boolean;
	} = {},
) {
	let clock = 1_000_000;
	const stored = new Map(vaults.map((row) => [row.deviceId, row]));
	const opened: ReturnType<typeof fakeController>[] = [];
	const passwords: Uint8Array[] = [];
	const calls: string[] = [];
	const signed: { policy: unknown; invitation: Uint8Array }[] = [];
	const held: HeldSignerFake = { passwords };
	let askStored = false;
	let storageBlocked = false;
	const typed =
		(signature: string) =>
		(payload: unknown, bytes: Uint8Array, invitation: Uint8Array) => {
			passwords.push(bytes);
			if (new TextDecoder().decode(bytes) !== PASSWORD)
				throw new Error(WRONG_PASSWORD);
			signed.push({ policy: payload, invitation });
			return signature;
		};
	const crypto = {
		unlockControllerVault(
			deviceId: string,
			bytes: Uint8Array,
			ciphertext: Uint8Array,
		) {
			passwords.push(bytes);
			calls.push(`unlock ${deviceId}`);
			if (
				new TextDecoder().decode(bytes) !== new TextDecoder().decode(ciphertext)
			)
				throw new Error("aead::Error");
			const value = fakeController(
				stored.get(deviceId) as LocalDeviceVault,
				options.heldSigner === false ? undefined : held,
			);
			opened.push(value);
			return value.controller;
		},
		verifyDeviceReceipt(value: DeviceReceipt, manifest: string) {
			const row = stored.get(value.device_id) as LocalDeviceVault;
			expect(manifest).toBe(row.manifestJws);
			return {
				device_id: value.device_id,
				api_base_url: "https://hub.test/api/v1",
				owner_id: scope.account,
				controller_key:
					row.ownerControllerKey ?? row.controllerPublic.controller_key,
			};
		},
		signManagementPolicy: typed("signed-policy"),
		signTelemetryRoster: typed("signed-telemetry"),
		signArchiveRoster: typed("signed-archive"),
	} as unknown as DeviceCrypto;

	const timers = new Map<number, { at: number; run: () => void }>();
	let timerId = 0;
	let pageHide: (() => void) | undefined;
	const pinned: string[] = [];
	const replaced: LocalDeviceVault[] = [];
	const backups: { password: string; leaseVault?: LocalDeviceVault }[] = [];
	let backupError: unknown;
	const io: Partial<KeySessionIo> = {
		async readDeviceVault(_scope, deviceId) {
			return stored.get(deviceId);
		},
		async replaceRestoredVault(_scope, previous, next) {
			expect(previous.requiresFreshEndpoint).toBe(true);
			replaced.push(next);
			stored.set(next.deviceId, next);
		},
		async pinDeviceIdentity(_scope, deviceId) {
			pinned.push(deviceId);
		},
		async saveAccountRecovery(input) {
			if (backupError) throw backupError;
			backups.push({
				password: input.password,
				leaseVault: input.lease?.vault,
			});
			return 1;
		},
		readAskPassword: () => askStored,
		writeAskPassword(_scope, ask) {
			if (storageBlocked) return false;
			askStored = ask;
			return true;
		},
		setTimer(run, ms) {
			timers.set(++timerId, { at: clock + ms, run });
			return timerId;
		},
		clearTimer(handle) {
			timers.delete(handle as number);
		},
		onPageHide(listener) {
			pageHide = listener;
			return () => {
				pageHide = undefined;
			};
		},
	};

	let identityCheck: "match" | "mismatch" | "unpinned" = "match";
	let summary: LocalSummary = {
		platform: options.platform ?? "desktop",
		persistence: "persisted",
		webLocks: true,
		indexedDb: true,
		cryptoLoaded: true,
		vaults: vaults.map((row) => ({
			deviceId: row.deviceId,
			role: row.grantId === "owner" ? "owner" : "shared",
			grantId: row.grantId,
			requiresFreshEndpoint: Boolean(row.requiresFreshEndpoint),
			identityPinnedAt: 500_999,
			identityFingerprint: identityFingerprint(identity(20)),
		})),
		backups: {},
		authorities: [],
	};
	const live = {
		acquired: [] as string[],
		released: 0,
		closed: [] as string[],
	};
	const activity: ActivityItem[] = [];
	const fleetRefreshed: string[] = [];
	let identityFetch: (deviceId: string) => Promise<DeviceReceipt> = async (
		deviceId,
	) => receipt(deviceId);
	let rows: unknown[] = [];
	const ports: KeySessionPorts = {
		local: {
			summary: () => summary,
			async reload() {
				calls.push("local reload");
			},
			identityCheck: () => identityCheck,
		},
		live: {
			acquire(deviceId, reason) {
				live.acquired.push(`${deviceId}:${reason}`);
				return () => {
					live.released++;
				};
			},
			call: () => {
				throw new Error("not used");
			},
			exclusive: () => Promise.reject(new Error("not used")),
			state: () => ({ kind: "idle" }),
			close(deviceId) {
				live.closed.push(deviceId);
			},
		},
		fleet: {
			watch: () => () => undefined,
			async refresh(deviceId) {
				fleetRefreshed.push(deviceId);
			},
			get: () => undefined,
		},
		hub: {
			async fetch<T>(path: string) {
				calls.push(`GET ${path}`);
				const identityPath = /^devices\/([^/]+)\/identity$/u.exec(path);
				if (identityPath)
					return (await identityFetch(
						decodeURIComponent(identityPath[1]),
					)) as T;
				if (path === "devices") return rows as T;
				throw new Error(`Unexpected ${path}`);
			},
		},
		activity: {
			start(item) {
				activity.push({
					...item,
					id: `a${activity.length}`,
					startedAt: 0,
					updatedAt: 0,
				});
				return `a${activity.length - 1}`;
			},
			update() {},
			finish(id, outcome) {
				const item = activity.find((row) => row.id === id);
				if (item) item.state = outcome;
			},
			list: () => activity,
		},
		lockedSummary: () => ({ readAt: 42, services: [] }),
	};
	const deps: WorkspaceDeps = {
		api: {} as IApiState,
		profile: {} as IProfile,
		scope,
		queryClient: {} as QueryClient,
		crypto: async () => crypto,
		platform: options.platform ?? "desktop",
		now: () => clock,
	};
	const keys = createKeySessionManager(deps, ports, io);
	return {
		keys,
		ports,
		calls,
		opened,
		passwords,
		pinned,
		replaced,
		backups,
		signed,
		live,
		activity,
		fleetRefreshed,
		stored,
		get pageHide() {
			return pageHide;
		},
		advance(ms: number) {
			clock += ms;
			for (const [id, timer] of [...timers].sort((a, b) => a[1].at - b[1].at))
				if (timer.at <= clock && timers.delete(id)) timer.run();
		},
		get pendingTimers() {
			return timers.size;
		},
		setIdentityCheck(value: typeof identityCheck) {
			identityCheck = value;
		},
		setIdentityFetch(value: typeof identityFetch) {
			identityFetch = value;
		},
		setBackupError(value: unknown) {
			backupError = value;
		},
		setRows(value: unknown[]) {
			rows = value;
		},
		setSummary(patch: Partial<LocalSummary>) {
			summary = { ...summary, ...patch };
		},
		/** The stored choice changes without a call here, as from another window. */
		setAskStored(value: boolean) {
			askStored = value;
		},
		blockStorage() {
			storageBlocked = true;
		},
		onAttach(run: (() => void) | undefined) {
			held.onAttach = run;
		},
	};
}

function row(deviceId: string, patch: Record<string, unknown> = {}) {
	return {
		device_id: deviceId,
		owner_id: scope.account,
		name: deviceId,
		status: "active",
		registered_at: 1,
		last_seen_at: 1_000,
		auth_epoch: 1,
		identity: identity(20),
		...patch,
	};
}
const zeroed = (buffers: Uint8Array[]) =>
	buffers.every((bytes) => bytes.every((byte) => byte === 0));

test("one unlock serves every tool on this device; a second tool never prompts again", async () => {
	const h = harness();
	const steps: string[] = [];
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "locked",
		role: "owner",
		grantId: "owner",
		canSign: false,
	});
	await h.keys.unlock("dev", PASSWORD, {
		onProgress: (step: UnlockStep) =>
			steps.push(
				`${step.id}:${step.state}${step.detail ? `:${step.detail.code}` : ""}`,
			),
	});
	expect(steps).toEqual([
		"unlocking_keys:active",
		"unlocking_keys:done",
		"rotating_endpoint:skipped",
		"checking_identity:active",
		"checking_identity:done",
		"saving_backup:skipped:not_requested",
		"reading_encrypted_status:active",
		"reading_encrypted_status:done",
	]);
	const snapshot = h.keys.snapshot("dev");
	expect(snapshot).toMatchObject({
		state: "unlocked",
		unlockedAt: 1_000_000,
		lastUsedAt: 1_000_000,
		idleLocksAt: 1_000_000 + IDLE_LOCK_MS,
		keepUnlocked: false,
	});
	expect(h.keys.snapshot("dev")).toBe(snapshot);
	expect(h.keys.controller("dev")).toBe(h.opened[0].controller);
	expect(h.keys.receipt("dev")?.device_id).toBe("dev");
	expect(h.keys.vault("dev")?.deviceId).toBe("dev");
	expect(h.pinned).toEqual(["dev"]);
	expect(h.fleetRefreshed).toEqual(["dev"]);
	expect(await queryDeviceLock(scope, "dev")).toBe("held_here");
	expect(zeroed(h.passwords)).toBe(true);

	await h.keys.unlock("dev", "anything", { connectLive: true });
	expect(h.calls.filter((call) => call.startsWith("unlock"))).toEqual([
		"unlock dev",
	]);
	expect(h.live.acquired).toEqual(["dev:view"]);
	expect(h.keys.list().map((row) => row.deviceId)).toEqual(["dev"]);

	h.keys.lock("dev");
	await flush();
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "locked",
		lockedSummary: { readAt: 42, services: [] },
	});
	expect(h.keys.controller("dev")).toBeUndefined();
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });
	expect(h.live.released).toBe(1);
	expect(h.live.closed).toEqual(["dev"]);
	expect(await queryDeviceLock(scope, "dev")).toBe("free");
});

test("a wrong password gives a plain code, zeroes the password and keeps no lock", async () => {
	const h = harness();
	const failed: string[] = [];
	const error = await h.keys
		.unlock("dev", "wrong password", {
			onProgress: (step) => {
				if (step.state === "failed")
					failed.push(`${step.id}:${step.detail?.code}`);
			},
		})
		.catch((cause: unknown) => cause);
	expect((error as KeySessionError).keyError).toEqual({
		code: "wrong_password",
	});
	expect(failed).toEqual(["unlocking_keys:wrong_password"]);
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "locked",
		lastError: { code: "wrong_password" },
	});
	expect(zeroed(h.passwords)).toBe(true);
	await flush();
	expect(await queryDeviceLock(scope, "dev")).toBe("free");
	await h.keys.unlock("dev", PASSWORD);
	expect(h.keys.snapshot("dev").lastError).toBeUndefined();
	h.keys.lockAll();
});

test("the idle lock waits 30 minutes of no use, counts active operations and honours keep unlocked", async () => {
	const h = harness();
	await h.keys.unlock("dev", PASSWORD);
	h.advance(29 * 60_000);
	h.keys.touch("dev");
	expect(h.keys.snapshot("dev").idleLocksAt).toBe(
		1_000_000 + 29 * 60_000 + IDLE_LOCK_MS,
	);
	h.advance(29 * 60_000);
	expect(h.keys.snapshot("dev").state).toBe("unlocked");
	h.activity.push({
		id: "rollout",
		kind: "safe_update",
		target: { deviceId: "dev" },
		state: "active",
		label: { code: "safe_update" },
		actions: [],
		startedAt: 0,
		updatedAt: 0,
		startedBy: "you",
	});
	h.advance(2 * 60_000);
	expect(h.keys.snapshot("dev").state).toBe("unlocked");
	h.activity[0].state = "done";
	h.advance(IDLE_LOCK_MS);
	expect(h.keys.snapshot("dev").state).toBe("locked");
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });

	await h.keys.unlock("dev", PASSWORD, { keepUnlocked: true });
	expect(h.keys.snapshot("dev")).toMatchObject({
		keepUnlocked: true,
		idleLocksAt: undefined,
	});
	h.advance(5 * IDLE_LOCK_MS);
	expect(h.keys.snapshot("dev").state).toBe("unlocked");
	h.keys.setKeepUnlocked("dev", false);
	h.advance(IDLE_LOCK_MS);
	expect(h.keys.snapshot("dev").state).toBe("locked");
	expect(h.pendingTimers).toBe(0);
});

test("held elsewhere shows in pre-flight, Use here steals, and the losing window locks itself", async () => {
	const h = harness();
	h.setRows([row("dev")]);
	const foreignLost = otherWindow(locks, "dev");
	const preflight = await h.keys.preflight("dev");
	expect(preflight.rows.find((row) => row.id === "D6")?.status).toBe("fail");
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "held_elsewhere",
		lastError: { code: "held_elsewhere" },
	});
	await expect(h.keys.unlock("dev", PASSWORD)).rejects.toBeInstanceOf(
		KeySessionError,
	);
	expect(h.calls.some((call) => call.startsWith("unlock"))).toBe(false);

	await h.keys.takeOver("dev");
	await flush();
	expect((foreignLost() as DOMException).name).toBe("AbortError");
	expect(h.keys.snapshot("dev").state).toBe("locked");
	const requests = locks.requests;
	await h.keys.unlock("dev", PASSWORD);
	expect(locks.requests).toBe(requests);
	expect(h.keys.snapshot("dev").state).toBe("unlocked");

	otherWindow(locks, "dev");
	await flush();
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "held_elsewhere",
		lastError: { code: "held_elsewhere" },
	});
	expect(h.keys.controller("dev")).toBeUndefined();
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });
	locks.held.clear();
	expect(
		(await h.keys.preflight("dev")).rows.find((row) => row.id === "D6")?.status,
	).toBe("pass");
	expect(h.keys.snapshot("dev").state).toBe("locked");
});

test("a browser without Web Locks cannot hold device keys", async () => {
	const h = harness();
	Object.defineProperty(globalThis, "navigator", {
		configurable: true,
		value: {},
	});
	const error = await h.keys.unlock("dev", PASSWORD).catch((cause) => cause);
	expect((error as KeySessionError).keyError).toEqual({
		code: "lock_unsupported",
	});
	expect(h.keys.snapshot("dev").lastError).toEqual({
		code: "lock_unsupported",
	});
	h.setRows([row("dev")]);
	const preflight = await h.keys.preflight("dev");
	expect(preflight.rows.find((row) => row.id === "D6")?.status).toBe("fail");
	expect(preflight.passwordEnabled).toBe(false);
});

test("late decrypted results after lock are discarded and the controller is closed and freed", async () => {
	const h = harness();
	let answer!: (value: DeviceReceipt) => void;
	h.setIdentityFetch(
		() =>
			new Promise<DeviceReceipt>((resolve) => {
				answer = resolve;
			}),
	);
	const pending = h.keys.unlock("dev", PASSWORD).catch((cause) => cause);
	await flush();
	expect(h.keys.snapshot("dev").state).toBe("unlocking");
	h.keys.lock("dev");
	answer(receipt("dev"));
	expect(((await pending) as DOMException).name).toBe("AbortError");
	expect(h.keys.snapshot("dev").state).toBe("locked");
	expect(h.keys.controller("dev")).toBeUndefined();
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });
	expect(h.pinned).toEqual([]);
	await flush();
	expect(await queryDeviceLock(scope, "dev")).toBe("free");

	const aborted = new AbortController();
	h.setIdentityFetch(async (deviceId) => {
		aborted.abort();
		return receipt(deviceId);
	});
	await expect(
		h.keys.unlock("dev", PASSWORD, { signal: aborted.signal }),
	).rejects.toThrow();
	expect(h.opened[1].state).toEqual({ closed: 1, freed: 1 });
	expect(h.keys.snapshot("dev")).toMatchObject({ state: "locked" });
	expect(h.keys.snapshot("dev").lastError).toBeUndefined();
});

test("a hub failure at the identity read stays on the session and the unlock rejects with the hub's error", async () => {
	const h = harness();
	const refused = Object.assign(new Error("[HTTP_503] hub down"), {
		status: 503,
	});
	h.setIdentityFetch(async () => {
		throw refused;
	});
	const failed: string[] = [];
	const error = await h.keys
		.unlock("dev", PASSWORD, {
			onProgress: (step) => {
				if (step.state === "failed")
					failed.push(`${step.id}:${step.detail?.code}`);
			},
		})
		.catch((cause: unknown) => cause);
	expect(error).toBe(refused);
	expect(failed).toEqual(["checking_identity:http_error"]);
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "locked",
		lastError: { code: "hub", status: 503 },
	});
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });

	h.setIdentityFetch(async () => {
		throw new TypeError("Failed to fetch");
	});
	await h.keys.unlock("dev", PASSWORD).catch(() => undefined);
	expect(h.keys.snapshot("dev").lastError).toEqual({ code: "hub" });

	h.setIdentityFetch(async (deviceId) => receipt(deviceId));
	await h.keys.unlock("dev", PASSWORD);
	expect(h.keys.snapshot("dev")).toMatchObject({ state: "unlocked" });
	expect(h.keys.snapshot("dev").lastError).toBeUndefined();
	h.keys.lockAll();
});

test("a restored vault gets a fresh endpoint at first unlock with the re-approval note", async () => {
	const h = harness([vault("dev", { restored: true })]);
	expect(h.keys.snapshot("dev").restoredNeedsFreshEndpoint).toBe(true);
	const steps: string[] = [];
	await h.keys.unlock("dev", PASSWORD, {
		onProgress: (step) =>
			steps.push(
				`${step.id}:${step.state}${step.detail ? `:${step.detail.code}` : ""}`,
			),
	});
	expect(steps.slice(0, 4)).toEqual([
		"unlocking_keys:active",
		"unlocking_keys:done",
		"rotating_endpoint:active",
		"rotating_endpoint:done:fresh_endpoint",
	]);
	expect(h.replaced).toHaveLength(1);
	expect(h.replaced[0]).toMatchObject({
		requiresFreshEndpoint: false,
		controllerPublic: { endpoint_id: "fresh-endpoint" },
	});
	expect(h.keys.vault("dev")).toBe(h.replaced[0]);
	expect(h.keys.snapshot("dev").restoredNeedsFreshEndpoint).toBe(false);
	expect(zeroed(h.passwords)).toBe(true);
	h.keys.lockAll();
});

test("an identity mismatch is a hard block before and after the password", async () => {
	const h = harness();
	h.setRows([row("dev", { identity: identity(30) })]);
	h.setIdentityCheck("mismatch");
	const preflight = await h.keys.preflight("dev");
	const d7 = preflight.rows.find((row) => row.id === "D7");
	expect(d7?.status).toBe("block");
	expect(d7?.copy.params?.since).toBe(500);
	expect(preflight.passwordEnabled).toBe(false);
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "blocked",
		lastError: {
			code: "identity_mismatch",
			pinnedAt: 500,
			fingerprint: identityFingerprint(identity(20)),
			reported: identityFingerprint(identity(30)),
		},
	});
	const error = await h.keys.unlock("dev", PASSWORD).catch((cause) => cause);
	expect((error as KeySessionError).keyError.code).toBe("identity_mismatch");
	expect(h.keys.snapshot("dev").state).toBe("blocked");
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });
	expect(h.pinned).toEqual([]);
	await flush();
	expect(await queryDeviceLock(scope, "dev")).toBe("free");

	h.setIdentityCheck("match");
	h.setRows([row("dev")]);
	expect((await h.keys.preflight("dev")).passwordEnabled).toBe(true);
	expect(h.keys.snapshot("dev").state).toBe("locked");
});

test("unlocking several devices tries each vault in turn and reports every outcome", async () => {
	const h = harness([
		vault("a"),
		vault("b", { secret: "another password" }),
		vault("c"),
	]);
	locks.held.set(lockName("c"), { reject: () => undefined });
	const outcomes: string[] = [];
	await h.keys.unlockMany(["a", "b", "c", "missing"], PASSWORD, (id, outcome) =>
		outcomes.push(`${id}:${outcome}`),
	);
	expect(outcomes).toEqual([
		"a:unlocked",
		"b:wrong_password",
		"c:held_elsewhere",
		"missing:no_vault",
	]);
	expect(h.calls.filter((call) => call.startsWith("unlock"))).toEqual([
		"unlock a",
		"unlock b",
	]);
	const stop = new AbortController();
	const later: string[] = [];
	await h.keys.unlockMany(
		["a", "b"],
		PASSWORD,
		(id, outcome) => {
			later.push(`${id}:${outcome}`);
			stop.abort();
		},
		stop.signal,
	);
	expect(later).toEqual(["a:unlocked"]);
	locks.held.clear();
	h.keys.lockAll();
});

test("the session holds the vault that storage has once the device lock is granted", async () => {
	const h = harness([vault("dev"), vault("gone")]);
	const rewrapped = { ...vault("dev") };
	const request = locks.request;
	locks.request = (name, options, run) => {
		if (name === lockName("dev")) h.stored.set("dev", rewrapped);
		else h.stored.delete("gone");
		return request(name, options, run);
	};
	await h.keys.unlock("dev", PASSWORD);
	expect(h.keys.vault("dev")).toBe(rewrapped);

	const missing = await h.keys.unlock("gone", PASSWORD).catch((cause) => cause);
	expect((missing as KeySessionError).keyError).toEqual({ code: "no_vault" });
	await flush();
	expect(await queryDeviceLock(scope, "gone")).toBe("free");
	h.keys.lockAll();
});

test("vault leases run under the session lock and fall back to a transient lock", async () => {
	const h = harness([vault("dev"), vault("other")]);
	await h.keys.unlock("dev", PASSWORD);
	const requests = locks.requests;
	const order: string[] = [];
	const next = { ...vault("dev"), controllerVault: new Uint8Array(80).fill(5) };
	const first = h.keys.withVaultLease("dev", async (lease) => {
		order.push("first start");
		await flush();
		lease.replace(next);
		order.push("first end");
		return "first";
	});
	const second = h.keys.withVaultLease("dev", async (lease) => {
		order.push(`second sees ${lease.vault === next}`);
		return "second";
	});
	expect(await Promise.all([first, second])).toEqual(["first", "second"]);
	expect(order).toEqual(["first start", "first end", "second sees true"]);
	expect(locks.requests).toBe(requests);
	expect(h.keys.vault("dev")).toBe(next);
	expect(h.keys.controller("dev")).toBeDefined();

	expect(
		await h.keys.withVaultLease("other", async (lease) => lease.vault.deviceId),
	).toBe("other");
	expect(locks.requests).toBe(requests + 1);
	await flush();
	expect(await queryDeviceLock(scope, "other")).toBe("free");
	locks.held.set(lockName("other"), { reject: () => undefined });
	const held = await h.keys
		.withVaultLease("other", async () => "never")
		.catch((cause) => cause);
	expect((held as KeySessionError).keyError).toEqual({
		code: "held_elsewhere",
	});
	const missing = await h.keys
		.withVaultLease("missing", async () => "never")
		.catch((cause) => cause);
	expect((missing as KeySessionError).keyError).toEqual({ code: "no_vault" });
	locks.held.clear();
	h.keys.lockAll();
});

test("the account backup is sealed with the typed password under the lease and tracked", async () => {
	const h = harness([vault("dev"), vault("full")]);
	const steps: string[] = [];
	const onProgress = (step: UnlockStep) => {
		if (step.id === "saving_backup")
			steps.push(`${step.state}${step.detail ? `:${step.detail.code}` : ""}`);
	};
	await h.keys.unlock("dev", PASSWORD, { backupToAccount: true, onProgress });
	expect(h.backups).toEqual([
		{ password: PASSWORD, leaseVault: h.keys.vault("dev") },
	]);
	expect(h.activity[0]).toMatchObject({
		kind: "account_backup",
		target: { deviceId: "dev" },
		state: "done",
	});
	h.setBackupError(
		new ApiResponseError({ status: 429, code: "LIMIT", message: "Too many" }),
	);
	await h.keys.unlock("full", PASSWORD, { backupToAccount: true, onProgress });
	expect(steps).toEqual([
		"active",
		"done:backup_saved",
		"active",
		"failed:backup_limit",
	]);
	expect(h.keys.snapshot("full").state).toBe("unlocked");
	expect(h.activity[1].state).toBe("failed");
	h.keys.lockAll();
});

const policy = { version: 2 } as unknown as ManagementPolicy;
const telemetryRoster = { scope: "device" } as unknown as TelemetryRoster;
const archiveRoster = { kind: "logs" } as unknown as ArchiveRoster;

test("a crypto bundle without the held key asks for the password at every owner signature", async () => {
	const h = harness([vault("dev"), vault("shared", { owner: false })], {
		heldSigner: false,
	});
	expect(h.keys.signer("dev")).toBeUndefined();
	await h.keys.unlock("dev", PASSWORD);
	await h.keys.unlock("shared", PASSWORD);
	expect(h.keys.signer("shared")).toBeUndefined();
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "unlocked",
		canSign: false,
	});
	const signer = h.keys.signer("dev");
	await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	expect(await signer?.signPolicy(policy, PASSWORD)).toBe("signed-policy");
	expect(await signer?.signPolicy(policy, PASSWORD)).toBe("signed-policy");
	const invitation = vault("dev").invitationVault as Uint8Array;
	expect(h.signed).toEqual([
		{ policy, invitation },
		{ policy, invitation },
	]);
	expect(h.keys.snapshot("dev").canSign).toBe(false);
	expect(zeroed(h.passwords)).toBe(true);
	h.keys.lock("dev");
	await expect(signer?.signPolicy(policy, PASSWORD)).rejects.toBeInstanceOf(
		KeySessionError,
	);
	h.keys.lockAll();
});

test("an owner unlock holds the invitation key, so access changes need no password until lock", async () => {
	const h = harness([vault("dev"), vault("shared", { owner: false })]);
	await h.keys.unlock("dev", PASSWORD);
	await h.keys.unlock("shared", PASSWORD);
	expect(h.keys.snapshot("dev")).toMatchObject({
		role: "owner",
		canSign: true,
	});
	expect(h.keys.snapshot("shared")).toMatchObject({
		state: "unlocked",
		role: "shared",
		canSign: false,
	});
	expect(h.opened.map(({ signer }) => signer)).toEqual([
		{ attached: 1, detached: 0, holding: true },
		{ attached: 0, detached: 0, holding: false },
	]);
	expect(h.keys.signer("shared")).toBeUndefined();

	const signer = h.keys.signer("dev");
	expect(await signer?.signPolicy(policy)).toBe("held-policy");
	expect(await signer?.signTelemetryRoster(telemetryRoster)).toBe(
		"held-telemetry",
	);
	expect(await signer?.signArchiveRoster(archiveRoster)).toBe("held-archive");
	expect(await signer?.signPolicy(policy, "not needed")).toBe("held-policy");
	expect(h.signed).toEqual([]);
	expect(h.opened[0].signer.attached).toBe(1);
	expect(zeroed(h.passwords)).toBe(true);

	h.advance(IDLE_LOCK_MS - 60_000);
	await signer?.signPolicy(policy);
	expect(h.keys.snapshot("dev").lastUsedAt).toBe(
		1_000_000 + IDLE_LOCK_MS - 60_000,
	);

	h.keys.lock("dev");
	expect(h.opened[0].signer).toEqual({
		attached: 1,
		detached: 1,
		holding: false,
	});
	expect(h.opened[0].state).toEqual({ closed: 1, freed: 1 });
	expect(h.keys.snapshot("dev").canSign).toBe(false);
	await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
		KeySessionError,
	);
	h.keys.lockAll();
});

test("with 'ask for my password again' on, nothing is held and every signature needs the password", async () => {
	const h = harness();
	expect(h.keys.askPasswordForAccessChanges()).toBe(false);
	h.keys.setAskPasswordForAccessChanges(true);
	expect(h.keys.askPasswordForAccessChanges()).toBe(true);
	await h.keys.unlock("dev", PASSWORD);
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "unlocked",
		canSign: false,
	});
	const signer = h.keys.signer("dev");
	await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	expect(await signer?.signPolicy(policy, PASSWORD)).toBe("signed-policy");
	expect(await signer?.signTelemetryRoster(telemetryRoster, PASSWORD)).toBe(
		"signed-telemetry",
	);
	expect(await signer?.signArchiveRoster(archiveRoster, PASSWORD)).toBe(
		"signed-archive",
	);
	expect(h.signed.map((entry) => entry.policy)).toEqual([
		policy,
		telemetryRoster,
		archiveRoster,
	]);
	await expect(signer?.signPolicy(policy, "wrong password")).rejects.toThrow(
		WRONG_PASSWORD,
	);
	expect(h.opened[0].signer).toEqual({
		attached: 0,
		detached: 0,
		holding: false,
	});
	expect(h.keys.snapshot("dev").canSign).toBe(false);
	expect(zeroed(h.passwords)).toBe(true);

	h.keys.setAskPasswordForAccessChanges(false);
	expect(h.keys.snapshot("dev").canSign).toBe(false);
	await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	expect(await signer?.signPolicy(policy, PASSWORD)).toBe("held-policy");
	expect(h.keys.snapshot("dev").canSign).toBe(true);
	expect(await signer?.signArchiveRoster(archiveRoster)).toBe("held-archive");
	expect(h.signed).toHaveLength(3);
	expect(zeroed(h.passwords)).toBe(true);
	h.keys.lockAll();
});

test("turning 'ask for my password again' on drops every held key at once, from this or another window", async () => {
	const h = harness([vault("a"), vault("b")]);
	await h.keys.unlock("a", PASSWORD);
	await h.keys.unlock("b", PASSWORD);
	let notified = 0;
	const stop = h.keys.subscribe(() => notified++);
	h.keys.setAskPasswordForAccessChanges(true);
	expect(notified).toBe(1);
	expect(h.keys.list().map((row) => `${row.state}:${row.canSign}`)).toEqual([
		"unlocked:false",
		"unlocked:false",
	]);
	expect(h.opened.map(({ signer }) => signer)).toEqual([
		{ attached: 1, detached: 1, holding: false },
		{ attached: 1, detached: 1, holding: false },
	]);
	await expect(h.keys.signer("a")?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	stop();

	h.keys.setAskPasswordForAccessChanges(false);
	h.keys.lock("a");
	await h.keys.unlock("a", PASSWORD);
	expect(h.keys.snapshot("a").canSign).toBe(true);
	h.setAskStored(true);
	expect(h.keys.askPasswordForAccessChanges()).toBe(true);
	await expect(h.keys.signer("a")?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	expect(h.keys.snapshot("a").canSign).toBe(false);
	expect(h.opened[2].signer).toEqual({
		attached: 1,
		detached: 1,
		holding: false,
	});
	expect(await h.keys.signer("a")?.signPolicy(policy, PASSWORD)).toBe(
		"signed-policy",
	);
	expect(h.opened[2].signer.attached).toBe(1);

	h.setAskStored(false);
	h.blockStorage();
	h.keys.setAskPasswordForAccessChanges(true);
	expect(h.keys.askPasswordForAccessChanges()).toBe(true);
	h.keys.lock("b");
	await h.keys.unlock("b", PASSWORD);
	expect(h.keys.snapshot("b").canSign).toBe(false);
	expect(h.opened[3].signer.attached).toBe(0);
	h.keys.lockAll();
});

test("a failed attach keeps the unlock, and an unlock that ends after attaching clears the held key", async () => {
	const h = harness();
	h.onAttach(() => {
		throw new Error("damaged invitation vault");
	});
	await h.keys.unlock("dev", PASSWORD);
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "unlocked",
		canSign: false,
	});
	const signer = h.keys.signer("dev");
	await expect(signer?.signPolicy(policy)).rejects.toBeInstanceOf(
		OwnerPasswordRequiredError,
	);
	await expect(signer?.signPolicy(policy, PASSWORD)).rejects.toThrow(
		"damaged invitation vault",
	);
	h.onAttach(undefined);
	await expect(signer?.signPolicy(policy, "wrong password")).rejects.toThrow(
		WRONG_PASSWORD,
	);
	expect(h.keys.snapshot("dev").canSign).toBe(false);
	expect(h.signed).toEqual([]);
	expect(await signer?.signPolicy(policy, PASSWORD)).toBe("held-policy");
	expect(h.keys.snapshot("dev").canSign).toBe(true);
	expect(zeroed(h.passwords)).toBe(true);
	h.keys.lock("dev");

	h.onAttach(() => queueMicrotask(() => h.keys.lock("dev")));
	const ended = await h.keys.unlock("dev", PASSWORD).catch((cause) => cause);
	expect((ended as DOMException).name).toBe("AbortError");
	expect(h.opened[1].signer).toEqual({
		attached: 1,
		detached: 1,
		holding: false,
	});
	expect(h.opened[1].state).toEqual({ closed: 1, freed: 1 });
	expect(h.keys.snapshot("dev")).toMatchObject({
		state: "locked",
		canSign: false,
	});
	expect(h.keys.controller("dev")).toBeUndefined();
	await flush();
	expect(await queryDeviceLock(scope, "dev")).toBe("free");
});

test("the 'ask for my password again' choice is stored per account on this computer and is off by default", () => {
	const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
	const install = (value: unknown) =>
		Object.defineProperty(globalThis, "localStorage", {
			configurable: true,
			value,
		});
	const entries = new Map<string, string>();
	try {
		install({
			getItem: (key: string) => entries.get(key) ?? null,
			setItem: (key: string, value: string) => void entries.set(key, value),
			removeItem: (key: string) => void entries.delete(key),
		});
		expect(readAskPasswordForAccessChanges(scope)).toBe(false);
		expect(writeAskPasswordForAccessChanges(scope, true)).toBe(true);
		expect(readAskPasswordForAccessChanges(scope)).toBe(true);
		expect(
			readAskPasswordForAccessChanges({ ...scope, account: "colleague" }),
		).toBe(false);
		expect([...entries.keys()]).toEqual([
			`flow-like/devices/ask-password-for-access-changes/${accountStorageKey(scope)}`,
		]);
		expect(writeAskPasswordForAccessChanges(scope, false)).toBe(true);
		expect(entries.size).toBe(0);
		expect(readAskPasswordForAccessChanges(scope)).toBe(false);

		const refuse = () => {
			throw new DOMException("Blocked", "SecurityError");
		};
		install({ getItem: refuse, setItem: refuse, removeItem: refuse });
		expect(readAskPasswordForAccessChanges(scope)).toBe(false);
		expect(writeAskPasswordForAccessChanges(scope, true)).toBe(false);
		install(undefined);
		expect(readAskPasswordForAccessChanges(scope)).toBe(false);
		expect(writeAskPasswordForAccessChanges(scope, true)).toBe(false);
	} finally {
		if (original) Object.defineProperty(globalThis, "localStorage", original);
		else Reflect.deleteProperty(globalThis, "localStorage");
	}
});

test("page hide and dispose lock every session", async () => {
	const h = harness([vault("a"), vault("b")]);
	await h.keys.unlock("a", PASSWORD);
	await h.keys.unlock("b", PASSWORD);
	h.pageHide?.();
	expect(h.keys.list().map((row) => row.state)).toEqual(["locked", "locked"]);
	await h.keys.unlock("a", PASSWORD);
	h.keys.dispose();
	expect(h.keys.snapshot("a").state).toBe("locked");
	expect(h.pageHide).toBeUndefined();
	expect(h.opened.every(({ state }) => state.freed === 1)).toBe(true);
});

test("pre-flight reads the hub device list by default and marks revoked keys stale", async () => {
	const h = harness([vault("dev"), vault("gone")], { platform: "web" });
	h.setRows([row("dev"), row("gone", { status: "revoked", revoked_at: 900 })]);
	const ready = await h.keys.preflight("dev");
	expect(ready.passwordEnabled).toBe(true);
	expect(ready.rows.map((row) => `${row.id}:${row.status}`)).toContain(
		"D7:pass",
	);
	expect(h.calls).toContain("GET devices");
	const revoked = await h.keys.preflight("gone");
	expect(revoked.passwordEnabled).toBe(false);
	expect(h.keys.snapshot("gone").state).toBe("stale");
	expect(h.keys.snapshot("missing").state).toBe("none");

	const facts: KeyPreflightFacts = {
		now: 2_000,
		hub: { state: "off" },
		auth: { signedIn: true, tokenScopeAll: true },
		relationship: "owner",
		clock: {},
	};
	const wired = createKeySessionManager(
		{
			api: {} as IApiState,
			profile: {} as IProfile,
			scope,
			queryClient: {} as QueryClient,
			crypto: async () => ({}) as DeviceCrypto,
			platform: "desktop",
		},
		{ ...h.ports, facts: async () => facts },
		{ onPageHide: () => () => undefined },
	);
	const off = await wired.preflight("dev");
	expect(off.rows.find((row) => row.id === "D1")?.status).toBe("block");
	expect(off.passwordEnabled).toBe(false);
});
