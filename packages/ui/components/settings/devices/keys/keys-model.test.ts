import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_ME,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	type KeysModelInput,
	buildKeysModel,
	keyCategory,
	localRevision,
	restoreCandidates,
	rowsWithKeys,
} from "./keys-model";

function input(overrides: Partial<KeysModelInput> = {}): KeysModelInput {
	const seed = sampleFleet();
	return {
		me: SAMPLE_ME,
		devices: seed.devices,
		devicesLoaded: true,
		keys: seed.keys,
		local: seed.local,
		accountBackup: (deviceId) =>
			seed.accountBackups[deviceId] ?? { revision: 0 },
		accessRequests: seed.accessRequests ?? [],
		...overrides,
	};
}

const categoryOf = (model: ReturnType<typeof buildKeysModel>, id: string) =>
	model.rows.find((row) => row.deviceId === id)?.category;

describe("keyCategory", () => {
	const owner = { relationship: "owner" as const };
	test.each([
		["a pending upload wins over everything", { pending: true }, 3, "pending"],
		["no account copy", {}, 0, "never"],
		[
			"password changed after the backup",
			{ passwordChangedSinceBackup: true },
			2,
			"oldpw",
		],
		["the account is ahead", { localRevision: 1 }, 3, "hub_newer"],
		[
			"the keys changed after the backup",
			{ localRevision: 2, sourceDigestChanged: true },
			2,
			"out_of_date",
		],
		["same version", { localRevision: 2 }, 2, "synced"],
		["account not read yet", { localRevision: 2 }, undefined, "unchecked"],
	] as const)("with keys here: %s", (_name, backup, hubRevision, expected) => {
		expect(
			keyCategory({
				...owner,
				hasVault: true,
				backup: { localRevision: 0, pending: false, ...backup },
				hubRevision,
			}),
		).toBe(expected);
	});

	test.each([
		["owner", 2, "restorable"],
		["owner", 0, "lost"],
		["owner", undefined, "unchecked"],
		["shared", 1, "restorable"],
		["cloud_approval", 4, "nokeys"],
		["unknown", 0, "nokeys"],
	] as const)(
		"without keys here: %s with account version %p",
		(relationship, hubRevision, expected) => {
			expect(keyCategory({ relationship, hasVault: false, hubRevision })).toBe(
				expected,
			);
		},
	);
});

describe("buildKeysModel on the golden fleet", () => {
	const model = buildKeysModel(input());

	test("lists every device this computer could manage, riskiest first", () => {
		expect(model.rows.map((row) => row.name)).toEqual([
			"cold-storage-nas",
			"studio-mac-mini",
			"edge-berlin-01",
			"lab-gpu-02",
			"warehouse-pi",
			"partner-edge",
		]);
		expect(categoryOf(model, SAMPLE_IDS.cold)).toBe("never");
		expect(categoryOf(model, SAMPLE_IDS.studio)).toBe("pending");
		expect(categoryOf(model, SAMPLE_IDS.edge)).toBe("synced");
		expect(categoryOf(model, SAMPLE_IDS.partner)).toBe("nokeys");
	});

	test("counts the keys of revoked devices as held here and files them as local-only", () => {
		expect(model.totalDevices).toBe(7);
		expect(model.keysHere).toBe(6);
		expect(model.localOnly.map((row) => [row.name, row.reason])).toEqual([
			["old-kiosk", "revoked"],
			["mira-render-01", "request"],
		]);
		expect(model.rows.some((row) => row.deviceId === SAMPLE_IDS.oldKiosk)).toBe(
			false,
		);
	});

	test("groups the rows for the summary windows", () => {
		expect(model.groups.synced.length).toBe(3);
		expect(model.groups.behind.map((row) => row.name)).toEqual([
			"studio-mac-mini",
		]);
		expect(model.groups.never.map((row) => row.name)).toEqual([
			"cold-storage-nas",
		]);
		expect(model.groups.nokeys.map((row) => row.name)).toEqual([
			"partner-edge",
		]);
		expect(rowsWithKeys(model).length).toBe(5);
		expect(restoreCandidates(model)).toEqual([]);
	});

	test("a staged upload is the version after the last acknowledged one", () => {
		expect(localRevision({ backup: { localRevision: 1, pending: true } })).toBe(
			2,
		);
		expect(
			localRevision({ backup: { localRevision: 3, pending: false } }),
		).toBe(3);
		expect(localRevision({})).toBe(0);
	});
});

describe("buildKeysModel without the facts it needs", () => {
	test("keys without a listed device are not called leftovers before the list answered", () => {
		const model = buildKeysModel(input({ devices: [], devicesLoaded: false }));
		expect(model.rows).toEqual([]);
		expect(model.localOnly).toEqual([]);
	});

	test("an unread account backup is 'unchecked', never 'not backed up'", () => {
		const model = buildKeysModel(input({ accountBackup: () => undefined }));
		expect(categoryOf(model, SAMPLE_IDS.cold)).toBe("unchecked");
		expect(categoryOf(model, SAMPLE_IDS.studio)).toBe("pending");
		expect(model.unchecked.length).toBe(4);
		expect(model.groups.never).toEqual([]);
	});

	test("a new computer offers every backed-up device for restore and names the lost one", () => {
		const seed = sampleFleet();
		const model = buildKeysModel(
			input({ local: { ...seed.local, vaults: [], backups: {} }, keys: [] }),
		);
		expect(model.keysHere).toBe(0);
		expect(restoreCandidates(model).map((row) => row.name)).toEqual([
			"cold-storage-nas",
			"edge-berlin-01",
			"lab-gpu-02",
			"studio-mac-mini",
			"warehouse-pi",
		]);
		expect(categoryOf(model, SAMPLE_IDS.cold)).toBe("lost");
		expect(categoryOf(model, SAMPLE_IDS.edge)).toBe("restorable");
	});

	test("a vault for a device that left the list is an unlisted leftover", () => {
		const seed = sampleFleet();
		const model = buildKeysModel(
			input({
				devices: seed.devices.filter(
					(row) => row.device_id !== SAMPLE_IDS.warehouse,
				),
			}),
		);
		expect(
			model.localOnly.find((row) => row.deviceId === SAMPLE_IDS.warehouse)
				?.reason,
		).toBe("unlisted");
	});
});
