import { describe, expect, test } from "bun:test";
import type { ResourceSummary } from "../../../../lib/device-management/model/types";
import type {
	BillingGrant,
	DeviceResources,
	ResourceGrant,
} from "../../../../lib/device-resources";
import {
	type CloudApproval,
	approvalsOfDevice,
	approvalsOfSummary,
	byState,
	currentApproval,
	daysUntil,
	endOf,
	mergeApprovals,
	spendTotals,
} from "./cloud-model";

const NOW = 1_790_769_600;
const DAY = 86_400;
const ME = "usr_me";
const DEVICE = "5b794764-6afc-4ac9-89c2-c6d9eb91fd42";

const grant = (patch: Partial<ResourceGrant> = {}): ResourceGrant => ({
	grant_id: "grant-1",
	device_id: DEVICE,
	placement_id: "invoice-extractor",
	deployment_id: "deployment-1",
	project_id: "app_invoice_ai",
	app_id: "app_invoice_ai",
	delegating_user_id: ME,
	authz_version: 1,
	model_ids: ["gpt-4.1-mini"],
	online_access: "read_write",
	max_instances: 2,
	expires_at: NOW + 30 * DAY,
	status: "active",
	...patch,
});

const billing = (patch: Partial<BillingGrant> = {}): BillingGrant => ({
	billing_grant_id: "limit-1",
	grant_id: "grant-1",
	payer_id: ME,
	authz_version: 1,
	limit_micros: 25_000_000,
	used_micros: 7_412_300,
	reserved_micros: 120_000,
	expires_at: NOW + 30 * DAY,
	status: "active",
	...patch,
});

const resources = (patch: Partial<DeviceResources> = {}): DeviceResources => ({
	grants: [grant()],
	billing: [billing()],
	instances: [],
	...patch,
});

type SummaryRow = ResourceSummary["devices"][number]["approvals"][number];

const summaryRow = (patch: Partial<SummaryRow> = {}): SummaryRow => ({
	grant_id: "grant-1",
	placement_id: "invoice-extractor",
	app_id: "app_invoice_ai",
	status: "active",
	expires_at: NOW + 30 * DAY,
	effective_expires_at: NOW + 30 * DAY,
	effective_limit: "approval",
	online_access: "read_write",
	online_write_blocked: null,
	payer_is_me: true,
	approver_is_me: true,
	...patch,
});

const summary = (
	approvals: SummaryRow[],
	limits: ResourceSummary["devices"][number]["billing"] = [],
): ResourceSummary => ({
	server_time: NOW,
	devices: [{ device_id: DEVICE, approvals, billing: limits }],
});

const only = <T>(rows: T[]): T => {
	const [row] = rows;
	if (rows.length !== 1 || !row) throw new Error("exactly one row expected");
	return row;
};

describe("approvalsOfDevice", () => {
	test("an active approval carries its details, limit and leases", () => {
		const lease = {
			instance_id: "instance-1",
			purpose: "workload" as const,
			device_id: DEVICE,
			grant_id: "grant-1",
			billing_grant_id: "limit-1",
			registered_at: NOW - 30,
			lease_expires_at: NOW + 570,
		};
		const row = only(
			approvalsOfDevice(
				resources({
					grants: [
						grant({
							effective_expires_at: NOW + 5 * DAY,
							effective_limit: "access_rules",
							approved_by_user_id: "usr_other",
							created_at: NOW - DAY,
						}),
					],
					instances: [lease, { ...lease, grant_id: "grant-2" }],
				}),
				ME,
				NOW,
			),
		);
		expect(row.state).toBe("active");
		expect(row.revocable).toBe(true);
		expect(row.effective).toEqual({ at: NOW + 5 * DAY, limit: "access_rules" });
		expect(endOf(row)).toBe(NOW + 5 * DAY);
		expect(row.details).toEqual({
			models: ["gpt-4.1-mini"],
			maxInstances: 2,
			version: 1,
			projectId: "app_invoice_ai",
			deploymentId: "deployment-1",
			approvedBy: "usr_other",
			approvedAt: NOW - DAY,
		});
		expect(row.limit?.state).toBe("active");
		expect(row.limit?.payerIsMe).toBe(true);
		expect(row.leases).toEqual([lease]);
		expect(row.approverIsMe).toBe(true);
	});

	test("an older hub sends no effective end: the approved date decides", () => {
		const row = only(approvalsOfDevice(resources(), ME, NOW));
		expect(row.effective).toBeUndefined();
		expect(endOf(row)).toBe(NOW + 30 * DAY);
		expect(row.details?.approvedBy).toBe(ME);
		expect(row.details?.approvedAt).toBeUndefined();
	});

	test("an effective end in the past means it has ended, whatever the approved date says", () => {
		const row = only(
			approvalsOfDevice(
				resources({
					grants: [
						grant({
							effective_expires_at: NOW - 60,
							effective_limit: "sharing_grant",
						}),
					],
				}),
				ME,
				NOW,
			),
		);
		expect(row.state).toBe("expired");
		// The hub still holds it for the service, so it can (and must) be revoked before a new one.
		expect(row.revocable).toBe(true);
	});

	test("past its approved date an approval is over for good: nothing is left to revoke", () => {
		const row = only(
			approvalsOfDevice(
				resources({ grants: [grant({ expires_at: NOW - 60 })], billing: [] }),
				ME,
				NOW,
			),
		);
		expect(row.state).toBe("expired");
		expect(row.revocable).toBe(false);
		const listed = only(
			approvalsOfSummary(
				summary([
					summaryRow({
						expires_at: NOW - 60,
						effective_expires_at: NOW - 60,
					}),
				]),
				NOW,
			),
		);
		expect(listed.revocable).toBe(false);
	});

	test("a revoked approval ignores the effective fields; storage-full is true only when said", () => {
		const [revoked, blocked] = approvalsOfDevice(
			resources({
				grants: [
					grant({ status: "revoked", effective_expires_at: NOW + DAY }),
					grant({ grant_id: "grant-2", online_write_blocked: "storage_full" }),
				],
				billing: [],
			}),
			ME,
			NOW,
		);
		expect(revoked?.state).toBe("revoked");
		expect(revoked?.revocable).toBe(false);
		expect(revoked?.effective).toBeUndefined();
		expect(revoked?.writeBlocked).toBe(false);
		expect(blocked?.writeBlocked).toBe(true);
	});

	test("the limit shown is the running one, else the first listed; a run-out limit is expired", () => {
		const row = only(
			approvalsOfDevice(
				resources({
					billing: [
						billing({ billing_grant_id: "old", status: "revoked" }),
						billing({ billing_grant_id: "new", payer_id: "usr_other" }),
					],
				}),
				ME,
				NOW,
			),
		);
		expect(row.limit?.id).toBe("new");
		expect(row.limit?.payerIsMe).toBe(false);

		const lapsed = only(
			approvalsOfDevice(
				resources({ billing: [billing({ expires_at: NOW - 1 })] }),
				ME,
				NOW,
			),
		);
		expect(lapsed.limit?.state).toBe("expired");
	});
});

describe("approvalsOfSummary and mergeApprovals", () => {
	test("a fleet row has no details; the device's own list completes it", () => {
		const listed = approvalsOfSummary(
			summary(
				[summaryRow({ online_write_blocked: "storage_full" })],
				[
					{
						billing_grant_id: "limit-1",
						grant_id: "grant-1",
						limit_micros: 25_000_000,
						used_micros: 1_000_000,
						reserved_micros: 0,
						expires_at: NOW + 30 * DAY,
						payer_is_me: true,
					},
				],
			),
			NOW,
		);
		const fleet = only(listed);
		expect(fleet.details).toBeUndefined();
		expect(fleet.leases).toBeUndefined();
		expect(fleet.limit?.payerId).toBeUndefined();

		const merged = only(
			mergeApprovals(listed, approvalsOfDevice(resources(), ME, NOW)),
		);
		expect(merged.details?.models).toEqual(["gpt-4.1-mini"]);
		expect(merged.limit?.used).toBe(7_412_300);
		expect(merged.limit?.payerId).toBe(ME);
		expect(merged.writeBlocked).toBe(true);
	});

	test("a revoked fleet row never reads as running, even with parser defaults", () => {
		const row = only(
			approvalsOfSummary(
				summary([
					summaryRow({
						status: "revoked",
						effective_expires_at: NOW + 30 * DAY,
					}),
				]),
				NOW,
			),
		);
		expect(row.state).toBe("revoked");
		expect(row.effective).toBeUndefined();
	});

	test("approvals only a device's list knows are appended after the fleet rows", () => {
		const fleet = approvalsOfSummary(summary([summaryRow()]), NOW);
		const own = approvalsOfDevice(
			resources({
				grants: [
					grant(),
					grant({ grant_id: "grant-9", placement_id: "other" }),
				],
			}),
			ME,
			NOW,
		);
		expect(mergeApprovals(fleet, own).map((row) => row.grantId)).toEqual([
			"grant-1",
			"grant-9",
		]);
	});
});

describe("ordering, the service's approval and the sum", () => {
	const row = (patch: Partial<CloudApproval>): CloudApproval => ({
		deviceId: DEVICE,
		serviceId: "svc",
		grantId: "g",
		appId: null,
		state: "active",
		revocable: true,
		expiresAt: NOW + DAY,
		files: "none",
		writeBlocked: false,
		approverIsMe: true,
		...patch,
	});

	test("running approvals first, the hub's order inside a state", () => {
		const rows = [
			row({ grantId: "a", state: "revoked" }),
			row({ grantId: "b", state: "active" }),
			row({ grantId: "c", state: "expired" }),
			row({ grantId: "d", state: "active" }),
		];
		expect(byState(rows).map((entry) => entry.grantId)).toEqual([
			"b",
			"d",
			"c",
			"a",
		]);
	});

	test("a service runs on its active approval, else the one that ended last", () => {
		const ended = { revocable: false };
		const rows = [
			row({
				...ended,
				grantId: "old",
				state: "revoked",
				expiresAt: NOW - 9 * DAY,
			}),
			row({
				...ended,
				grantId: "newer",
				state: "expired",
				expiresAt: NOW - DAY,
			}),
			row({ grantId: "elsewhere", serviceId: "other" }),
		];
		expect(currentApproval(rows, "svc")?.grantId).toBe("newer");
		expect(
			currentApproval([...rows, row({ grantId: "live" })], "svc")?.grantId,
		).toBe("live");
		expect(currentApproval(rows, "none")).toBeUndefined();
	});

	test("an approval that ended early is still held by the hub: it comes before a revoked one with a later date", () => {
		const rows = [
			// Revoked last month; its approved date is still ahead.
			row({
				grantId: "revoked",
				state: "revoked",
				revocable: false,
				expiresAt: NOW + 20 * DAY,
			}),
			// Ended ten minutes ago with the access rules; not revoked, so it blocks a new approval.
			row({
				grantId: "held",
				state: "expired",
				revocable: true,
				expiresAt: NOW + 29 * DAY,
				effective: { at: NOW - 600, limit: "access_rules" },
			}),
		];
		expect(currentApproval(rows, "svc")?.grantId).toBe("held");
	});

	test("the sum counts only limits the viewer pays for that still accept charges", () => {
		const limit = {
			id: "l",
			grantId: "g",
			limit: 25_000_000,
			used: 7_000_000,
			reserved: 100_000,
			expiresAt: NOW + DAY,
			state: "active" as const,
			payerIsMe: true,
		};
		expect(
			spendTotals([
				row({ limit }),
				row({
					limit: { ...limit, limit: 50_000_000, used: 12_500_000, reserved: 0 },
				}),
				row({ limit: { ...limit, payerIsMe: false } }),
				row({ limit: { ...limit, state: "revoked" } }),
				row({}),
			]),
		).toEqual({
			count: 2,
			used: 19_500_000,
			reserved: 100_000,
			limit: 75_000_000,
		});
	});

	test("days until an end are whole and at least one", () => {
		expect(daysUntil(NOW + 30 * DAY, NOW)).toBe(30);
		expect(daysUntil(NOW + 10, NOW)).toBe(1);
		expect(daysUntil(NOW - DAY, NOW)).toBe(1);
	});
});
