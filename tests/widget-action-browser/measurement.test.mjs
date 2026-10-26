import assert from "node:assert/strict";
import test from "node:test";
import { percentile, requestRouteKey, summarize } from "./measurement.mjs";
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

test("route keys group only exact method and pathname while omitting private URL content", () => {
	const url =
		"https://person:secret@localhost/api/v1/apps/private-app/runs/run-one?token=secret";
	const key = requestRouteKey(url, "GET");
	assert.match(key, /^[a-f0-9]{64}$/);
	assert.equal(
		key,
		requestRouteKey(
			"http://127.0.0.1/api/v1/apps/private-app/runs/run-one?different=query",
			"GET",
		),
	);
	assert.notEqual(key, requestRouteKey(url, "POST"));
	assert.notEqual(
		key,
		requestRouteKey(url.replace("run-one", "run-two"), "GET"),
	);
	assert.notEqual(
		key,
		requestRouteKey(url.replace("private-app", "different-app"), "GET"),
	);
	assert.equal(requestRouteKey("invalid", "GET"), null);
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
