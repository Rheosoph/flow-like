import { describe, expect, test } from "bun:test";
import type { PlacementStatusPlus } from "../../../../lib/device-management/model/types";
import type {
	BillingGrant,
	DeviceResources,
	ResourceGrant,
} from "../../../../lib/device-resources";
import {
	readyToRemove,
	serviceCloudAccess,
	waitingChanges,
} from "./remove-model";

const NOW = 1_790_769_600;
const ME = "user-me";
const OTHER = "user-other";

function row(patch: Partial<PlacementStatusPlus> = {}): PlacementStatusPlus {
	return {
		id: "field-notes",
		project_id: "app_field_notes",
		deployment_id: "496e5d88-3147-4ec4-84bc-dfce6cc635fc",
		revision: "ef87af1b",
		desired_state: "stopped",
		observed_state: "stopped",
		config_revision: 9,
		intent_revision: 3,
		applied_revision: 9,
		...patch,
	};
}

function grant(patch: Partial<ResourceGrant> = {}): ResourceGrant {
	return {
		grant_id: "grant-1",
		device_id: "device-1",
		placement_id: "field-notes",
		deployment_id: "496e5d88-3147-4ec4-84bc-dfce6cc635fc",
		project_id: "app_field_notes",
		app_id: "app_field_notes",
		delegating_user_id: ME,
		authz_version: 1,
		model_ids: [],
		online_access: "read_write",
		max_instances: 1,
		expires_at: NOW + 86_400,
		status: "active",
		...patch,
	};
}

function limit(patch: Partial<BillingGrant> = {}): BillingGrant {
	return {
		billing_grant_id: "limit-1",
		grant_id: "grant-1",
		payer_id: ME,
		authz_version: 1,
		limit_micros: 25_000_000,
		used_micros: 0,
		reserved_micros: 0,
		expires_at: NOW + 86_400,
		status: "active",
		...patch,
	};
}

const resources = (
	grants: ResourceGrant[],
	billing: BillingGrant[] = [],
): DeviceResources => ({ grants, billing, instances: [] });

const owner = { id: ME, owner: true };
const guest = { id: ME, owner: false };

describe("when the device accepts a removal", () => {
	test("a service that is requested and observed stopped, with no process left", () => {
		expect(readyToRemove(row())).toBe(true);
		expect(readyToRemove(row({ observed_state: "failed" }))).toBe(true);
		expect(readyToRemove(row({ process_id: null, replicas: [] }))).toBe(true);
	});

	test("a stop that was only requested isn't enough: the device still runs it", () => {
		expect(readyToRemove(row({ observed_state: "running" }))).toBe(false);
		expect(readyToRemove(row({ observed_state: "stopping" }))).toBe(false);
		expect(readyToRemove(row({ desired_state: "running" }))).toBe(false);
	});

	test("a process that is still there keeps the service", () => {
		expect(readyToRemove(row({ process_id: 4_211 }))).toBe(false);
		expect(
			readyToRemove(
				row({
					replicas: [
						{
							slot: 0,
							observed_state: "stopped",
							applied_revision: 9,
							process_id: null,
						},
						{
							slot: 1,
							observed_state: "stopping",
							applied_revision: 9,
							process_id: 4_212,
						},
					],
				}),
			),
		).toBe(false);
	});

	test("a service the device no longer lists has nothing left to wait for", () => {
		expect(readyToRemove(undefined)).toBe(true);
	});
});

describe("buffered changes a removal would lose", () => {
	test("an offline copy buffers nothing", () => {
		expect(
			waitingChanges(false, true, { pending: 4, quarantined: false }),
		).toBe(0);
	});

	test("an online service loses what its queues still hold", () => {
		expect(waitingChanges(true, true, { pending: 17, quarantined: true })).toBe(
			17,
		);
		expect(waitingChanges(true, true, { pending: 0, quarantined: false })).toBe(
			0,
		);
	});

	test("queues that weren't read are unknown, not empty, unless buffering is off", () => {
		expect(waitingChanges(true, true, "not_loaded")).toBe("unknown");
		expect(waitingChanges(true, false, "not_loaded")).toBe(0);
		expect(waitingChanges(true, true, undefined)).toBe(0);
	});
});

describe("the cloud access of a service", () => {
	test("isn't known while the hub's list wasn't read", () => {
		expect(serviceCloudAccess(undefined, "field-notes", owner, NOW)).toEqual({
			state: "unknown",
			approvals: [],
			limits: [],
		});
	});

	test("the owner sees every approval, so none listed means none given", () => {
		const other = resources([grant({ placement_id: "support-bot" })]);
		expect(serviceCloudAccess(other, "field-notes", owner, NOW).state).toBe(
			"none",
		);
		expect(serviceCloudAccess(other, "field-notes", guest, NOW).state).toBe(
			"hidden",
		);
	});

	test("revoked and ended approvals don't count as cloud access that stays", () => {
		const ended = resources([
			grant({ status: "revoked" }),
			grant({ grant_id: "grant-2", expires_at: NOW - 1 }),
			grant({
				grant_id: "grant-3",
				expires_at: NOW + 86_400,
				effective_expires_at: NOW - 60,
			}),
		]);
		expect(serviceCloudAccess(ended, "field-notes", owner, NOW).state).toBe(
			"none",
		);
	});

	test("the owner may end every approval; the limits are the ones this account pays", () => {
		const listed = resources(
			[
				grant(),
				grant({ grant_id: "grant-2", delegating_user_id: OTHER }),
				grant({ grant_id: "grant-9", placement_id: "support-bot" }),
			],
			[
				limit(),
				limit({
					billing_grant_id: "limit-2",
					grant_id: "grant-2",
					payer_id: OTHER,
				}),
				limit({ billing_grant_id: "limit-3", status: "revoked" }),
				limit({ billing_grant_id: "limit-9", grant_id: "grant-9" }),
			],
		);
		expect(serviceCloudAccess(listed, "field-notes", owner, NOW)).toEqual({
			state: "listed",
			approvals: ["grant-1", "grant-2"],
			limits: ["limit-1"],
		});
	});

	test("on someone else's device only the approvals this account gave can be ended", () => {
		const listed = resources(
			[grant({ delegating_user_id: OTHER }), grant({ grant_id: "grant-2" })],
			[limit(), limit({ billing_grant_id: "limit-2", grant_id: "grant-2" })],
		);
		expect(serviceCloudAccess(listed, "field-notes", guest, NOW)).toEqual({
			state: "listed",
			approvals: ["grant-2"],
			limits: ["limit-1", "limit-2"],
		});
		const foreign = resources([grant({ delegating_user_id: OTHER })]);
		expect(serviceCloudAccess(foreign, "field-notes", guest, NOW)).toEqual({
			state: "listed",
			approvals: [],
			limits: [],
		});
	});
});
