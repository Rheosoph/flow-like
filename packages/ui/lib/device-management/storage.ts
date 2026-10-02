import { isTauri } from "../platform";
import type { LocalCertificateAuthority } from "./certificate-authority";
import type {
	AccountRecoveryWrite,
	BrowserMlsEndpoint,
	Checkpoint,
	ControllerPublic,
	DeviceReceipt,
	Ed25519PublicKey,
	FleetLocalState,
	InventoryScope,
	OnboardingManifest,
	ProtectedSnapshot,
} from "./types";

export interface DeviceAccountScope {
	issuer: string;
	account: string;
	apiOrigin: string;
	profileId: string;
}
export interface LocalDeviceVault {
	deviceId: string;
	controllerPublic: ControllerPublic;
	controllerVault: Uint8Array;
	invitationVault?: Uint8Array;
	manifestJws: string;
	grantId: string;
	ownerControllerKey?: Ed25519PublicKey;
	requiresFreshEndpoint?: boolean;
}
interface SnapshotRow {
	snapshot: ProtectedSnapshot;
	checkpoint: Checkpoint;
}

export function accountStorageKey(scope: DeviceAccountScope): string {
	if (!scope.account || !scope.apiOrigin)
		throw new Error(
			"Device storage requires an authenticated account and hub.",
		);
	return JSON.stringify([
		scope.issuer,
		scope.account,
		scope.apiOrigin,
		scope.profileId,
	]);
}
function itemKey(scope: DeviceAccountScope, ...ids: string[]): string {
	return JSON.stringify([accountStorageKey(scope), ...ids]);
}

export function deviceApiBase(scope: DeviceAccountScope): string {
	return `${scope.apiOrigin.replace(/\/$/u, "")}/api/v1`;
}

export function isPublicKey(value: unknown): value is Ed25519PublicKey {
	const key = value as Ed25519PublicKey | undefined;
	return (
		!!key &&
		typeof key === "object" &&
		key.kty === "OKP" &&
		key.crv === "Ed25519" &&
		typeof key.x === "string" &&
		/^[A-Za-z0-9_-]{43}$/u.test(key.x)
	);
}

/** Reads a manifest payload without verifying it; callers verify its signature before trusting it. */
export function unsignedManifest(compact: string): OnboardingManifest {
	let value: unknown;
	try {
		const part = compact.split(".")[1] ?? "";
		const bytes = Uint8Array.from(
			atob(part.replaceAll("-", "+").replaceAll("_", "/")),
			(c) => c.charCodeAt(0),
		);
		value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
	} catch {
		value = undefined;
	}
	if (!value || typeof value !== "object" || Array.isArray(value))
		throw new Error("The device vault's onboarding manifest is unreadable.");
	return value as OnboardingManifest;
}

/** Owner vaults anchor on their own controller key; shared vaults on the owner's key only. */
export function assertVaultAuthority(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
	manifest: OnboardingManifest,
): void {
	const owner = vault.grantId === "owner";
	const local = vault.controllerPublic.controller_key;
	if (
		!/^[A-Za-z0-9_:.-]{1,128}$/u.test(vault.grantId) ||
		!isPublicKey(local) ||
		(owner
			? vault.ownerControllerKey !== undefined
			: !isPublicKey(vault.ownerControllerKey) ||
				vault.invitationVault !== undefined)
	)
		throw new Error(
			`The vault for device ${vault.deviceId} mixes owner and shared-access authority (grant ${vault.grantId}).`,
		);
	const anchor = owner ? local : vault.ownerControllerKey;
	if (
		manifest.device_id !== vault.deviceId ||
		manifest.api_base_url !== deviceApiBase(scope) ||
		manifest.controller_key?.x !== anchor?.x ||
		(owner
			? manifest.owner_id !== scope.account
			: manifest.controller_key.x === local.x)
	)
		throw new Error(
			`The onboarding manifest for device ${vault.deviceId} does not match this vault's ${owner ? "owner" : "shared"} authority, account or hub.`,
		);
}

function vaultIdentity(vault: LocalDeviceVault): string {
	return JSON.stringify([
		vault.deviceId,
		vault.controllerPublic,
		vault.manifestJws,
		vault.grantId,
		vault.ownerControllerKey,
		Boolean(vault.requiresFreshEndpoint),
	]);
}
function sameBytes(
	left: Uint8Array | undefined,
	right: Uint8Array | undefined,
): boolean {
	return left === undefined || right === undefined
		? left === right
		: left.length === right.length &&
				left.every((byte, index) => byte === right[index]);
}

/** Both password envelopes move together; endpoint snapshots retain their storage key. */
export function replaceRewrappedVault(
	scope: DeviceAccountScope,
	previous: LocalDeviceVault,
	next: LocalDeviceVault,
): Promise<void> {
	if (
		vaultIdentity(previous) !== vaultIdentity(next) ||
		Boolean(previous.invitationVault) !== Boolean(next.invitationVault) ||
		![
			next.controllerVault,
			...(next.invitationVault ? [next.invitationVault] : []),
		].every(
			(bytes) =>
				bytes instanceof Uint8Array &&
				bytes.length >= 64 &&
				bytes.length <= 65_600,
		) ||
		sameBytes(previous.controllerVault, next.controllerVault) ||
		(previous.invitationVault &&
			sameBytes(previous.invitationVault, next.invitationVault))
	)
		return Promise.reject(
			new Error(
				"Password changes must preserve the device's existing identities and replace both encrypted vaults.",
			),
		);
	return transact(
		"vaults",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, previous.deviceId);
			const request = store.get(key);
			request.onsuccess = () => {
				const current = request.result as LocalDeviceVault | undefined;
				if (
					!current ||
					vaultIdentity(current) !== vaultIdentity(previous) ||
					!sameBytes(current.controllerVault, previous.controllerVault) ||
					!sameBytes(current.invitationVault, previous.invitationVault)
				) {
					fail(
						new Error(
							"The local device vault changed. Close and reopen management before changing its password.",
						),
					);
					return;
				}
				store.put(next, key);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

export function encryptedControllerBackup(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
): Blob {
	return new Blob(
		[
			JSON.stringify({
				version: 1,
				apiOrigin: scope.apiOrigin,
				deviceId: vault.deviceId,
				controllerPublic: vault.controllerPublic,
				controllerVault: Array.from(vault.controllerVault),
				invitationVault: vault.invitationVault
					? Array.from(vault.invitationVault)
					: undefined,
				manifestJws: vault.manifestJws,
				grantId: vault.grantId,
				ownerControllerKey: vault.ownerControllerKey,
			}),
		],
		{ type: "application/json" },
	);
}

async function database(): Promise<IDBDatabase> {
	if (!globalThis.indexedDB)
		throw new Error("Encrypted device storage is unavailable in this browser.");
	return new Promise((resolve, reject) => {
		const request = indexedDB.open("flow-like-device-management", 4);
		request.onupgradeneeded = () => {
			for (const name of [
				"vaults",
				"snapshots",
				"recovery",
				"fleet",
				"authorities",
			])
				if (!request.result.objectStoreNames.contains(name))
					request.result.createObjectStore(name);
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(new Error("Encrypted device storage could not be opened."));
		request.onblocked = () =>
			reject(new Error("Close another app tab before opening device storage."));
	});
}

async function transact<T>(
	storeName: string,
	mode: IDBTransactionMode,
	action: (
		store: IDBObjectStore,
		set: (value: T) => void,
		fail: (error: Error) => void,
	) => void,
	options?: IDBTransactionOptions,
): Promise<T> {
	return transactStores(
		storeName,
		mode,
		(transaction, set, fail) => {
			action(transaction.objectStore(storeName), set, fail);
		},
		options,
	);
}

async function transactStores<T>(
	storeNames: string | string[],
	mode: IDBTransactionMode,
	action: (
		transaction: IDBTransaction,
		set: (value: T) => void,
		fail: (error: Error) => void,
	) => void,
	options?: IDBTransactionOptions,
): Promise<T> {
	const db = await database();
	try {
		return await new Promise<T>((resolve, reject) => {
			const tx = db.transaction(storeNames, mode, options);
			let value: T;
			let failure: Error | undefined;
			tx.oncomplete = () => resolve(value);
			tx.onerror = () =>
				reject(failure ?? new Error("Encrypted device storage failed."));
			tx.onabort = () =>
				reject(
					failure ?? new Error("Encrypted device storage was not committed."),
				);
			try {
				action(
					tx,
					(next) => {
						value = next;
					},
					(error) => {
						failure = error;
						tx.abort();
					},
				);
			} catch (error) {
				failure =
					error instanceof Error
						? error
						: new Error("Encrypted storage failed.");
				tx.abort();
			}
		});
	} finally {
		db.close();
	}
}

export function readDeviceVault(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<LocalDeviceVault | undefined> {
	return transact("vaults", "readonly", (store, set) => {
		const request = store.get(itemKey(scope, deviceId));
		request.onsuccess = () => set(request.result);
	});
}

export function addDeviceVault(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
): Promise<void> {
	if (
		vault.controllerPublic.device_id !== vault.deviceId ||
		vault.controllerVault.length < 64 ||
		!vault.manifestJws
	)
		return Promise.reject(new Error("Invalid encrypted device vault."));
	return transact("vaults", "readwrite", (store, set, fail) => {
		const request = store.add(vault, itemKey(scope, vault.deviceId));
		request.onerror = (event) => {
			if (request.error?.name !== "ConstraintError") return;
			event.preventDefault();
			fail(new DeviceVaultExistsError(vault.deviceId));
		};
		set(undefined);
	});
}

export class DeviceVaultExistsError extends Error {
	constructor(readonly deviceId: string) {
		super(
			`This app already holds encrypted keys for device ${deviceId}. Open device management to use them instead of creating new ones.`,
		);
		this.name = "DeviceVaultExistsError";
	}
}

interface DeviceIdentityPin {
	identity: string;
	pinnedAt: number;
}
export interface DeviceIdentityPinRecord extends DeviceIdentityPin {
	enrollmentId: string;
}

function pinRecord(enrollmentId: string | undefined, value: unknown) {
	const pin = value as Partial<DeviceIdentityPin> | undefined;
	if (!enrollmentId || typeof pin?.identity !== "string") return undefined;
	return typeof pin.pinnedAt === "number"
		? { enrollmentId, identity: pin.identity, pinnedAt: pin.pinnedAt }
		: undefined;
}

/** The pins among the rows of `keys`, per device and newest first. */
function pinsByDevice(keys: string[], values: unknown[]) {
	const result = new Map<string, DeviceIdentityPinRecord[]>();
	for (const [index, key] of keys.entries()) {
		const [, deviceId, kind, enrollmentId] = keyParts(key);
		const pin = kind === "identity" && pinRecord(enrollmentId, values[index]);
		if (pin) result.set(deviceId, [...(result.get(deviceId) ?? []), pin]);
	}
	for (const pins of result.values())
		pins.sort((left, right) => right.pinnedAt - left.pinnedAt);
	return result;
}

/** The comparable form a pin stores; equal keys mean the same device identity. */
export function deviceIdentityKey(identity: DeviceReceipt["identity"]): string {
	return JSON.stringify([
		identity.auth_key.x,
		identity.telemetry_key.x,
		Array.from(identity.management_key),
	]);
}

export function pinnedDeviceIdentity(
	pin: Pick<DeviceIdentityPin, "identity">,
): DeviceReceipt["identity"] | undefined {
	try {
		const [auth, telemetry, management] = JSON.parse(pin.identity) as [
			unknown,
			unknown,
			unknown,
		];
		if (
			typeof auth !== "string" ||
			typeof telemetry !== "string" ||
			!Array.isArray(management) ||
			!management.every(
				(byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255,
			)
		)
			return undefined;
		const key = (x: string) => ({
			kty: "OKP" as const,
			crv: "Ed25519" as const,
			x,
		});
		return {
			auth_key: key(auth),
			telemetry_key: key(telemetry),
			management_key: management as number[],
		};
	} catch {
		return undefined;
	}
}

function prefixRange(...parts: string[]): IDBKeyRange {
	const prefix = `${JSON.stringify(parts).slice(0, -1)},`;
	return IDBKeyRange.bound(prefix, `${prefix}\uffff`);
}
function accountRange(scope: DeviceAccountScope): IDBKeyRange {
	return prefixRange(accountStorageKey(scope));
}
function deviceRange(scope: DeviceAccountScope, deviceId: string): IDBKeyRange {
	return prefixRange(accountStorageKey(scope), deviceId);
}

/** Reads keys and values of one range in a single transaction, in key order. */
function readRange<T>(
	storeName: string,
	range: IDBKeyRange,
): Promise<[string[], T[]]> {
	return transact(storeName, "readonly", (store, set) => {
		const keys = store.getAllKeys(range);
		const values = store.getAll(range);
		values.onsuccess = () =>
			set([keys.result as string[], values.result as T[]]);
	});
}
function keyParts(key: string): string[] {
	const parts = JSON.parse(key) as unknown;
	return Array.isArray(parts) && parts.every((part) => typeof part === "string")
		? parts
		: [];
}

/** Newest first. Pins are keyed by enrollment, so a device set up again has one pin per setup. */
export async function readDeviceIdentityPins(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<DeviceIdentityPinRecord[]> {
	const [keys, values] = await readRange<unknown>(
		"vaults",
		prefixRange(accountStorageKey(scope), deviceId, "identity"),
	);
	return pinsByDevice(keys, values).get(deviceId) ?? [];
}

/** Every pin of this scope in one read, per device and newest first. */
export async function listDeviceIdentityPins(
	scope: DeviceAccountScope,
): Promise<Map<string, DeviceIdentityPinRecord[]>> {
	const [keys, values] = await readRange<unknown>(
		"vaults",
		accountRange(scope),
	);
	return pinsByDevice(keys, values);
}

/** Without an enrollment id every pin of the device is forgotten. */
export function forgetDeviceIdentityPin(
	scope: DeviceAccountScope,
	deviceId: string,
	enrollmentId?: string,
): Promise<void> {
	return transact(
		"vaults",
		"readwrite",
		(store, set) => {
			store.delete(
				enrollmentId === undefined
					? prefixRange(accountStorageKey(scope), deviceId, "identity")
					: itemKey(scope, deviceId, "identity", enrollmentId),
			);
			set(undefined);
		},
		{ durability: "strict" },
	);
}

function isVaultRow(value: unknown): value is LocalDeviceVault {
	const row = value as LocalDeviceVault | undefined;
	return (
		!!row &&
		typeof row.deviceId === "string" &&
		row.controllerVault instanceof Uint8Array &&
		typeof row.grantId === "string"
	);
}

export async function listDeviceVaults(
	scope: DeviceAccountScope,
): Promise<LocalDeviceVault[]> {
	const [keys, values] = await readRange<unknown>(
		"vaults",
		accountRange(scope),
	);
	return values.filter(
		(value, index): value is LocalDeviceVault =>
			isVaultRow(value) && keys[index] === itemKey(scope, value.deviceId),
	);
}

const DEVICE_STORES = ["vaults", "recovery", "fleet", "snapshots"] as const;

/** Removes the vault, identity pins, backup watermark, fleet anchors and MLS snapshots of one device. */
export function deleteDeviceVault(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<void> {
	return transactStores(
		[...DEVICE_STORES],
		"readwrite",
		(transaction, set) => {
			for (const name of DEVICE_STORES) {
				const store = transaction.objectStore(name);
				store.delete(itemKey(scope, deviceId));
				store.delete(deviceRange(scope, deviceId));
			}
			set(undefined);
		},
		{ durability: "strict" },
	);
}

/** Call only with a verified receipt: its first-seen keys become the device's pinned identity. */
export function pinDeviceIdentity(
	scope: DeviceAccountScope,
	deviceId: string,
	receipt: DeviceReceipt,
): Promise<void> {
	if (receipt.device_id !== deviceId)
		return Promise.reject(
			new Error(
				`The device identity for ${receipt.device_id} cannot be pinned for device ${deviceId}.`,
			),
		);
	const identity = deviceIdentityKey(receipt.identity);
	return transact(
		"vaults",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, deviceId, "identity", receipt.enrollment_id);
			const read = store.get(key);
			read.onsuccess = () => {
				const pinned = read.result as DeviceIdentityPin | undefined;
				if (pinned && pinned.identity !== identity) {
					fail(
						new Error(
							`Device ${deviceId} presented keys that differ from the identity this app first verified. Management stays blocked because the hub or relay may be impersonating the device.`,
						),
					);
					return;
				}
				if (!pinned) store.put({ identity, pinnedAt: Date.now() }, key);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

export type DeviceStoragePersistence = "persisted" | "denied" | "unavailable";

export function deviceStorageWarning(
	persistence: DeviceStoragePersistence | undefined,
): string | undefined {
	return persistence && persistence !== "persisted"
		? "This browser has not granted persistent storage, so it may delete this app's encrypted device keys when space runs low or after a period without use. Keep a controller backup or an account backup to restore management."
		: undefined;
}

/** Passive status read: never calls `persist()`, so it never prompts. "denied" means not granted yet. */
export async function readStoragePersistence(): Promise<DeviceStoragePersistence> {
	if (isTauri()) return "persisted";
	const storage = globalThis.navigator?.storage;
	if (!storage?.persisted) return "unavailable";
	try {
		return (await storage.persisted()) ? "persisted" : "denied";
	} catch {
		return "unavailable";
	}
}

/** Browsers may evict unpersisted IndexedDB data, which holds the only copy of these keys. */
export async function requestPersistentDeviceStorage(): Promise<DeviceStoragePersistence> {
	if (isTauri()) return "persisted";
	const storage = globalThis.navigator?.storage;
	if (!storage?.persist) return "unavailable";
	try {
		if (await storage.persisted?.()) return "persisted";
		return (await storage.persist()) ? "persisted" : "denied";
	} catch {
		return "unavailable";
	}
}

export function addCertificateAuthority(
	scope: DeviceAccountScope,
	authority: LocalCertificateAuthority,
): Promise<void> {
	if (
		authority.public_bundle.account_binding !== accountStorageKey(scope) ||
		authority.vault.length < 64 ||
		authority.vault.length > 65_600
	)
		return Promise.reject(
			new Error("Invalid encrypted certificate authority for this account."),
		);
	return transact(
		"authorities",
		"readwrite",
		(store, set) => {
			store.add(
				{ public_bundle: authority.public_bundle, vault: authority.vault },
				itemKey(scope, authority.public_bundle.authority_id),
			);
			set(undefined);
		},
		{ durability: "strict" },
	);
}

export function readCertificateAuthorities(
	scope: DeviceAccountScope,
): Promise<LocalCertificateAuthority[]> {
	return transact("authorities", "readonly", (store, set) => {
		const request = store.getAll(accountRange(scope));
		request.onsuccess = () =>
			set(
				(request.result as LocalCertificateAuthority[]).filter(
					(row) =>
						row.public_bundle.account_binding === accountStorageKey(scope),
				),
			);
	});
}

export function removeCertificateAuthority(
	scope: DeviceAccountScope,
	authorityId: string,
): Promise<void> {
	return transact(
		"authorities",
		"readwrite",
		(store, set) => {
			store.delete(itemKey(scope, authorityId));
			set(undefined);
		},
		{ durability: "strict" },
	);
}

export function replaceCertificateAuthority(
	scope: DeviceAccountScope,
	previous: LocalCertificateAuthority,
	next: LocalCertificateAuthority,
): Promise<void> {
	if (
		next.public_bundle.account_binding !== accountStorageKey(scope) ||
		next.public_bundle.authority_id !== previous.public_bundle.authority_id ||
		next.public_bundle.root_certificate_pem !==
			previous.public_bundle.root_certificate_pem ||
		next.vault.length < 64 ||
		next.vault.length > 65_600
	)
		return Promise.reject(
			new Error("The renewed authority does not match the saved root."),
		);
	return transact(
		"authorities",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, previous.public_bundle.authority_id);
			const request = store.get(key);
			request.onsuccess = () => {
				const current = request.result as LocalCertificateAuthority | undefined;
				if (!current || !sameBytes(current.vault, previous.vault)) {
					fail(
						new Error(
							"The local authority changed. Refresh before renewing it.",
						),
					);
					return;
				}
				store.put(
					{ public_bundle: next.public_bundle, vault: next.vault },
					key,
				);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

export function controllerBackup(
	text: string,
	scope: DeviceAccountScope,
	deviceId: string,
): LocalDeviceVault {
	if (new TextEncoder().encode(text).length > 1024 * 1024)
		throw new Error("Controller backups must be smaller than 1 MiB.");
	const value = JSON.parse(text) as Record<string, unknown> | null;
	if (!value || typeof value !== "object")
		throw new Error("Invalid encrypted controller backup.");
	const bytes = (value: unknown): Uint8Array => {
		if (
			!Array.isArray(value) ||
			value.length < 64 ||
			value.length > 65_600 ||
			!value.every((byte) => Number.isInteger(byte) && byte >= 0 && byte <= 255)
		)
			throw new Error("Invalid encrypted controller backup.");
		return Uint8Array.from(value);
	};
	if (
		value.version !== 1 ||
		value.apiOrigin !== scope.apiOrigin ||
		value.deviceId !== deviceId ||
		typeof value.manifestJws !== "string" ||
		value.manifestJws.length > 16_384 ||
		typeof value.grantId !== "string" ||
		!value.controllerPublic ||
		typeof value.controllerPublic !== "object" ||
		(value.controllerPublic as ControllerPublic).device_id !== deviceId
	)
		throw new Error("This backup belongs to another device or hub.");
	const vault: LocalDeviceVault = {
		deviceId,
		controllerPublic: value.controllerPublic as ControllerPublic,
		controllerVault: bytes(value.controllerVault),
		invitationVault: value.invitationVault
			? bytes(value.invitationVault)
			: undefined,
		manifestJws: value.manifestJws,
		grantId: value.grantId,
		ownerControllerKey: (value.ownerControllerKey ?? undefined) as
			| Ed25519PublicKey
			| undefined,
		requiresFreshEndpoint: true,
	};
	assertVaultAuthority(scope, vault, unsignedManifest(vault.manifestJws));
	return vault;
}

export function replaceRestoredVault(
	scope: DeviceAccountScope,
	previous: LocalDeviceVault,
	next: LocalDeviceVault,
): Promise<void> {
	if (!previous.requiresFreshEndpoint)
		return Promise.reject(
			new Error("This controller vault is not a restored endpoint."),
		);
	return replaceEndpointVault(scope, previous, next);
}

/** Rotate only this browser's private group state, retaining its approved authority. */
export function replaceEndpointVault(
	scope: DeviceAccountScope,
	previous: LocalDeviceVault,
	next: LocalDeviceVault,
): Promise<void> {
	if (
		previous.deviceId !== next.deviceId ||
		previous.manifestJws !== next.manifestJws ||
		previous.grantId !== next.grantId ||
		previous.controllerPublic.controller_key.x !==
			next.controllerPublic.controller_key.x ||
		JSON.stringify(previous.controllerPublic.archive_key) !==
			JSON.stringify(next.controllerPublic.archive_key) ||
		previous.controllerPublic.endpoint_id ===
			next.controllerPublic.endpoint_id ||
		next.requiresFreshEndpoint ||
		next.controllerVault.length < 64
	)
		return Promise.reject(
			new Error(
				"The replacement endpoint changed its approved device authority.",
			),
		);
	return transact("vaults", "readwrite", (store, set, fail) => {
		const key = itemKey(scope, previous.deviceId);
		const request = store.get(key);
		request.onsuccess = () => {
			const current = request.result as LocalDeviceVault | undefined;
			if (
				!current ||
				current.requiresFreshEndpoint !== previous.requiresFreshEndpoint ||
				current.controllerPublic.controller_key.x !==
					next.controllerPublic.controller_key.x ||
				current.controllerVault.length !== previous.controllerVault.length ||
				!current.controllerVault.every(
					(byte, index) => byte === previous.controllerVault[index],
				)
			) {
				fail(
					new Error(
						"The restored controller vault changed. Close and reopen device management.",
					),
				);
				return;
			}
			store.put(next, key);
			set(undefined);
		};
	});
}

export function sameCheckpoint(
	left: Checkpoint | null | undefined,
	right: Checkpoint | null | undefined,
): boolean {
	if (!left || !right) return !left && !right;
	return (
		left.revision === right.revision &&
		left.digest === right.digest &&
		left.store_id.length === 32 &&
		right.store_id.length === 32 &&
		left.store_id.every((byte, index) => byte === right.store_id[index])
	);
}

export function readMlsSnapshot(
	scope: DeviceAccountScope,
	deviceId: string,
	endpointId: string,
	audience: string,
): Promise<SnapshotRow | undefined> {
	return transact("snapshots", "readonly", (store, set) => {
		const request = store.get(itemKey(scope, deviceId, endpointId, audience));
		request.onsuccess = () => set(request.result);
	});
}

export async function commitMlsSnapshot(
	scope: DeviceAccountScope,
	deviceId: string,
	endpointId: string,
	audience: string,
	endpoint: BrowserMlsEndpoint,
): Promise<unknown> {
	const prepared = endpoint.preparedSnapshot();
	const key = itemKey(scope, deviceId, endpointId, audience);
	try {
		await transact<void>("snapshots", "readwrite", (store, set, fail) => {
			const current = store.get(key);
			current.onsuccess = () => {
				const existing = current.result as SnapshotRow | undefined;
				if (
					!sameCheckpoint(existing?.checkpoint, prepared.previous_checkpoint)
				) {
					fail(
						new Error(
							"This MLS endpoint changed in another session. Lock and reopen it.",
						),
					);
					return;
				}
				store.put(
					{ snapshot: prepared.snapshot, checkpoint: prepared.checkpoint },
					key,
				);
				set(undefined);
			};
		});
	} catch (error) {
		endpoint.discardPrepared();
		throw error;
	}
	return endpoint.confirmCommit(prepared.checkpoint);
}

export class DeviceLockHeldError extends Error {
	constructor() {
		super("This device is unlocked in another tab. Lock it there first.");
		this.name = "DeviceLockHeldError";
	}
}
export class DeviceLockUnsupportedError extends Error {
	constructor() {
		super(
			"This browser cannot exclusively lock device keys. Use a supported browser.",
		);
		this.name = "DeviceLockUnsupportedError";
	}
}

export interface DeviceLockOptions {
	/** Web Locks `steal`: the previous holder's request rejects and its `onLost` runs. */
	steal?: boolean;
	/** Runs once when another window steals this lock before it was released. */
	onLost?: () => void;
}
export type DeviceLockHolder =
	| "free"
	| "held_here"
	| "held_elsewhere"
	| "unsupported";

const locksHeldHere = new Map<string, number>();
function countHeldHere(name: string, delta: 1 | -1) {
	const next = (locksHeldHere.get(name) ?? 0) + delta;
	if (next > 0) locksHeldHere.set(name, next);
	else locksHeldHere.delete(name);
}
function deviceLockName(scope: DeviceAccountScope, deviceId: string) {
	return itemKey(scope, deviceId, "unlock");
}

/** Hold this lock until the controller and all its derived sessions are closed. */
export async function acquireDeviceLock(
	scope: DeviceAccountScope,
	deviceId: string,
	options: DeviceLockOptions = {},
): Promise<() => void> {
	const locks = globalThis.navigator?.locks;
	if (!locks) throw new DeviceLockUnsupportedError();
	const name = deviceLockName(scope, deviceId);
	let state: "waiting" | "held" | "released" | "lost" = "waiting";
	let release!: () => void;
	let acquired!: () => void;
	let rejected!: (error: unknown) => void;
	const ready = new Promise<void>((resolve, reject) => {
		acquired = resolve;
		rejected = reject;
	});
	const hold = new Promise<void>((resolve) => {
		release = () => {
			if (state === "held") {
				state = "released";
				countHeldHere(name, -1);
			}
			resolve();
		};
	});
	void locks
		.request(
			name,
			options.steal
				? { mode: "exclusive", steal: true }
				: { mode: "exclusive", ifAvailable: true },
			async (lock) => {
				if (!lock) {
					rejected(new DeviceLockHeldError());
					return;
				}
				state = "held";
				countHeldHere(name, 1);
				acquired();
				await hold;
			},
		)
		.catch((error) => {
			if (state !== "held") {
				rejected(error);
				return;
			}
			state = "lost";
			countHeldHere(name, -1);
			options.onLost?.();
		});
	await ready;
	return release;
}

/** A key-session lease already holds this device's lock; without one the lock is taken for a single call. */
export async function holdDeviceLock(
	scope: DeviceAccountScope,
	deviceId: string,
	lease?: { vault: LocalDeviceVault },
): Promise<() => void> {
	if (!lease) return acquireDeviceLock(scope, deviceId);
	if (lease.vault.deviceId !== deviceId)
		throw new Error(
			`The key session for device ${lease.vault.deviceId} cannot lend its lock to device ${deviceId}.`,
		);
	return () => undefined;
}

/** "held_here" counts only locks taken through `acquireDeviceLock` in this window. */
export async function queryDeviceLock(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<DeviceLockHolder> {
	const locks = globalThis.navigator?.locks;
	if (!locks) return "unsupported";
	const name = deviceLockName(scope, deviceId);
	if (locksHeldHere.has(name)) return "held_here";
	try {
		const snapshot = await locks.query();
		return snapshot.held?.some((lock) => lock.name === name)
			? "held_elsewhere"
			: "free";
	} catch {
		return "free";
	}
}

export interface AccountRecoveryState {
	revision: number;
	sourceDigest?: string;
	pending?: {
		sourceDigest: string;
		request: AccountRecoveryWrite;
		/** Sealed before a later password change, so publishing it keeps the flag. */
		sealedBeforePasswordChange?: boolean;
	};
	/** Set by a password change, cleared once a backup sealed after it is published. */
	passwordChangedSinceBackup?: boolean;
}

export function readAccountRecoveryState(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<AccountRecoveryState> {
	return transact("recovery", "readonly", (store, set) => {
		const request = store.get(itemKey(scope, deviceId));
		request.onsuccess = () => set(request.result ?? { revision: 0 });
	});
}

/** Keyed by device id; devices without a backup watermark are absent. */
export async function listAccountRecoveryStates(
	scope: DeviceAccountScope,
): Promise<Record<string, AccountRecoveryState>> {
	const [keys, values] = await readRange<AccountRecoveryState>(
		"recovery",
		accountRange(scope),
	);
	const result: Record<string, AccountRecoveryState> = {};
	for (const [index, key] of keys.entries()) {
		const parts = keyParts(key);
		if (parts.length === 2 && parts[0] === accountStorageKey(scope))
			result[parts[1]] = values[index];
	}
	return result;
}

export function markPasswordChangedSinceBackup(
	scope: DeviceAccountScope,
	deviceId: string,
	changed: boolean,
): Promise<void> {
	return transact(
		"recovery",
		"readwrite",
		(store, set) => {
			const key = itemKey(scope, deviceId);
			const read = store.get(key);
			read.onsuccess = () => {
				const { passwordChangedSinceBackup: _, ...rest }: AccountRecoveryState =
					read.result ?? { revision: 0 };
				store.put(
					changed
						? {
								...rest,
								passwordChangedSinceBackup: true,
								pending: rest.pending && {
									...rest.pending,
									sealedBeforePasswordChange: true,
								},
							}
						: rest,
					key,
				);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

export function stageAccountRecovery(
	scope: DeviceAccountScope,
	deviceId: string,
	sourceDigest: string,
	request: AccountRecoveryWrite,
): Promise<void> {
	return transact(
		"recovery",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, deviceId);
			const read = store.get(key);
			read.onsuccess = () => {
				const current: AccountRecoveryState = read.result ?? { revision: 0 };
				if (current.pending || current.revision + 1 !== request.revision) {
					fail(
						new Error(
							"The account backup changed in another app tab. Reopen recovery before saving.",
						),
					);
					return;
				}
				store.put({ ...current, pending: { sourceDigest, request } }, key);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

export function completeAccountRecovery(
	scope: DeviceAccountScope,
	deviceId: string,
	request: AccountRecoveryWrite,
): Promise<void> {
	return transact(
		"recovery",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, deviceId);
			const read = store.get(key);
			read.onsuccess = () => {
				const current: AccountRecoveryState | undefined = read.result;
				if (
					!current?.pending ||
					current.pending.request.revision !== request.revision ||
					current.pending.request.ciphertext !== request.ciphertext
				) {
					fail(
						new Error(
							"The pending account backup changed. Reopen recovery before retrying.",
						),
					);
					return;
				}
				store.put(
					{
						revision: request.revision,
						sourceDigest: current.pending.sourceDigest,
						...(current.pending.sealedBeforePasswordChange &&
							current.passwordChangedSinceBackup && {
								passwordChangedSinceBackup: true,
							}),
					},
					key,
				);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
}

/** Kept local envelopes keep their password-change flag; restored ones open with the backup's password. */
function restoredWatermark(
	current: AccountRecoveryState,
	revision: number,
	sourceDigest: string,
	keptLocalVault: boolean,
): AccountRecoveryState {
	return keptLocalVault && current.passwordChangedSinceBackup
		? { revision, sourceDigest, passwordChangedSinceBackup: true }
		: { revision, sourceDigest };
}

/** Restored keys and their rollback watermark become visible in one durable commit. */
export function restoreAccountRecoveryVault(
	scope: DeviceAccountScope,
	vault: LocalDeviceVault,
	revision: number,
	sourceDigest: string,
	reconciledPending?: AccountRecoveryWrite,
): Promise<LocalDeviceVault> {
	if (
		vault.controllerPublic.device_id !== vault.deviceId ||
		vault.controllerVault.length < 64 ||
		!vault.manifestJws ||
		!Number.isSafeInteger(revision) ||
		revision < 1
	)
		return Promise.reject(new Error("Invalid encrypted device recovery."));
	return transactStores(
		["vaults", "recovery"],
		"readwrite",
		(transaction, set, fail) => {
			const key = itemKey(scope, vault.deviceId);
			const recovery = transaction.objectStore("recovery");
			const vaults = transaction.objectStore("vaults");
			const read = recovery.get(key);
			read.onsuccess = () => {
				const current: AccountRecoveryState = read.result ?? { revision: 0 };
				const pendingMatches =
					current.pending &&
					reconciledPending &&
					current.pending.request.revision === reconciledPending.revision &&
					current.pending.request.ciphertext === reconciledPending.ciphertext &&
					current.pending.request.proof_jws === reconciledPending.proof_jws &&
					revision >= reconciledPending.revision;
				if (
					(current.pending
						? !pendingMatches
						: reconciledPending !== undefined) ||
					current.revision > revision
				) {
					fail(
						new Error(
							"An account backup is pending or the downloaded revision is older than this app has seen.",
						),
					);
					return;
				}
				const lookup = vaults.get(key);
				lookup.onsuccess = () => {
					const existing = lookup.result as LocalDeviceVault | undefined;
					if (
						existing &&
						existing.controllerPublic.controller_key.x !==
							vault.controllerPublic.controller_key.x
					) {
						fail(
							new Error(
								"This app has a different controller. Export its backup before replacing local keys.",
							),
						);
						return;
					}
					// A verified cloud copy may reconcile a concurrent browser's
					// revision; keep this browser's password envelopes and endpoint.
					if (!existing) vaults.add(vault, key);
					recovery.put(
						restoredWatermark(current, revision, sourceDigest, !!existing),
						key,
					);
					set(existing ?? vault);
				};
			};
		},
		{ durability: "strict" },
	);
}

export interface InventoryAnchor {
	scope: InventoryScope;
	revision: number;
	digest: string;
}
/** Keyed by `inventoryScopeKey(scope)`. */
export type InventoryAnchors = Record<string, InventoryAnchor>;

function inventoryAnchorKey(
	scope: DeviceAccountScope,
	deviceId: string,
	controllerKey: string,
): string {
	return itemKey(scope, deviceId, controllerKey, "inventory");
}

export function readInventoryAnchors(
	scope: DeviceAccountScope,
	deviceId: string,
	controllerKey: string,
): Promise<InventoryAnchors> {
	return transact("fleet", "readonly", (store, set) => {
		const request = store.get(
			inventoryAnchorKey(scope, deviceId, controllerKey),
		);
		request.onsuccess = () => set(request.result ?? {});
	});
}

/** Compare-and-set in one transaction; `update` throws to abort. `stored` is false before the first write (one-time import). */
export function updateInventoryAnchors(
	scope: DeviceAccountScope,
	deviceId: string,
	controllerKey: string,
	update: (current: InventoryAnchors, stored: boolean) => InventoryAnchors,
): Promise<InventoryAnchors> {
	return transact(
		"fleet",
		"readwrite",
		(store, set, fail) => {
			const key = inventoryAnchorKey(scope, deviceId, controllerKey);
			const read = store.get(key);
			read.onsuccess = () => {
				try {
					const next = update(read.result ?? {}, read.result !== undefined);
					if (JSON.stringify(next).length > 128 * 1024)
						throw new Error(
							"Saved inventory anchors exceed their local limit.",
						);
					store.put(next, key);
					set(next);
				} catch (error) {
					fail(
						error instanceof Error
							? error
							: new Error("Saved inventory anchor update failed."),
					);
				}
			};
		},
		{ durability: "strict" },
	);
}

/** Only signed public declarations and rollback watermarks are retained here. */
export function updateFleetState(
	scope: DeviceAccountScope,
	deviceId: string,
	controllerKey: string,
	update: (current: FleetLocalState) => FleetLocalState,
): Promise<FleetLocalState> {
	return transact(
		"fleet",
		"readwrite",
		(store, set, fail) => {
			const key = itemKey(scope, deviceId, controllerKey);
			const read = store.get(key);
			read.onsuccess = () => {
				try {
					const next = update(read.result ?? { revision: 0, anchors: {} });
					if (JSON.stringify(next).length > 128 * 1024)
						throw new Error("Fleet trust history exceeds its local limit.");
					store.put(next, key);
					set(next);
				} catch (error) {
					fail(
						error instanceof Error
							? error
							: new Error("Fleet trust update failed."),
					);
				}
			};
		},
		{ durability: "strict" },
	);
}
