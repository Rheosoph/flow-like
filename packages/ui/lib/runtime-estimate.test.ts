import { describe, expect, test } from "bun:test";
import type { DailyQuotaUsage, QuotaResource } from "./quota";
import {
	availableRuntimeCapacity,
	estimateCloudRuns,
	formatRuntimeSeconds,
	getRuntimeSample,
} from "./runtime-estimate";

const now = new Date("2026-09-14T00:30:00Z");
const row = (values: Partial<DailyQuotaUsage> = {}): DailyQuotaUsage => ({
	day: "2026-09-13",
	appId: "example-app",
	modelId: null,
	provider: null,
	fundingClass: "cloud",
	executionMode: "realtime",
	runtimeMs: 50_000,
	cloudStarts: 10,
	aiCalls: 0,
	aiCostMicros: 0,
	...values,
});
const resource = (values: Partial<QuotaResource> = {}): QuotaResource => ({
	resource: "cloud_runtime_ms",
	used: 40_000,
	reserved: 20_000,
	limit: 100_000,
	remaining: 99_999,
	threshold: 0,
	unit: "milliseconds",
	...values,
});

describe("observed workflow runtime", () => {
	test("weights settled runtime by starts and includes runtime-only corrections", () => {
		expect(
			getRuntimeSample(
				{
					usage: [
						row({ runtimeMs: 20_000, cloudStarts: 2 }),
						row({
							day: "2026-09-12",
							runtimeMs: 8_000,
							cloudStarts: 8,
							executionMode: "async",
						}),
						row({ runtimeMs: 2_000, cloudStarts: 0 }),
					],
				},
				now,
			),
		).toEqual({ runtimeMs: 30_000, cloudStarts: 10, averageRuntimeMs: 3_000 });
	});

	test("uses only the previous 30 complete UTC days and workflow dimensions", () => {
		const sample = getRuntimeSample(
			{
				usage: [
					row({ day: "2026-08-15" }),
					row({ fundingClass: "hosted", executionMode: "hosted_ai" }),
					row({ fundingClass: "byok", executionMode: "cloud_assistant" }),
					row({ executionMode: "local" }),
					row({ executionMode: null }),
					row({ appId: null }),
					row({ appId: "" }),
					row({ day: "2026-09-14" }),
					row({ day: "2026-09-15" }),
					row({ day: "2026-08-14" }),
					row({ day: "not-a-date" }),
				],
			},
			now,
		);
		expect(sample).toEqual({
			runtimeMs: 50_000,
			cloudStarts: 10,
			averageRuntimeMs: 5_000,
		});
	});

	test("does not infer an average from partial, sparse, zero or missing data", () => {
		expect(getRuntimeSample(undefined, now)).toBeNull();
		expect(getRuntimeSample({ usage: [] }, now)).toBeNull();
		expect(
			getRuntimeSample({ usage: [row()], usageTruncated: true }, now),
		).toBeNull();
		expect(
			getRuntimeSample({ usage: [row({ cloudStarts: 9 })] }, now),
		).toBeNull();
		expect(
			getRuntimeSample({ usage: [row({ runtimeMs: 0 })] }, now),
		).toBeNull();
		expect(
			getRuntimeSample({ usage: [row()] }, new Date("invalid")),
		).toBeNull();
		expect(
			getRuntimeSample(
				{ usage: [row({ day: "2026-02-30" })] },
				new Date("2026-03-10T12:00:00Z"),
			),
		).toBeNull();
	});

	test("invalid matching values suppress the sample rather than biasing the average", () => {
		for (const invalid of [-1, Number.NaN, Number.POSITIVE_INFINITY]) {
			expect(
				getRuntimeSample({ usage: [row(), row({ runtimeMs: invalid })] }, now),
			).toBeNull();
			expect(
				getRuntimeSample(
					{ usage: [row(), row({ cloudStarts: invalid })] },
					now,
				),
			).toBeNull();
		}
		expect(
			getRuntimeSample(
				{
					usage: [
						row(),
						row({ fundingClass: "hosted", runtimeMs: Number.NaN }),
					],
				},
				now,
			)?.averageRuntimeMs,
		).toBe(5_000);
		expect(
			getRuntimeSample(
				{
					usage: [
						row({ runtimeMs: Number.MAX_VALUE }),
						row({ runtimeMs: Number.MAX_VALUE }),
					],
				},
				now,
			),
		).toBeNull();
	});
});

describe("estimated cloud runs", () => {
	test("floors runtime capacity and respects the cloud-start ceiling", () => {
		expect(estimateCloudRuns(72_000_000, 10_000, 5_000)).toEqual({
			runs: 10_000,
			runtimeRuns: 14_400,
			startLimited: true,
		});
		expect(estimateCloudRuns(10_001, 100, 3_000)).toEqual({
			runs: 3,
			runtimeRuns: 3,
			startLimited: false,
		});
		expect(estimateCloudRuns(30_000, 10, 3_000)?.startLimited).toBe(false);
		expect(estimateCloudRuns(0, 10, 1_000)?.runs).toBe(0);
		expect(estimateCloudRuns(30_000, 0, 1_000)?.runs).toBe(0);
	});

	test("keeps explicitly unlimited, missing and invalid allowances distinct", () => {
		expect(estimateCloudRuns(-1, -1, 1_000)).toEqual({
			runs: null,
			runtimeRuns: null,
			startLimited: false,
		});
		expect(estimateCloudRuns(-1, 10, 1_000)).toEqual({
			runs: 10,
			runtimeRuns: null,
			startLimited: true,
		});
		expect(estimateCloudRuns(10_000, -1, 1_000)).toEqual({
			runs: 10,
			runtimeRuns: 10,
			startLimited: false,
		});
		expect(estimateCloudRuns(undefined, 10, 1_000)).toBeNull();
		expect(estimateCloudRuns(10_000, undefined, 1_000)).toBeNull();
		for (const average of [0, -1, Number.NaN, Number.POSITIVE_INFINITY])
			expect(estimateCloudRuns(10_000, 10, average)).toBeNull();
		expect(estimateCloudRuns(Number.POSITIVE_INFINITY, 10, 1_000)).toBeNull();
		expect(estimateCloudRuns(10_000, Number.NaN, 1_000)).toBeNull();
		expect(
			estimateCloudRuns(Number.MAX_VALUE, -1, Number.MIN_VALUE),
		).toBeNull();
	});

	test("deducts settled and pending capacity before estimating remaining runs", () => {
		const runtime = availableRuntimeCapacity(resource());
		const starts = availableRuntimeCapacity(
			resource({ resource: "cloud_starts", limit: 10, used: 2, reserved: 3 }),
		);
		expect(runtime).toBe(40_000);
		expect(starts).toBe(5);
		expect(estimateCloudRuns(runtime, starts, 5_000)).toEqual({
			runs: 5,
			runtimeRuns: 8,
			startLimited: true,
		});
		expect(
			availableRuntimeCapacity(resource({ used: 90_000, reserved: 20_000 })),
		).toBe(0);
		expect(availableRuntimeCapacity(resource({ limit: 0 }))).toBe(0);
		expect(availableRuntimeCapacity(resource({ limit: -1 }))).toBe(-1);
		expect(availableRuntimeCapacity(undefined)).toBeUndefined();
		expect(availableRuntimeCapacity(resource({ used: -1 }))).toBeUndefined();
		expect(
			availableRuntimeCapacity(resource({ reserved: Number.NaN })),
		).toBeUndefined();
		expect(
			availableRuntimeCapacity(resource({ limit: Number.POSITIVE_INFINITY })),
		).toBeUndefined();
	});

	test("formats duration in seconds without hiding short executions", () => {
		expect(formatRuntimeSeconds(5_000)).toBe((5).toLocaleString());
		expect(formatRuntimeSeconds(1)).toBe(
			(0.001).toLocaleString(undefined, { maximumSignificantDigits: 3 }),
		);
		expect(formatRuntimeSeconds(1_234)).toBe((1.23).toLocaleString());
	});
});
