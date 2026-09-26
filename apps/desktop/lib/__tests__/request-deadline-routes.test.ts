import { describe, expect, test } from "vitest";
import {
	DATA_REQUEST_TIMEOUT_MS,
	DISPATCH_REQUEST_TIMEOUT_MS,
	JOB_REQUEST_TIMEOUT_MS,
	UPLOAD_FLOOR_BYTES_PER_SECOND,
	WRITE_REQUEST_TIMEOUT_MS,
	requestTimeoutMs,
} from "../request-deadline";

/**
 * Every bound must exceed the server's own budget for that route so the API's
 * 504 arrives before the client gives up (packages/api/src/middleware/deadline.rs).
 */
describe("requestTimeoutMs mirrors the API deadline classes", () => {
	test.each([
		["POST", "apps/app-1/events/ev-1/prerun", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/events/ev-1", WRITE_REQUEST_TIMEOUT_MS],
		["DELETE", "apps/app-1/events/ev-1", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/events", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/invoke/presign", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/packages", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/pages/bootstrap", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "user/info", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "registry/search", WRITE_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/board/board-1", DATA_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/board/board-1", DATA_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/board/board-1/logs", DATA_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/db/table/columns", DATA_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/graph/overlay/cypher", DATA_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/analytics/summary", DATA_REQUEST_TIMEOUT_MS],
		["GET", "apps/app-1/sales/discounts/d-1", DATA_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/fork", DATA_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/fork/offline/begin", DATA_REQUEST_TIMEOUT_MS],
		["POST", "apps/fork/online/begin", DATA_REQUEST_TIMEOUT_MS],
		["GET", "apps/fork/jobs/job-1", DATA_REQUEST_TIMEOUT_MS],
		["POST", "registry/publish", DATA_REQUEST_TIMEOUT_MS],
		["POST", "courses/c-1/assets/a-1/optimize", DATA_REQUEST_TIMEOUT_MS],
		["GET", "execution/poll", DATA_REQUEST_TIMEOUT_MS],
		["DELETE", "execution/run/run-1", DATA_REQUEST_TIMEOUT_MS],
		["POST", "oauth/token", DATA_REQUEST_TIMEOUT_MS],
		["GET", "audit/verify", DATA_REQUEST_TIMEOUT_MS],
		["POST", "admin/cache/sweep", JOB_REQUEST_TIMEOUT_MS],
		["POST", "maintenance/run", JOB_REQUEST_TIMEOUT_MS],
		["PUT", "apps/app-1/events/ev-1", DISPATCH_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/events/ev-1/setup", DISPATCH_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/events/ev-1/restore", DISPATCH_REQUEST_TIMEOUT_MS],
		[
			"POST",
			"apps/app-1/events/ev-1/canary/promote",
			DISPATCH_REQUEST_TIMEOUT_MS,
		],
		[
			"POST",
			"apps/app-1/events/ev-1/mcp-operation",
			DISPATCH_REQUEST_TIMEOUT_MS,
		],
		["POST", "apps/app-1/events/ev-1/mcp/tools", DISPATCH_REQUEST_TIMEOUT_MS],
		["POST", "apps/app-1/events/ev-1/rest/orders", DISPATCH_REQUEST_TIMEOUT_MS],
		[
			"POST",
			"apps/app-1/events/ev-1/invoke/async",
			DISPATCH_REQUEST_TIMEOUT_MS,
		],
		["POST", "apps/app-1/board/board-1/invoke", DISPATCH_REQUEST_TIMEOUT_MS],
		[
			"POST",
			"apps/app-1/graph/overlay/actions/act-1/invoke",
			DISPATCH_REQUEST_TIMEOUT_MS,
		],
		["POST", "sink/trigger/sink-1", DISPATCH_REQUEST_TIMEOUT_MS],
		["POST", "r/alias", DISPATCH_REQUEST_TIMEOUT_MS],
		["POST", "m/alias", DISPATCH_REQUEST_TIMEOUT_MS],
	])("%s %s → %i ms", (method, path, expected) => {
		expect(requestTimeoutMs(path, method)).toBe(expected);
	});

	test("the method is matched case-insensitively", () => {
		expect(requestTimeoutMs("apps/app-1/events/ev-1", "put")).toBe(
			DISPATCH_REQUEST_TIMEOUT_MS,
		);
	});

	test("a request body earns upload time on top of the route bound", () => {
		const fourMiB = 4 * 1024 * 1024;
		expect(requestTimeoutMs("apps/app-1/board/board-1", "POST", fourMiB)).toBe(
			DATA_REQUEST_TIMEOUT_MS +
				Math.floor(fourMiB / UPLOAD_FLOOR_BYTES_PER_SECOND) * 1000,
		);
		expect(requestTimeoutMs("apps/app-1/board/board-1", "POST", 0)).toBe(
			DATA_REQUEST_TIMEOUT_MS,
		);
		// A JSON body of a few bytes is not an upload.
		expect(requestTimeoutMs("apps/app-1/events/ev-1/prerun", "POST", 2)).toBe(
			WRITE_REQUEST_TIMEOUT_MS,
		);
	});

	test("bounds are ordered write < data < job < dispatch", () => {
		expect(WRITE_REQUEST_TIMEOUT_MS).toBeLessThan(DATA_REQUEST_TIMEOUT_MS);
		expect(DATA_REQUEST_TIMEOUT_MS).toBeLessThan(JOB_REQUEST_TIMEOUT_MS);
		expect(JOB_REQUEST_TIMEOUT_MS).toBeLessThan(DISPATCH_REQUEST_TIMEOUT_MS);
	});
});
