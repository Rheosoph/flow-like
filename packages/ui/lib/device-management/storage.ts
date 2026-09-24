import type {
	AccountRecoveryWrite,
	FleetLocalState,
	BrowserMlsEndpoint,
	Checkpoint,
	ControllerPublic,
	Ed25519PublicKey,
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
		const request = indexedDB.open("flow-like-device-management", 3);
		request.onupgradeneeded = () => {
			for (const name of ["vaults", "snapshots", "recovery", "fleet"])
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
	return transact("vaults", "readwrite", (store, set) => {
		store.add(vault, itemKey(scope, vault.deviceId));
		set(undefined);
	});
}

export function controllerBackup(
	text: string,
	scope: DeviceAccountScope,
	deviceId: string,
): LocalDeviceVault {
	if (new TextEncoder().encode(text).length > 1024 * 1024)
		throw new Error("Controller backups must be smaller than 1 MiB.");
	const value = JSON.parse(text) as Record<string, unknown>;
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
		typeof value.controllerPublic !== "object"
	)
		throw new Error("This backup belongs to another device or hub.");
	return {
		deviceId,
		controllerPublic: value.controllerPublic as ControllerPublic,
		controllerVault: bytes(value.controllerVault),
		invitationVault: value.invitationVault
			? bytes(value.invitationVault)
			: undefined,
		manifestJws: value.manifestJws,
		grantId: value.grantId,
		ownerControllerKey: value.ownerControllerKey as
			| Ed25519PublicKey
			| undefined,
		requiresFreshEndpoint: true,
	};
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

/** Hold this lock until the controller and all its derived sessions are closed. */
export async function acquireDeviceLock(
	scope: DeviceAccountScope,
	deviceId: string,
): Promise<() => void> {
	if (!navigator.locks)
		throw new Error(
			"This browser cannot exclusively lock device keys. Use a supported browser.",
		);
	let release!: () => void;
	let acquired!: () => void;
	let rejected!: (error: unknown) => void;
	const ready = new Promise<void>((resolve, reject) => {
		acquired = resolve;
		rejected = reject;
	});
	const hold = new Promise<void>((resolve) => {
		release = resolve;
	});
	void navigator.locks
		.request(
			itemKey(scope, deviceId, "unlock"),
			{ mode: "exclusive", ifAvailable: true },
			async (lock) => {
				if (!lock) {
					rejected(
						new Error(
							"This device is unlocked in another tab. Lock it there first.",
						),
					);
					return;
				}
				acquired();
				await hold;
			},
		)
		.catch(rejected);
	await ready;
	return release;
}

export interface AccountRecoveryState {
	revision: number;
	sourceDigest?: string;
	pending?: { sourceDigest: string; request: AccountRecoveryWrite };
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
					},
					key,
				);
				set(undefined);
			};
		},
		{ durability: "strict" },
	);
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
					recovery.put({ revision, sourceDigest }, key);
					set(existing ?? vault);
				};
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
