import { afterEach, describe, expect, test, vi } from "vitest";

import {
	UniversityApiError,
	createUniversityClient,
	normalizeUniversityBaseUrl,
	uploadToSignedUrl,
} from "../client";

const originalFetch = globalThis.fetch;

afterEach(() => {
	globalThis.fetch = originalFetch;
	vi.restoreAllMocks();
});

function stubFetch(
	handler: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>,
): void {
	globalThis.fetch = vi.fn(handler) as unknown as typeof fetch;
}

function jsonResponse(value: unknown, status = 200): Response {
	return new Response(JSON.stringify(value), {
		status,
		headers: { "Content-Type": "application/json" },
	});
}

describe("University API client", () => {
	test("normalizes a deployment root to exactly one API prefix", () => {
		expect(
			normalizeUniversityBaseUrl(" https://flow.example///?old=1#hash "),
		).toBe("https://flow.example/api/v1");
		expect(normalizeUniversityBaseUrl("https://flow.example/api/v1/")).toBe(
			"https://flow.example/api/v1",
		);
		expect(normalizeUniversityBaseUrl("https://flow.example/backend/")).toBe(
			"https://flow.example/backend/api/v1",
		);
	});

	test.each([
		"ftp://flow.example",
		"http://flow.example",
		"https://user:password@flow.example",
		"not a URL",
	])("rejects unsafe API base URL %s", (baseUrl) => {
		expect(() => normalizeUniversityBaseUrl(baseUrl)).toThrow(
			"University API base URL",
		);
	});

	test("keeps the PAT non-enumerable at runtime", () => {
		const client = createUniversityClient({
			baseUrl: "https://flow.example",
			pat: "pat_test.secret",
		});

		expect(Object.keys(client)).toEqual(["baseUrl"]);
		expect(JSON.stringify(client)).not.toContain("pat_test.secret");
	});

	test("allows HTTP only for loopback development", () => {
		expect(normalizeUniversityBaseUrl("http://127.0.0.1:8080")).toBe(
			"http://127.0.0.1:8080/api/v1",
		);
		expect(normalizeUniversityBaseUrl("http://[::1]:8080")).toBe(
			"http://[::1]:8080/api/v1",
		);
	});

	test("sends a raw PAT and builds encoded paths and queries", async () => {
		let captured: { url: string; init?: RequestInit } | undefined;
		stubFetch(async (input, init) => {
			captured = { url: String(input), init };
			return jsonResponse([]);
		});
		const client = createUniversityClient({
			baseUrl: "https://flow.example/",
			pat: "pat_test.secret",
		});

		await client.listCourses({
			language: "de DE",
			include_unpublished: true,
			limit: 25,
			offset: 0,
		});

		expect(captured?.url).toBe(
			"https://flow.example/api/v1/courses?language=de+DE&include_unpublished=true&limit=25&offset=0",
		);
		const headers = new Headers(captured?.init?.headers);
		expect(headers.get("Authorization")).toBe("pat_test.secret");
		expect(headers.get("Accept")).toBe("application/json");
		expect(headers.get("Content-Type")).toBeNull();

		stubFetch(async (input) => {
			captured = { url: String(input) };
			return jsonResponse({ id: "course/one" });
		});
		await client.getCourse("course/one", "en-US");
		expect(captured?.url).toBe(
			"https://flow.example/api/v1/courses/course%2Fone?language=en-US",
		);
	});

	test.each(["", "Bearer token", "pat_has whitespace"])(
		"rejects non-PAT credential %j",
		(pat) => {
			expect(() =>
				createUniversityClient({
					baseUrl: "https://flow.example",
					pat,
				}),
			).toThrow(/PAT|whitespace/);
		},
	);

	test("parses the backend nested error envelope without losing diagnostics", async () => {
		const body = {
			error: {
				code: "FORBIDDEN",
				id: "error-reference-1",
				message: "WriteCourses required",
			},
		};
		stubFetch(async () => jsonResponse(body, 403));
		const client = createUniversityClient({
			baseUrl: "https://flow.example",
			pat: "pat_test.secret",
		});

		let caught: unknown;
		try {
			await client.getCourse("private-course");
		} catch (error) {
			caught = error;
		}

		expect(caught).toBeInstanceOf(UniversityApiError);
		expect(caught).toMatchObject({
			message: "WriteCourses required",
			status: 403,
			method: "GET",
			url: "https://flow.example/api/v1/courses/private-course",
			code: "FORBIDDEN",
			errorId: "error-reference-1",
			body,
		});
	});

	test("sets Azure BlockBlob headers and deletes asset metadata after upload failure", async () => {
		const asset = {
			id: "asset/id",
			course_id: "course one",
			name: "EditorOverview",
			filename: "overview.png",
			mime_type: "image/png",
			size: 3,
			kind: "IMAGE" as const,
			created_at: "2026-08-13T00:00:00Z",
			updated_at: "2026-08-13T00:00:00Z",
		};
		const signedUrl =
			"https://courseassets.blob.core.windows.net/assets/overview.png?sig=test";
		const calls: Array<{ url: string; init?: RequestInit }> = [];
		stubFetch(async (input, init) => {
			calls.push({ url: String(input), init });
			if (calls.length === 1) {
				return jsonResponse({ asset, signed_url: signedUrl }, 201);
			}
			if (calls.length === 2) {
				return jsonResponse(
					{
						error: {
							code: "UPLOAD_FAILED",
							message: "Storage rejected upload",
						},
					},
					500,
				);
			}
			return new Response(null, { status: 204 });
		});
		const client = createUniversityClient({
			baseUrl: "https://flow.example",
			pat: "pat_test.secret",
		});
		const metadata = {
			name: "EditorOverview",
			filename: "overview.png",
			mime_type: "image/png",
			size: 3,
			kind: "IMAGE" as const,
			extension: "png",
		};

		await expect(
			client.uploadCourseAsset(
				"course one",
				metadata,
				new Uint8Array([1, 2, 3]),
			),
		).rejects.toMatchObject({
			message: "Storage rejected upload",
			status: 500,
			code: "UPLOAD_FAILED",
		});

		expect(calls.map(({ url, init }) => [init?.method, url])).toEqual([
			["POST", "https://flow.example/api/v1/courses/course%20one/assets"],
			["PUT", signedUrl],
			[
				"DELETE",
				"https://flow.example/api/v1/courses/course%20one/assets/asset%2Fid",
			],
		]);
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual(metadata);

		const createHeaders = new Headers(calls[0]?.init?.headers);
		const uploadHeaders = new Headers(calls[1]?.init?.headers);
		const cleanupHeaders = new Headers(calls[2]?.init?.headers);
		expect(createHeaders.get("Authorization")).toBe("pat_test.secret");
		expect(uploadHeaders.get("Content-Type")).toBe("image/png");
		expect(uploadHeaders.get("x-ms-blob-type")).toBe("BlockBlob");
		expect(uploadHeaders.has("Authorization")).toBe(false);
		expect(cleanupHeaders.get("Authorization")).toBe("pat_test.secret");
	});

	test("redacts signed upload URLs and refuses credential headers", async () => {
		stubFetch(async () =>
			jsonResponse(
				{
					error: {
						message:
							"Upload failed at https://storage.example/file?X-Amz-Signature=secret",
					},
				},
				403,
			),
		);
		let caught: unknown;
		try {
			await uploadToSignedUrl(
				"https://storage.example/file?X-Amz-Signature=secret",
				new Uint8Array([1]),
			);
		} catch (error) {
			caught = error;
		}
		expect(caught).toBeInstanceOf(UniversityApiError);
		expect(caught).toMatchObject({
			message:
				"Upload failed at https://storage.example/file?X-Amz-Signature=[REDACTED]",
			url: "https://storage.example/file?X-Amz-Signature=%5BREDACTED%5D",
		});
		expect(JSON.stringify(caught)).not.toContain("secret");

		await expect(
			uploadToSignedUrl("https://storage.example/file", new Uint8Array([1]), {
				headers: { Authorization: "Bearer secret" },
			}),
		).rejects.toThrow("Sensitive header authorization");
	});

	test("rejects invalid successful response shapes", async () => {
		stubFetch(async () => new Response(null, { status: 204 }));
		const client = createUniversityClient({
			baseUrl: "https://flow.example",
			pat: "pat_test.secret",
		});

		await expect(client.listCourses()).rejects.toThrow("expected an array");
	});
});
