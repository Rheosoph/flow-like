import { describe, expect, test } from "vitest";
import {
	ApiResponseError,
	apiErrorDiagnostic,
	apiResponseError,
} from "../api-error";

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

	test("redacts bearer capabilities and query data from diagnostic paths", () => {
		const error = apiResponseError(
			response(403, "Forbidden"),
			"",
			"apps/app-id/team/link/join/invite-secret?access_token=oauth-secret",
		);

		expect(error.path).toContain("invite-secret");
		expect(apiErrorDiagnostic(error).path).toBe(
			"apps/app-id/team/link/join/[REDACTED]",
		);
		expect(apiErrorDiagnostic(error).path).not.toContain("oauth-secret");
	});

	test("strips credentials and signatures from absolute diagnostic URLs", () => {
		const error = apiResponseError(
			response(502, "Bad Gateway"),
			"",
			"https://user:password@example.com/file?X-Amz-Signature=secret",
		);

		expect(error.path).toContain("password");
		expect(apiErrorDiagnostic(error).path).toBe("https://example.com/file");
	});

	test("does not serialize the server response message into diagnostics", () => {
		const error = apiResponseError(
			response(400, "Bad Request"),
			JSON.stringify({ message: "rejected secret-value" }),
			"apps/example/events",
		);

		expect(error.message).toContain("secret-value");
		expect(JSON.stringify(error.toJSON())).toContain("secret-value");
		expect(JSON.stringify(apiErrorDiagnostic(error))).not.toContain(
			"secret-value",
		);
	});

	test("preserves ordinary paths and redacts tracking capabilities", () => {
		const ordinary = apiResponseError(
			response(404, "Not Found"),
			"",
			"apps/example/events",
		);
		const capability = apiResponseError(
			response(404, "Not Found"),
			"",
			"solution/track/tracking-secret",
		);

		expect(apiErrorDiagnostic(ordinary).path).toBe("apps/example/events");
		expect(apiErrorDiagnostic(capability).path).toBe(
			"solution/track/[REDACTED]",
		);
	});
});
