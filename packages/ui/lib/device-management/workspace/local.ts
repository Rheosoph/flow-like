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
	listDeviceIdentityPins,
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
	/** One read for every device's pins; used unless only `readDeviceIdentityPins` is replaced. */
	listDeviceIdentityPins: typeof listDeviceIdentityPins;
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
	listDeviceIdentityPins,
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
	const pinsPerDevice =
		overrides.readDeviceIdentityPins !== undefined &&
		overrides.listDeviceIdentityPins === undefined;
	const listeners = new Set<() => void>();
	const vaults = new Map<string, LocalDeviceVault>();
	const newestPins = new Map<string, DeviceIdentityPinRecord>();
	let loading: Promise<void> | undefined;
	let queued: Promise<void> | undefined;
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

	async function readPins(rows: LocalDeviceVault[]) {
		if (!pinsPerDevice) return io.listDeviceIdentityPins(deps.scope);
		return new Map(
			await Promise.all(
				rows.map(
					async ({ deviceId }) =>
						[
							deviceId,
							await io.readDeviceIdentityPins(deps.scope, deviceId),
						] as const,
				),
			),
		);
	}

	async function readVaults(): Promise<VaultRow[]> {
		const rows = await io.listDeviceVaults(deps.scope);
		const pins = await readPins(rows);
		return rows.map((vault) => ({ vault, pin: pins.get(vault.deviceId)?.[0] }));
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
	async function load() {
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

	/** One read at a time; callers that arrive while it runs share a single read after it, so none sees older rows. */
	function reload(): Promise<void> {
		if (!loading) {
			const run = load().finally(() => {
				if (loading === run) loading = undefined;
			});
			loading = run;
			return run;
		}
		queued ??= loading
			.catch(() => undefined)
			.then(() => {
				queued = undefined;
				return reload();
			});
		return queued;
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
