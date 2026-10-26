import assert from "node:assert/strict";
import test from "node:test";
import { benchmarkRequestId } from "./benchmark-request-id.mjs";

const id = "12345678-1234-4abc-8abc-123456789abc";

test("retains the exact proxy UUID regardless of header name casing", () => {
	for (const name of [
		"x-flowlike-benchmark-request-id",
		"X-Flowlike-Benchmark-Request-Id",
	])
		assert.equal(
			benchmarkRequestId({
				[name]: id,
				"set-cookie": "private",
				authorization: "private",
			}),
			id,
		);
	assert.equal(
		benchmarkRequestId({ "x-flowlike-benchmark-request-id": id.toUpperCase() }),
		id.toUpperCase(),
	);
});

test("missing, malformed and non-UUID header contents are omitted", () => {
	for (const value of [
		"private token",
		"",
		` ${id}`,
		`${id}\nprivate`,
		`${id}, ${id}`,
		[id],
		123,
		id.replace("4abc", "0abc"),
		id.replace("8abc", "1abc"),
	])
		assert.equal(
			benchmarkRequestId({ "x-flowlike-benchmark-request-id": value }),
			undefined,
		);
	for (const headers of [
		undefined,
		null,
		[],
		"private",
		{ authorization: "private" },
	])
		assert.equal(benchmarkRequestId(headers), undefined);
});

test("ambiguous duplicate header names cannot authorize a request", () => {
	assert.equal(
		benchmarkRequestId({
			"x-flowlike-benchmark-request-id": id,
			"X-Flowlike-Benchmark-Request-Id": id,
		}),
		undefined,
	);
});
