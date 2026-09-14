import { describe, expect, test } from "bun:test";
import { apiResponseError, isUpgradeRequiredError } from "./api-error";
import {
	formatQuota,
	quotaBelongsToAnotherPayer,
	quotaWarningKey,
	supportsHostedModel,
	type QuotaOverview,
	type QuotaResource,
} from "./quota";

describe("plan quota UI contract", () => {
	test("keeps structured quota and reset information on API errors", () => {
		const quota = {
			resource: "hosted_ai_cost_micros",
			scope: "account",
			payerId: "payer",
			plan: "FREE",
			used: 800000,
			reserved: 200000,
			requested: 10000,
			limit: 1000000,
			unit: "eur_micros",
			periodEnd: "2026-10-01T00:00:00Z",
			actions: ["upgrade", "use_own_model"],
		};
		const error = apiResponseError(
			{ status: 402, headers: new Headers(), statusText: "Payment Required" },
			JSON.stringify({
				error: {
					code: "PLAN_LIMIT_EXCEEDED",
					message: "Your Free AI allowance is full",
					quota,
				},
			}),
		);
		expect(error.quota).toEqual(quota);
		expect(error.serverMessage).toBe("Your Free AI allowance is full");
		expect(isUpgradeRequiredError(error)).toBe(true);
		expect(quotaBelongsToAnotherPayer(error.quota, "payer")).toBe(false);
		expect(quotaBelongsToAnotherPayer(error.quota, "collaborator")).toBe(true);
		expect(quotaBelongsToAnotherPayer(error.quota, undefined)).toBe(true);
		expect(quotaBelongsToAnotherPayer(undefined, "payer")).toBe(false);
	});
	test("does not turn ordinary capacity or permission errors into upgrades", () => {
		for (const status of [403, 423, 429, 503]) {
			const error = apiResponseError(
				{ status, headers: new Headers(), statusText: "Unavailable" },
				JSON.stringify({
					error: { code: "CAPACITY_UNAVAILABLE", message: "Try again later" },
				}),
			);
			expect(isUpgradeRequiredError(error)).toBe(false);
		}
	});
	test("uses decimal storage and EUR millionths", () => {
		expect(formatQuota(25_000_000_000, "storage_bytes")).toBe("25 GB");
		expect(formatQuota(72_000_000, "cloud_runtime_ms")).toBe("20 h");
		expect(formatQuota(1_000_000, "hosted_ai_cost_micros")).toContain("1");
		expect(formatQuota(-1, "hosted_ai_calls")).toBe("Unlimited");
	});
	test("a marketplace purchase does not suggest upgrading a subscription", () => {
		const error = apiResponseError(
			{ status: 402, headers: new Headers(), statusText: "Payment Required" },
			JSON.stringify({error: {code: "PURCHASE_REQUIRED", message: "Purchase required to download this package"}}),
		);
		expect(isUpgradeRequiredError(error)).toBe(false);
	});
	test("warning deduplication separates payer, allowance period and threshold", () => {
		const overview = {
			payerId: "payer-a",
			periodStart: "2026-09-01",
		} as QuotaOverview;
		const resource = { resource: "cloud_runtime_ms" } as QuotaResource;
		expect(quotaWarningKey(overview, resource, 75)).not.toBe(
			quotaWarningKey({ ...overview, payerId: "payer-b" }, resource, 75),
		);
		expect(quotaWarningKey(overview, resource, 75)).not.toBe(
			quotaWarningKey({ ...overview, periodStart: "2026-10-01" }, resource, 75),
		);
		expect(quotaWarningKey(overview, resource, 75)).not.toBe(
			quotaWarningKey(overview, resource, 90),
		);
	});
});

test("CSV exports quote fields and neutralize spreadsheet formulas", async () => {
	const { usageCsv } = await import("./quota");
	const csv = usageCsv([
		{
			day: "2026-09-13",
			appId: '=HYPERLINK("malicious")',
			modelId: "model,with,commas",
			provider: "provider",
			fundingClass: "hosted",
			executionMode: "realtime",
			runtimeMs: 1500,
			aiCostMicros: 250000,
			aiCalls: 1,
			cloudStarts: 1,
		},
	]);
	expect(csv).toContain('"\'=HYPERLINK(""malicious"")"');
	expect(csv).toContain('"model,with,commas"');
	expect(csv).toContain('"0.25"');
});

test("hosted model upgrades offer only plans that include the required model tier", () => {
	const tiers = {
		PREMIUM: { llm_tiers: ["PREMIUM", "FREE"] },
		PRO: { llm_tiers: ["PRO", "PREMIUM", "FREE"] },
		MAX: { llm_tiers: ["MAX", "PRO", "PREMIUM", "FREE"] },
	};
	expect(
		Object.keys(tiers).filter((key) =>
			supportsHostedModel(tiers[key as keyof typeof tiers], "PRO"),
		),
	).toEqual(["PRO", "MAX"]);
	expect(
		Object.values(tiers).some((tier) =>
			supportsHostedModel(tier, "ENTERPRISE"),
		),
	).toBe(false);
	expect(supportsHostedModel(tiers.PREMIUM)).toBe(true);
});
