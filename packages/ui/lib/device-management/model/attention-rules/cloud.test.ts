import { describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	SAMPLE_NOW,
	resourceSummaryOf,
	sampleFleet,
} from "../__fixtures__/sample-fleet";
import type { AttentionInputExt } from "../attention";
import type { AttentionKey } from "../types";
import {
	CLOUD_RULES,
	cloudFacts,
	consentResources,
	revokedApprovalServices,
} from "./cloud";

const evaluate = (key: AttentionKey, input: AttentionInputExt) =>
	CLOUD_RULES.find((rule) => rule.key === key)?.evaluate(input) ?? [];

function edgeResources(input: AttentionInputExt) {
	const resources = input.resources[SAMPLE_IDS.edge];
	if (!resources) throw new Error("fixture: edge resources");
	return resources;
}

const resummarize = (input: AttentionInputExt): AttentionInputExt => ({
	...input,
	resourceSummary: resourceSummaryOf(input.resources, input.me, input.now),
});

describe("approvals", () => {
	test("FG4 summary and per-device lists normalise the same way", () => {
		const input = sampleFleet();
		const fromSummary = cloudFacts(input);
		const fromLists = cloudFacts({ ...input, resourceSummary: undefined });
		expect(fromSummary.approvals.map((entry) => entry.grantId).sort()).toEqual(
			fromLists.approvals.map((entry) => entry.grantId).sort(),
		);
		expect(fromSummary.billing.length).toBe(fromLists.billing.length);
	});

	test("a service bound to a revoked approval is Critical", () => {
		const input = sampleFleet();
		expect(evaluate("cloud_access_invalid", input)).toEqual([]);
		edgeResources(input).grants[0].status = "revoked";
		const [item] = evaluate("cloud_access_invalid", resummarize(input));
		expect(item.severity).toBe("critical");
		expect(item.copy.params).toMatchObject({
			service: "invoice-extractor",
			reason: "revoked",
		});
		expect(
			revokedApprovalServices(resummarize(input)).has(
				`${SAMPLE_IDS.edge}/invoice-extractor`,
			),
		).toBe(true);
	});

	test("expired approvals with no bound grant id still invalidate a running service", () => {
		const input = sampleFleet();
		edgeResources(input).grants[0].expires_at = SAMPLE_NOW - 1;
		const placements = input.live[SAMPLE_IDS.edge].placements;
		if (!placements) throw new Error("fixture");
		placements["invoice-extractor"] = { host: "127.0.0.1", port: 8_081 };
		const [item] = evaluate("cloud_access_invalid", resummarize(input));
		expect(item.copy.params?.reason).toBe("expired");
	});

	test("ending within 7 days is a Warning for the approver or owner", () => {
		const input = sampleFleet();
		edgeResources(input).grants[0].expires_at = SAMPLE_NOW + 2 * 86_400;
		const [item] = evaluate("cloud_access_ending", resummarize(input));
		expect(item.severity).toBe("warning");
		expect(item.subject).toMatchObject({
			deviceId: SAMPLE_IDS.edge,
			serviceId: "invoice-extractor",
			projectId: "app_invoice_ai",
		});
	});

	test("read-only online files need BG33 on the summary", () => {
		const input = sampleFleet();
		expect(evaluate("online_files_read_only", input)).toEqual([]);
		const approval = input.resourceSummary?.devices.find(
			(device) => device.device_id === SAMPLE_IDS.edge,
		)?.approvals[0];
		if (!approval) throw new Error("fixture");
		approval.online_write_blocked = "storage_full";
		expect(evaluate("online_files_read_only", { ...input })[0]?.severity).toBe(
			"warning",
		);
	});
});

describe("spending limits", () => {
	test("80 % used is a Warning, 100 % Critical", () => {
		const input = sampleFleet();
		expect(evaluate("spending_limit_low", input)).toEqual([]);
		edgeResources(input).billing[0].used_micros = 20_000_000;
		expect(
			evaluate("spending_limit_low", resummarize(input))[0]?.severity,
		).toBe("warning");
		edgeResources(input).billing[0].used_micros = 25_000_000;
		expect(
			evaluate("spending_limit_low", resummarize(input))[0]?.severity,
		).toBe("critical");
	});

	test("ending within 7 days is a Notice for the payer", () => {
		const input = sampleFleet();
		edgeResources(input).billing[0].expires_at = SAMPLE_NOW + 86_400;
		expect(
			evaluate("spending_limit_ending", resummarize(input))[0]?.severity,
		).toBe("notice");
		edgeResources(input).billing[0].payer_id = "someone-else";
		expect(evaluate("spending_limit_ending", resummarize(input))).toEqual([]);
	});

	test("consent on a revoked device: billing first, then approvals", () => {
		const input = sampleFleet();
		expect(
			consentResources(input, SAMPLE_IDS.partner)?.billing?.limitMicros,
		).toBe(50_000_000);
		expect(consentResources(input, SAMPLE_IDS.oldKiosk)).toBeUndefined();
	});
});
