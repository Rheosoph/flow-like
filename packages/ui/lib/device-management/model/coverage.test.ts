import { describe, expect, test } from "bun:test";
import type { ManagementGrant } from "../types";
import {
	SAMPLE_APPS,
	SAMPLE_IDS,
	SAMPLE_PEOPLE,
	ed25519,
	sampleFleet,
	sampleFleetOlderHub,
} from "./__fixtures__/sample-fleet";
import { generateFleet } from "./__fixtures__/sample-fleet-200";
import type { AttentionInputExt } from "./attention";
import { type Coverage, coverage } from "./coverage";

const ID = SAMPLE_IDS;
const APP = SAMPLE_APPS;

function expectConsistent(result: Coverage, appId?: string) {
	expect(result.readable + result.unknown.length + result.noAccess.length).toBe(
		result.total,
	);
	expect(result.live + result.snapshot).toBe(result.readable);
	expect(result.locked.length + result.noKeys.length).toBeLessThanOrEqual(
		result.unknown.length,
	);
	if (appId)
		expect(result.deployed.length + result.notDeployed.length).toBe(
			result.readable,
		);
	else expect(result.deployed.length + result.notDeployed.length).toBe(0);
}

function labGrant(scope: ManagementGrant["scope"]): ManagementGrant {
	return {
		grant_id: "lab-grant",
		user_id: SAMPLE_PEOPLE.felix,
		controller_key: ed25519("controller-felix"),
		scope,
		capabilities: ["status"],
		expires_at: 1_790_820_000,
		group_id: null,
		group_version: null,
	};
}

/** The lab device without BG22 my-access: scope comes from the verified policy, if any. */
function labFromPolicy(scope?: ManagementGrant["scope"]): AttentionInputExt {
	const input = sampleFleet();
	input.myAccess = undefined;
	if (scope) {
		const lab = input.fleet[ID.lab];
		lab.policy = { version: 3, myGrant: labGrant(scope) };
	}
	return input;
}

describe("fleet scope (N1)", () => {
	test("golden sample: status from 3 of 5 active devices, 2 locked", () => {
		const result = coverage(sampleFleet());
		expect(result).toEqual({
			total: 5,
			readable: 3,
			live: 2,
			snapshot: 1,
			unknown: [ID.lab, ID.cold],
			locked: [ID.lab, ID.cold],
			noKeys: [],
			noAccess: [],
			partial: [ID.lab],
			deployed: [],
			notDeployed: [],
		});
		expectConsistent(result);
	});

	test("revoked devices never count", () => {
		const result = coverage(sampleFleet());
		for (const id of [ID.oldKiosk, ID.partner])
			for (const list of [
				result.unknown,
				result.locked,
				result.partial,
				result.noAccess,
			])
				expect(list).not.toContain(id);
	});

	test("no keys on this computer is its own reason", () => {
		const input = sampleFleet();
		input.keys = input.keys.filter((entry) => entry.deviceId !== ID.cold);
		input.local.vaults = input.local.vaults.filter(
			(entry) => entry.deviceId !== ID.cold,
		);
		const result = coverage(input);
		expect(result.noKeys).toEqual([ID.cold]);
		expect(result.locked).toEqual([ID.lab]);
		expectConsistent(result);
	});

	test("unlocking a device makes it readable", () => {
		const input = sampleFleet();
		const lab = input.keys.find((entry) => entry.deviceId === ID.lab);
		if (!lab) throw new Error("fixture: lab keys");
		lab.state = "unlocked";
		input.fleet[ID.lab].status = {
			observations: [],
			observedAt: input.now - 30,
			bootId: null,
			sequence: 1,
		};
		const result = coverage(input);
		expect(result.readable).toBe(4);
		expect(result.snapshot).toBe(2);
		expect(result.locked).toEqual([ID.cold]);
		expectConsistent(result);
	});

	test("an empty fleet covers nothing", () => {
		const input = sampleFleet();
		input.devices = [];
		const result = coverage(input);
		expect(result.total).toBe(0);
		expectConsistent(result);
	});
});

describe("app scope (N4, App › Devices)", () => {
	test("invoice AI: the shared lab device is covered but locked", () => {
		const result = coverage(sampleFleet(), APP.invoiceAi);
		expect(result).toMatchObject({
			total: 5,
			readable: 3,
			unknown: [ID.lab, ID.cold],
			locked: [ID.lab, ID.cold],
			noAccess: [],
			partial: [ID.lab],
			deployed: [ID.edge],
			notDeployed: [ID.warehouse, ID.studio],
		});
		expectConsistent(result, APP.invoiceAi);
	});

	test("an app outside the shared grant is 'no access', not unknown", () => {
		const result = coverage(sampleFleet(), APP.supportPortal);
		expect(result.noAccess).toEqual([ID.lab]);
		expect(result.unknown).toEqual([ID.cold]);
		expect(result.partial).toEqual([]);
		expect(result.deployed).toEqual([ID.edge]);
		expectConsistent(result, APP.supportPortal);
	});

	test("deployed per app follows the readable service lists", () => {
		const input = sampleFleet();
		expect(coverage(input, APP.warehouseScanner).deployed).toEqual([
			ID.warehouse,
		]);
		expect(coverage(input, APP.fieldNotes).deployed).toEqual([ID.studio]);
		expect(coverage(input, APP.partnerReports).deployed).toEqual([]);
	});

	test("without BG22 the verified policy grant decides; without either, nothing is assumed", () => {
		const project = coverage(
			labFromPolicy({ kind: "project", project_id: APP.invoiceAi }),
			APP.supportPortal,
		);
		expect(project.noAccess).toEqual([ID.lab]);
		const placement = coverage(
			labFromPolicy({
				kind: "placement",
				project_id: APP.supportPortal,
				placement_id: "support-bot",
			}),
			APP.supportPortal,
		);
		expect(placement.noAccess).toEqual([]);
		expect(placement.partial).toEqual([ID.lab]);
		const device = coverage(labFromPolicy({ kind: "device" }), APP.crmSync);
		expect(device.noAccess).toEqual([]);
		expect(device.partial).toEqual([]);
		const unknownScope = coverage(labFromPolicy(), APP.crmSync);
		expect(unknownScope.noAccess).toEqual([]);
		expect(unknownScope.unknown).toContain(ID.lab);
	});

	test("an older hub never claims 'no access' it can't know", () => {
		const input = sampleFleetOlderHub();
		const result = coverage(input, APP.supportPortal);
		expect(result.noAccess).toEqual([]);
		expect(result.partial).toEqual([]);
		expectConsistent(result, APP.supportPortal);
	});
});

describe("generated fleets (SPEC §7.3)", () => {
	test("counts stay consistent for 43 and 200 devices", () => {
		for (const size of [43, 200]) {
			const { input, kinds } = generateFleet(size);
			const result = coverage(input);
			const revoked = [...kinds.values()].filter(
				(kind) => kind === "revoked",
			).length;
			expect(result.total).toBe(size - revoked - 2);
			expectConsistent(result);
			expectConsistent(coverage(input, APP.supportPortal), APP.supportPortal);
		}
	});
});
