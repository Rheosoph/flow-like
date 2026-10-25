import assert from "node:assert/strict";
import test from "node:test";
import { percentile, summarize } from "./measurement.mjs";
import { requestCategory } from "./app-measurement.mjs";

test("reports interpolated percentiles without changing raw samples", () => {
	const values = [100, 10, 20, 30];
	assert.equal(percentile(values, 0.5), 25);
	assert.equal(percentile(values, 0.95), 89.49999999999997);
	assert.deepEqual(values, [100, 10, 20, 30]);
	assert.equal(percentile([], 0.5), null);
});

test("network categories discard app IDs, route queries, and credentials", () => {
	assert.equal(
		requestCategory(
			"http://127.0.0.1:8440/api/v1/apps/private-app/pages/bootstrap?token=secret&route=/people",
			"GET",
		),
		"bootstrap",
	);
	assert.equal(
		requestCategory(
			"http://127.0.0.1:8440/api/v1/apps/private-app/events/private-event/invoke",
			"OPTIONS",
		),
		"invoke_preflight",
	);
	assert.equal(
		requestCategory("http://127.0.0.1:3440/_next/static/app.js", "GET"),
		null,
	);
	assert.equal(requestCategory("invalid", "GET"), null);
});

test("keeps failed attempts visible without counting their partial timings as successes", () => {
	const report = summarize([
		{ status: "ok", clickToHistoryDomMs: 200 },
		{ status: "error", clickToHistoryDomMs: 20_000, error: "timeout" },
		{ status: "ok", clickToHistoryDomMs: 300 },
	]);
	assert.equal(report.attempts, 3);
	assert.equal(report.failed, 1);
	assert.equal(report.metrics.clickToHistoryDomMs.p50, 250);
	assert.equal(report.metrics.clickToHistoryFrameOpportunityMs.count, 0);
});
