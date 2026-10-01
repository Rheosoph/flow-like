import type { LocalCertificateAuthority } from "../certificate-authority";
import { identityFingerprint } from "../fingerprint";
import { removeFleetReader } from "../fleet";
import { backupSourceDigest } from "../recovery";
import {
	type AccountRecoveryState,
	type DeviceIdentityPinRecord,
	type LocalDeviceVault,
	deleteDeviceVault,
	deviceIdentityKey,
	forgetDeviceIdentityPin,
	listAccountRecoveryStates,
	listDeviceVaults,
	pinnedDeviceIdentity,
	readCertificateAuthorities,
	readDeviceIdentityPins,
	readStoragePersistence,
	requestPersistentDeviceStorage,
} from "../storage";
import type {
	LocalInventory,
	LocalSummary,
	LocalVaultSummary,
	WorkspaceDeps,
} from "./types";

/** Storage seams; tests replace them, production uses `storage.ts`. */
export interface LocalInventoryIo {
	listDeviceVaults: typeof listDeviceVaults;
	readDeviceIdentityPins: typeof readDeviceIdentityPins;
	forgetDeviceIdentityPin: typeof forgetDeviceIdentityPin;
	deleteDeviceVault: typeof deleteDeviceVault;
	listAccountRecoveryStates: typeof listAccountRecoveryStates;
	readCertificateAuthorities: typeof readCertificateAuthorities;
	readStoragePersistence: typeof readStoragePersistence;
	requestPersistentDeviceStorage: typeof requestPersistentDeviceStorage;
	backupSourceDigest: typeof backupSourceDigest;
}

const STORAGE_IO: LocalInventoryIo = {
	listDeviceVaults,
	readDeviceIdentityPins,
	forgetDeviceIdentityPin,
	deleteDeviceVault,
	listAccountRecoveryStates,
	readCertificateAuthorities,
	readStoragePersistence,
	requestPersistentDeviceStorage,
	backupSourceDigest,
};

export type LocalBackupSummary = LocalSummary["backups"][string];

function browserSupport() {
	return {
		webLocks: Boolean(globalThis.navigator?.locks),
		indexedDb: Boolean(globalThis.indexedDB),
	};
}

function fingerprintOf(pin: DeviceIdentityPinRecord): string | undefined {
	const identity = pinnedDeviceIdentity(pin);
	try {
		return identity && identityFingerprint(identity);
	} catch {
		return undefined;
	}
}

function vaultSummary(
	vault: LocalDeviceVault,
	pin: DeviceIdentityPinRecord | undefined,
): LocalVaultSummary {
	return {
		deviceId: vault.deviceId,
		role: vault.grantId === "owner" ? "owner" : "shared",
		grantId: vault.grantId,
		requiresFreshEndpoint: Boolean(vault.requiresFreshEndpoint),
		identityPinnedAt: pin?.pinnedAt,
		identityFingerprint: pin && fingerprintOf(pin),
	};
}

async function backupSummary(
	io: LocalInventoryIo,
	scope: WorkspaceDeps["scope"],
	state: AccountRecoveryState,
	vault: LocalDeviceVault | undefined,
): Promise<LocalBackupSummary> {
	const summary: LocalBackupSummary = {
		localRevision: state.revision,
		pending: Boolean(state.pending),
	};
	if (state.passwordChangedSinceBackup)
		summary.passwordChangedSinceBackup = true;
	if (vault && state.sourceDigest)
		summary.sourceDigestChanged =
			(await io
				.backupSourceDigest(scope, vault)
				.catch(() => state.sourceDigest)) !== state.sourceDigest;
	return summary;
}

function patchFrom<T, K extends keyof LocalSummary>(
	result: PromiseSettledResult<T>,
	key: K,
	map: (value: T) => LocalSummary[K],
): Partial<LocalSummary> {
	return result.status === "fulfilled"
		? ({ [key]: map(result.value) } as Partial<LocalSummary>)
		: {};
}

function isRejected(
	result: PromiseSettledResult<unknown>,
): result is PromiseRejectedResult {
	return result.status === "rejected";
}

function authoritySummary({ public_bundle }: LocalCertificateAuthority) {
	return {
		authorityId: public_bundle.authority_id,
		label: public_bundle.label,
		issuingNotAfter: public_bundle.issuer_not_after,
		rootNotAfter: public_bundle.not_after,
	};
}

interface VaultRow {
	vault: LocalDeviceVault;
	pin?: DeviceIdentityPinRecord;
}

export function createLocalInventory(
	deps: WorkspaceDeps,
	overrides: Partial<LocalInventoryIo> = {},
): LocalInventory {
	const io: LocalInventoryIo = { ...STORAGE_IO, ...overrides };
	const listeners = new Set<() => void>();
	const vaults = new Map<string, LocalDeviceVault>();
	const newestPins = new Map<string, DeviceIdentityPinRecord>();
	let generation = 0;
	let cryptoProbe: Promise<void> | undefined;
	let summary: LocalSummary = {
		platform: deps.platform,
		persistence: "unknown",
		...browserSupport(),
		cryptoLoaded: "unknown",
		vaults: [],
		backups: {},
		authorities: [],
	};

	function emit(patch: Partial<LocalSummary>) {
		summary = { ...summary, ...patch };
		for (const listener of listeners) listener();
	}

	function probeCrypto() {
		if (summary.cryptoLoaded === true || cryptoProbe) return;
		cryptoProbe = deps
			.crypto()
			.then(
				() => emit({ cryptoLoaded: true }),
				() => emit({ cryptoLoaded: false }),
			)
			.finally(() => {
				cryptoProbe = undefined;
			});
	}

	async function readVaults(): Promise<VaultRow[]> {
		const rows = await io.listDeviceVaults(deps.scope);
		const pins = await Promise.all(
			rows.map((vault) =>
				io
					.readDeviceIdentityPins(deps.scope, vault.deviceId)
					.then((records) => records[0]),
			),
		);
		return rows.map((vault, index) => ({ vault, pin: pins[index] }));
	}

	async function readBackups(rows: Map<string, LocalDeviceVault>) {
		const states = await io.listAccountRecoveryStates(deps.scope);
		const entries = await Promise.all(
			Object.entries(states).map(
				async ([deviceId, state]) =>
					[
						deviceId,
						await backupSummary(io, deps.scope, state, rows.get(deviceId)),
					] as const,
			),
		);
		return Object.fromEntries(entries);
	}

	function applyVaults(rows: VaultRow[]): LocalVaultSummary[] {
		vaults.clear();
		newestPins.clear();
		for (const { vault, pin } of rows) {
			vaults.set(vault.deviceId, vault);
			if (pin) newestPins.set(vault.deviceId, pin);
		}
		return rows.map(({ vault, pin }) => vaultSummary(vault, pin));
	}

	/** Each source updates on its own; a failed read keeps the previous value and rejects after. */
	async function reload() {
		const run = ++generation;
		probeCrypto();
		const [vaultRows, persistence, authorities] = await Promise.allSettled([
			readVaults(),
			io.readStoragePersistence(),
			io.readCertificateAuthorities(deps.scope),
		]);
		const rows =
			vaultRows.status === "fulfilled"
				? new Map(vaultRows.value.map(({ vault }) => [vault.deviceId, vault]))
				: new Map(vaults);
		const [backups] = await Promise.allSettled([readBackups(rows)]);
		if (run !== generation) return;
		emit({
			...browserSupport(),
			...patchFrom(vaultRows, "vaults", applyVaults),
			...patchFrom(persistence, "persistence", (value) => value),
			...patchFrom(authorities, "authorities", (value) =>
				value.map(authoritySummary),
			),
			...patchFrom(backups, "backups", (value) => value),
		});
		const failed = [vaultRows, persistence, authorities, backups].find(
			isRejected,
		);
		if (failed) throw failed.reason;
	}

	/** Best effort: otherwise the hub keeps storing encrypted status for a reader nobody can open; it expires by itself. */
	function releaseFleetReader(vault: LocalDeviceVault) {
		return removeFleetReader(
			deps.api,
			deps.profile,
			vault.deviceId,
			vault.controllerPublic.controller_key.x,
		).catch(() => undefined);
	}

	return {
		summary: () => summary,
		subscribe(listener) {
			listeners.add(listener);
			return () => listeners.delete(listener);
		},
		reload,
		identityCheck(deviceId, reported) {
			const pin = newestPins.get(deviceId);
			if (!pin) return "unpinned";
			return pin.identity === deviceIdentityKey(reported)
				? "match"
				: "mismatch";
		},
		async forgetIdentity(deviceId) {
			await io.forgetDeviceIdentityPin(deps.scope, deviceId);
			await reload();
		},
		async deleteKeys(deviceId) {
			const vault =
				vaults.get(deviceId) ??
				(await io.listDeviceVaults(deps.scope)).find(
					(row) => row.deviceId === deviceId,
				);
			if (vault) await releaseFleetReader(vault);
			await io.deleteDeviceVault(deps.scope, deviceId);
			await reload();
		},
		async requestPersistence() {
			const persistence = await io.requestPersistentDeviceStorage();
			emit({ persistence });
			return persistence;
		},
	};
}
