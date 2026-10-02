import { expect, test } from "bun:test";
import type { QueryClient } from "@tanstack/react-query";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import type { LocalCertificateAuthority } from "../certificate-authority";
import { base64url } from "../crypto";
import { identityFingerprint } from "../fingerprint";
import {
	type AccountRecoveryState,
	type DeviceIdentityPinRecord,
	type LocalDeviceVault,
	deviceIdentityKey,
} from "../storage";
import type { DeviceCrypto, DeviceReceipt } from "../types";
import {
	type LocalBackupSummary,
	type LocalInventoryIo,
	createLocalInventory,
} from "./local";
import type { WorkspaceDeps } from "./types";

const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function identity(seed: number): DeviceReceipt["identity"] {
	const key = (fill: number) => ({
		kty: "OKP" as const,
		crv: "Ed25519" as const,
		x: base64url(new Uint8Array(32).fill(fill)),
	});
	return {
		auth_key: key(seed),
		telemetry_key: key(seed + 1),
		management_key: Array(32).fill(seed + 2),
	};
}
function vault(deviceId: string, grantId = "owner"): LocalDeviceVault {
	const key = {
		kty: "OKP" as const,
		crv: "Ed25519" as const,
		x: `${deviceId}-key`,
	};
	return {
		deviceId,
		grantId,
		manifestJws: "manifest",
		controllerPublic: {
			device_id: deviceId,
			endpoint_id: "endpoint",
			controller_key: key,
			archive_key: Array(32).fill(7),
			telemetry_member: { endpoint_id: "endpoint", signing_key: key },
		},
		controllerVault: new Uint8Array(80).fill(1),
		requiresFreshEndpoint: grantId === "owner" ? true : undefined,
	};
}
function pin(
	enrollmentId: string,
	seed: number,
	pinnedAt: number,
): DeviceIdentityPinRecord {
	return {
		enrollmentId,
		identity: deviceIdentityKey(identity(seed)),
		pinnedAt,
	};
}

function harness() {
	const vaults = new Map<string, LocalDeviceVault>([
		["dev-a", vault("dev-a")],
		["dev-b", vault("dev-b", "reader-grant")],
	]);
	const pins = new Map<string, DeviceIdentityPinRecord[]>([
		["dev-a", [pin("second", 10, 2_000), pin("first", 20, 1_000)]],
	]);
	const states: Record<string, AccountRecoveryState> = {
		"dev-a": {
			revision: 2,
			sourceDigest: "old",
			passwordChangedSinceBackup: true,
		},
		"dev-b": {
			revision: 0,
			pending: {
				sourceDigest: "b",
				request: {
					public_key: vault("dev-b").controllerPublic.controller_key,
					ciphertext: "c",
					revision: 1,
					proof_jws: "p",
				},
			},
		},
	};
	const authority = {
		public_bundle: {
			authority_id: "ca-1",
			label: "Plant CA",
			issuer_not_after: 300,
			not_after: 900,
		},
	} as unknown as LocalCertificateAuthority;
	const calls: string[] = [];
	let failVaults = false;
	let persisted: "persisted" | "denied" = "denied";
	const io: Partial<LocalInventoryIo> = {
		async listDeviceVaults() {
			if (failVaults) throw new Error("Encrypted device storage failed.");
			return [...vaults.values()];
		},
		async readDeviceIdentityPins(_scope, deviceId) {
			return pins.get(deviceId) ?? [];
		},
		async forgetDeviceIdentityPin(_scope, deviceId) {
			calls.push(`forget ${deviceId}`);
			pins.delete(deviceId);
		},
		async deleteDeviceVault(_scope, deviceId) {
			calls.push(`delete ${deviceId}`);
			vaults.delete(deviceId);
			pins.delete(deviceId);
			delete states[deviceId];
		},
		async listAccountRecoveryStates() {
			return structuredClone(states);
		},
		async readCertificateAuthorities() {
			return [authority];
		},
		async readStoragePersistence() {
			return persisted;
		},
		async requestPersistentDeviceStorage() {
			persisted = "persisted";
			return persisted;
		},
		async backupSourceDigest() {
			return "new";
		},
	};
	let reader: { revision: number; deleted: boolean } | Error = {
		revision: 3,
		deleted: false,
	};
	const api = {
		async get(_profile: IProfile, path: string) {
			calls.push(`GET ${path}`);
			if (reader instanceof Error) throw reader;
			return reader;
		},
		async del(_profile: IProfile, path: string, body: unknown) {
			calls.push(`DELETE ${path} ${JSON.stringify(body)}`);
		},
	} as unknown as IApiState;
	let cryptoFails = false;
	const deps: WorkspaceDeps = {
		api,
		profile: {} as IProfile,
		scope,
		queryClient: {} as QueryClient,
		crypto: async () => {
			if (cryptoFails) throw new Error("WASM missing");
			return {} as DeviceCrypto;
		},
		platform: "web",
		now: () => 5_000,
	};
	return {
		deps,
		io,
		calls,
		failVaults: (value: boolean) => {
			failVaults = value;
		},
		failCrypto: () => {
			cryptoFails = true;
		},
		setReader: (value: typeof reader) => {
			reader = value;
		},
	};
}

test("the summary lists this scope's vaults, newest pins, backups, authorities and persistence", async () => {
	const { deps, io } = harness();
	const local = createLocalInventory(deps, io);
	let notified = 0;
	local.subscribe(() => notified++);
	expect(local.summary()).toMatchObject({
		platform: "web",
		persistence: "unknown",
		cryptoLoaded: "unknown",
		vaults: [],
	});
	await local.reload();
	await flush();
	const summary = local.summary();
	expect(summary.persistence).toBe("denied");
	expect(summary.cryptoLoaded).toBe(true);
	expect(summary.vaults).toEqual([
		{
			deviceId: "dev-a",
			role: "owner",
			grantId: "owner",
			requiresFreshEndpoint: true,
			identityPinnedAt: 2_000,
			identityFingerprint: identityFingerprint(identity(10)),
		},
		{
			deviceId: "dev-b",
			role: "shared",
			grantId: "reader-grant",
			requiresFreshEndpoint: false,
			identityPinnedAt: undefined,
			identityFingerprint: undefined,
		},
	]);
	expect(summary.backups as Record<string, LocalBackupSummary>).toEqual({
		"dev-a": {
			localRevision: 2,
			pending: false,
			passwordChangedSinceBackup: true,
			sourceDigestChanged: true,
		},
		"dev-b": { localRevision: 0, pending: true },
	});
	expect(summary.authorities).toEqual([
		{
			authorityId: "ca-1",
			label: "Plant CA",
			issuingNotAfter: 300,
			rootNotAfter: 900,
		},
	]);
	expect(notified).toBeGreaterThanOrEqual(2);
	const unchanged = local.summary();
	expect(local.summary()).toBe(unchanged);
});

test("a reload reads every identity pin at once and overlapping reloads share one follow-up", async () => {
	const { deps, io } = harness();
	const { readDeviceIdentityPins: _, ...shared } = io;
	const reads = { vaults: 0, pins: 0 };
	const local = createLocalInventory(deps, {
		...shared,
		async listDeviceVaults(scope) {
			reads.vaults++;
			return (await io.listDeviceVaults?.(scope)) ?? [];
		},
		async listDeviceIdentityPins() {
			reads.pins++;
			return new Map([
				["dev-a", [pin("second", 10, 2_000), pin("first", 20, 1_000)]],
				["gone", [pin("old", 30, 500)]],
			]);
		},
	});
	const settled = await Promise.allSettled([
		local.reload(),
		local.reload(),
		local.reload(),
		local.reload(),
	]);
	expect(settled.map((result) => result.status)).toEqual(
		Array(4).fill("fulfilled"),
	);
	expect(reads).toEqual({ vaults: 2, pins: 2 });
	expect(
		local.summary().vaults.map((row) => [row.deviceId, row.identityPinnedAt]),
	).toEqual([
		["dev-a", 2_000],
		["dev-b", undefined],
	]);
	expect(local.identityCheck("dev-a", identity(10))).toBe("match");
	expect(local.identityCheck("gone", identity(30))).toBe("unpinned");
	await local.reload();
	expect(reads).toEqual({ vaults: 3, pins: 3 });
});

test("the identity pre-check compares only against the newest pin", async () => {
	const { deps, io, calls } = harness();
	const local = createLocalInventory(deps, io);
	expect(local.identityCheck("dev-a", identity(10))).toBe("unpinned");
	await local.reload();
	expect(local.identityCheck("dev-a", identity(10))).toBe("match");
	expect(local.identityCheck("dev-a", identity(20))).toBe("mismatch");
	expect(local.identityCheck("dev-b", identity(10))).toBe("unpinned");
	await local.forgetIdentity("dev-a");
	expect(calls).toEqual(["forget dev-a"]);
	expect(local.identityCheck("dev-a", identity(20))).toBe("unpinned");
	expect(local.summary().vaults[0].identityPinnedAt).toBeUndefined();
});

test("deleting keys releases the fleet reader when possible and always removes local rows", async () => {
	const { deps, io, calls, setReader } = harness();
	const local = createLocalInventory(deps, io);
	await local.reload();
	await local.deleteKeys("dev-a");
	const path = "devices/dev-a/fleet/readers/dev-a-key";
	expect(calls).toEqual([
		`GET ${path}`,
		`DELETE ${path} {"revision":4}`,
		"delete dev-a",
	]);
	expect(local.summary().vaults.map((row) => row.deviceId)).toEqual(["dev-b"]);
	expect(local.summary().backups["dev-a"]).toBeUndefined();

	calls.length = 0;
	setReader(new Error("Hub unreachable"));
	await local.deleteKeys("dev-b");
	expect(calls).toEqual([
		"GET devices/dev-b/fleet/readers/dev-b-key",
		"delete dev-b",
	]);
	expect(local.summary().vaults).toEqual([]);

	calls.length = 0;
	setReader({ revision: 5, deleted: true });
	await local.deleteKeys("unknown");
	expect(calls).toEqual(["delete unknown"]);
});

test("a failed read keeps the last good value, still applies the rest and rejects", async () => {
	const { deps, io, failVaults, failCrypto } = harness();
	const local = createLocalInventory(deps, io);
	await local.reload();
	const before = local.summary().vaults;
	failVaults(true);
	failCrypto();
	await expect(local.reload()).rejects.toThrow("storage failed");
	expect(local.summary().vaults).toBe(before);
	expect(local.identityCheck("dev-a", identity(10))).toBe("match");
	expect(await local.requestPersistence()).toBe("persisted");
	expect(local.summary().persistence).toBe("persisted");
	expect(local.summary().cryptoLoaded).toBe(true);
});

test("crypto that cannot load is reported, and reload probes it again", async () => {
	const { deps, io, failCrypto } = harness();
	failCrypto();
	const local = createLocalInventory(deps, io);
	await local.reload();
	await flush();
	expect(local.summary().cryptoLoaded).toBe(false);
	deps.crypto = async () => ({}) as DeviceCrypto;
	await local.reload();
	await flush();
	expect(local.summary().cryptoLoaded).toBe(true);
});
