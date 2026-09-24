import { describe, expect, test } from "bun:test";
import {
	apiResponseError,
	isPurchaseRequiredError,
	isUpgradeRequiredError,
} from "./api-error";
import {
	type QuotaOverview,
	type QuotaResource,
	formatQuota,
	mergeQuotaCounters,
	quotaBelongsToAnotherPayer,
	quotaWarningKey,
	supportsHostedModel,
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
			JSON.stringify({
				error: {
					code: "PURCHASE_REQUIRED",
					message: "Purchase required to download this package",
				},
			}),
		);
		expect(isUpgradeRequiredError(error)).toBe(false);
	});
	test("a package licence for a project does not suggest upgrading a subscription", () => {
		const error = apiResponseError(
			{ status: 402, headers: new Headers(), statusText: "Payment Required" },
			JSON.stringify({
				error: {
					code: "PACKAGE_LICENSE_REQUIRED",
					message: "Buy this package before adding it to the project",
				},
			}),
		);
		expect(isUpgradeRequiredError(error)).toBe(false);
		expect(isPurchaseRequiredError(error)).toBe(true);
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

describe("counter-only quota refresh", () => {
	const overview: QuotaOverview = {
		plan: "FREE",
		payerId: "payer-a",
		periodStart: "2026-09-01",
		periodEnd: "2026-10-01",
		updatedAt: "2026-09-14T10:00:00Z",
		usageTruncated: true,
		resources: [
			{
				resource: "cloud_runtime_ms",
				used: 1000,
				reserved: 0,
				limit: 10_000,
				remaining: 9000,
				unit: "milliseconds",
				threshold: 0,
			},
		],
		usage: [
			{
				day: "2026-09-14",
				appId: "app-a",
				modelId: "model-a",
				provider: "provider",
				fundingClass: "hosted",
				runtimeMs: 1000,
				aiCostMicros: 200,
				aiCalls: 1,
				cloudStarts: 1,
			},
		],
	};
	const summary: QuotaOverview = {
		...overview,
		plan: "PREMIUM",
		periodStart: "2026-10-01",
		periodEnd: "2026-11-01",
		trackingSince: "2026-09-01",
		updatedAt: "2026-09-14T10:01:00Z",
		usageTruncated: false,
		usage: [],
		resources: [{ ...overview.resources[0], used: 3000, remaining: 7000 }],
	};

	test("refreshes counters and periods without clearing or refreshing history", () => {
		const merged = mergeQuotaCounters(overview, summary);
		expect(merged.resources).toBe(summary.resources);
		expect(merged.plan).toBe("PREMIUM");
		expect(merged.periodStart).toBe(summary.periodStart);
		expect(merged.periodEnd).toBe(summary.periodEnd);
		expect(merged.trackingSince).toBe(summary.trackingSince);
		expect(merged.usage).toBe(overview.usage);
		expect(merged.usageTruncated).toBe(true);
		expect(merged.updatedAt).toBe(overview.updatedAt);
		expect(merged.countersUpdatedAt).toBe(summary.updatedAt);
	});

	test("rejects another payer and older or invalid snapshots", () => {
		expect(
			mergeQuotaCounters(overview, { ...summary, payerId: "payer-b" }),
		).toBe(overview);
		expect(
			mergeQuotaCounters(overview, {
				...summary,
				updatedAt: "2026-09-14T09:59:00Z",
			}),
		).toBe(overview);
		expect(
			mergeQuotaCounters(overview, { ...summary, updatedAt: "invalid" }),
		).toBe(overview);
		const merged = mergeQuotaCounters(overview, summary);
		expect(
			mergeQuotaCounters(merged, {
				...summary,
				updatedAt: "2026-09-14T10:00:30Z",
			}),
		).toBe(merged);
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
