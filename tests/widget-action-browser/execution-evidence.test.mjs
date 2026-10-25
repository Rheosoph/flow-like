import assert from "node:assert/strict";
import test from "node:test";
import { createExecutionParser } from "./execution-evidence.mjs";

test("extracts terminal evidence from chunked SSE without retaining payload data", () => {
	const parser = createExecutionParser();
	const body =
		'event: message\r\ndata: {"event_type":"run_initiated","payload":{"run_id":"run-1"}}\r\n\r\nevent: completed\r\ndata: {"event_type":"completed","payload":{"run_id":"run-1","status":"completed","log_level":0,"customer":"do-not-export"}}\r\n\r\n';
	for (let index = 0; index < body.length; index += 7)
		parser.feed(body.slice(index, index + 7));
	parser.end();
	assert.equal(parser.snapshot().success, true);
	assert.equal(parser.snapshot().runId, "run-1");
	assert.ok(!JSON.stringify(parser.snapshot()).includes("do-not-export"));
});

test("rejects a failed run even when it reaches a completed event", () => {
	for (const failure of [
		{ status: "failed", log_level: 3 },
		{ status: "completed", log_level: 3 },
		{ status: "completed", error: "private-error-detail" },
		{ status: "unknown" },
	]) {
		const parser = createExecutionParser();
		parser.feed(JSON.stringify({ event_type: "completed", payload: failure }));
		parser.end();
		assert.equal(parser.snapshot().terminalObserved, true);
		assert.equal(parser.snapshot().success, false);
		assert.ok(
			!JSON.stringify(parser.snapshot()).includes("private-error-detail"),
		);
	}
});

test("rejects earlier error events and conflicting run IDs in NDJSON", () => {
	const parser = createExecutionParser();
	for (const event of [
		{ event_type: "run_initiated", payload: { run_id: "one" } },
		{ event_type: "error", payload: { message: "private-error" } },
		{
			event_type: "completed",
			payload: { run_id: "two", status: "completed" },
		},
	])
		parser.feed(`${JSON.stringify(event)}\n`);
	assert.equal(parser.snapshot().success, false);
	assert.equal(parser.snapshot().errorEvent, true);
	assert.equal(parser.snapshot().runIdMismatch, true);
});

test("generic UI events cannot establish terminal execution success", () => {
	const parser = createExecutionParser();
	parser.feed(
		'data: {"event_type":"a2ui","payload":{"status":"completed"}}\n\n',
	);
	assert.equal(parser.snapshot().terminalObserved, false);
	assert.equal(parser.snapshot().success, false);
});
