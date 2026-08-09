import { describe, expect, test } from "vitest";
import { ApiResponseError, apiResponseError } from "../api-error";

function response(status: number, statusText: string, headers = new Headers()) {
	return { status, statusText, headers };
}

describe("apiResponseError", () => {
	test("preserves API code, body correlation id, status, and path", () => {
		const error = apiResponseError(
			response(500, "Internal Server Error"),
			JSON.stringify({
				error: {
					code: "INTERNAL_ERROR",
					id: "m6ai687qs5uxfd6wxmgwmztt",
					message: "Internal Server Error",
				},
			}),
			"apps/example/events",
		);

		expect(error).toBeInstanceOf(ApiResponseError);
		expect(error).toMatchObject({
			status: 500,
			code: "INTERNAL_ERROR",
			errorId: "m6ai687qs5uxfd6wxmgwmztt",
			path: "apps/example/events",
		});
		expect(error.message).toContain("m6ai687qs5uxfd6wxmgwmztt");
	});

	test("falls back to the x-error-id response header", () => {
		const headers = new Headers({ "x-error-id": "server-reference" });
		const error = apiResponseError(
			response(503, "Service Unavailable", headers),
			"",
		);

		expect(error.errorId).toBe("server-reference");
		expect(error.message).toContain("Service Unavailable");
	});

	test("keeps a non-JSON server response useful", () => {
		const error = apiResponseError(
			response(502, "Bad Gateway"),
			"upstream reset",
		);

		expect(error.status).toBe(502);
		expect(error.message).toContain("upstream reset");
	});
});
