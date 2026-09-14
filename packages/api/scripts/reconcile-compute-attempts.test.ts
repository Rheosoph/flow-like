import { describe, expect, test } from "bun:test";
import { buildRevision, parseAwsReport } from "./reconcile-compute-attempts";

describe("AWS REPORT reconciliation", () => {
	test("reads plain and JSON platform reports and rejects application messages", () => {
		const plain = parseAwsReport(
			"REPORT RequestId: abc-123\tDuration: 12.51 ms\tBilled Duration: 102 ms\tMemory Size: 2048 MB\tMax Memory Used: 90 MB\tStatus: timeout",
		);
		expect(plain).toEqual({
			requestId: "abc-123",
			durationMs: 12.51,
			billedDurationMs: 102,
			memoryMb: 2048,
			status: "timeout",
		});
		expect(
			parseAwsReport(
				JSON.stringify({
					type: "platform.report",
					record: {
						requestId: "abc-123",
						status: "success",
						metrics: {
							durationMs: 12.51,
							billedDurationMs: 102,
							memorySizeMB: 2048,
						},
					},
				}),
			)?.status,
		).toBe("completed");
		expect(
			parseAwsReport(
				"INFO user said REPORT RequestId: abc-123 Duration: 12 ms Billed Duration: 1 ms Memory Size: 2048 MB",
			),
		).toBeNull();
		expect(
			parseAwsReport(
				JSON.stringify({ type: "function", record: { requestId: "abc-123" } }),
			),
		).toBeNull();
		expect(
			parseAwsReport(
				"REPORT RequestId: abc-123 Duration: 12 ms Billed Duration: 1 ms Memory Size: 0 MB",
			),
		).toBeNull();
	});
	test("keeps public measurements separate and produces stable evidence", () => {
		const attempt = {
			id: "id",
			functionName: "worker",
			requestId: "req",
			operationId: "op",
			payerId: "payer",
			role: "executor",
			costClass: "workflow_compute",
			memoryMb: 2048,
			architecture: "x86_64",
			region: "eu-central-1",
			measuredDurationMs: 10,
			status: "completed",
			startedAt: "2026-09-13T00:00:00Z",
		};
		const report = {
			requestId: "req",
			durationMs: 12.5,
			billedDurationMs: 120,
			memoryMb: 2048,
			status: "completed" as const,
		};
		const revision = buildRevision(attempt, report);
		expect(revision.measuredDurationMs).toBe(10);
		expect(revision.billedDurationMs).toBe(120);
		expect(revision.costMicroUsd).toBeNull();
		expect(revision.evidence).toBe("aws_report_estimate");
		expect(revision.revision).toBe(buildRevision(attempt, report).revision);
		expect(() =>
			buildRevision(attempt, { ...report, requestId: "another" }),
		).toThrow();
		expect(() =>
			buildRevision(attempt, { ...report, memoryMb: 128 }),
		).toThrow();
	});
});
