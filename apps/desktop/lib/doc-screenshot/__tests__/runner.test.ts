import { describe, expect, test } from "vitest";

import { validateDocScreenshotHttpFixture } from "../plan";
import {
	buildScreenshotUrl,
	redactScreenshotUrl,
	resolveElementCaptureClip,
	resolveHttpFixtureRequest,
} from "../runner";
import { DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA } from "../types";

describe("document screenshot URL helpers", () => {
	test("builds same-origin URLs with scalar and repeated query values", () => {
		const url = buildScreenshotUrl(
			new URL("http://127.0.0.1:3210/base?from=base"),
			"/onboarding?preserved=yes&tag=stale",
			{
				tag: ["welcome", "profiles"],
				empty: null,
				enabled: true,
				count: 3,
				omitted: undefined,
			},
		);

		expect(url.origin).toBe("http://127.0.0.1:3210");
		expect(url.pathname).toBe("/onboarding");
		expect(url.searchParams.get("preserved")).toBe("yes");
		expect(url.searchParams.getAll("tag")).toEqual(["welcome", "profiles"]);
		expect(url.searchParams.get("empty")).toBe("");
		expect(url.searchParams.get("enabled")).toBe("true");
		expect(url.searchParams.get("count")).toBe("3");
		expect(url.searchParams.has("omitted")).toBe(false);
	});

	test("removes URL credentials and redacts every sensitive query value", () => {
		const redacted = new URL(
			redactScreenshotUrl(
				"https://docs-user:docs-pass@example.test/onboarding?token=one&token=two&apiKey=secret&authorization=bearer&theme=dark#intro",
			),
		);

		expect(redacted.username).toBe("");
		expect(redacted.password).toBe("");
		expect(redacted.searchParams.getAll("token")).toEqual([
			"[REDACTED]",
			"[REDACTED]",
		]);
		expect(redacted.searchParams.get("apiKey")).toBe("[REDACTED]");
		expect(redacted.searchParams.get("authorization")).toBe("[REDACTED]");
		expect(redacted.searchParams.get("theme")).toBe("dark");
		expect(redacted.hash).toBe("#intro");
		expect(redacted.toString()).not.toContain("docs-pass");
		expect(redacted.toString()).not.toContain("secret");
		expect(redacted.toString()).not.toContain("bearer");
	});
});

describe("document screenshot element capture", () => {
	test("scrolls an offscreen target into view before resolving its clip", async () => {
		const calls: string[] = [];
		let isOffscreen = true;
		const clip = await resolveElementCaptureClip(
			{
				async scrollIntoView() {
					calls.push("scroll");
					isOffscreen = false;
				},
				async boundingBox() {
					calls.push("bounds");
					return {
						x: 12,
						y: isOffscreen ? -900 : 24,
						width: 320,
						height: 180,
					};
				},
			},
			'[data-doc-screenshot="live-editor"]',
			8,
		);

		expect(calls).toEqual(["scroll", "bounds"]);
		expect(clip).toEqual({
			x: 4,
			y: 16,
			width: 336,
			height: 196,
		});
	});
});

describe("document screenshot HTTP fixture routing", () => {
	const fixture = validateDocScreenshotHttpFixture({
		schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
		strict: true,
		routes: [
			{
				request: {
					method: "POST",
					url: "https://api.example.test/config?mode=docs",
					body: "{}",
				},
				response: {
					status: 201,
					json: { created: true },
				},
			},
		],
	});

	test("responds only to the exact method, URL, and raw body", () => {
		expect(
			resolveHttpFixtureRequest(fixture, "http://127.0.0.1:3001", {
				method: "POST",
				url: "https://api.example.test/config?mode=docs",
				body: "{}",
			}),
		).toEqual({
			action: "respond",
			response: {
				status: 201,
				headers: {},
				body: undefined,
				json: { created: true },
			},
		});

		expect(
			resolveHttpFixtureRequest(fixture, "http://127.0.0.1:3001", {
				method: "POST",
				url: "https://api.example.test/config?mode=docs",
				body: '{ "different": true }',
			}),
		).toMatchObject({ action: "abort" });
	});

	test("allows a route to ignore a non-deterministic request body", () => {
		const bodyAgnosticFixture = validateDocScreenshotHttpFixture({
			schema: DOC_SCREENSHOT_HTTP_FIXTURE_SCHEMA,
			routes: [
				{
					request: {
						method: "POST",
						url: "https://telemetry.example.test/envelope",
					},
					response: { status: 200 },
				},
			],
		});
		expect(
			resolveHttpFixtureRequest(bodyAgnosticFixture, "http://127.0.0.1:3001", {
				method: "POST",
				url: "https://telemetry.example.test/envelope",
				body: '{"event_id":"changes-every-run"}',
			}),
		).toMatchObject({ action: "respond" });
	});

	test("always allows unmatched same-origin and non-HTTP resources", () => {
		expect(
			resolveHttpFixtureRequest(fixture, "http://127.0.0.1:3001", {
				method: "GET",
				url: "http://127.0.0.1:3001/_next/static/app.js",
			}),
		).toEqual({ action: "continue" });
		expect(
			resolveHttpFixtureRequest(fixture, "http://127.0.0.1:3001", {
				method: "GET",
				url: "data:image/svg+xml;base64,PHN2Zy8+",
			}),
		).toEqual({ action: "continue" });
	});

	test("blocks an explicitly declared telemetry origin without a strict violation", () => {
		const telemetryFixture = {
			...fixture,
			blockedOrigins: ["https://telemetry.example.test"],
		};
		expect(
			resolveHttpFixtureRequest(telemetryFixture, "http://127.0.0.1:3001", {
				method: "POST",
				url: "https://telemetry.example.test/envelope?version=changes",
				body: '{"event_id":"changes"}',
			}),
		).toEqual({ action: "block" });
	});

	test("blocks a declared endpoint for every query without blocking its origin", () => {
		const endpointFixture = {
			...fixture,
			blockedEndpoints: ["https://api.example.test/og"],
		};
		expect(
			resolveHttpFixtureRequest(endpointFixture, "http://127.0.0.1:3001", {
				method: "GET",
				url: "https://api.example.test/og?url=https%3A%2F%2Fexample.test",
			}),
		).toEqual({ action: "block" });
		expect(
			resolveHttpFixtureRequest(endpointFixture, "http://127.0.0.1:3001", {
				method: "GET",
				url: "https://api.example.test/config",
			}),
		).toMatchObject({ action: "abort" });
	});

	test("blocks unmatched cross-origin HTTP and redacts sensitive query values", () => {
		const resolution = resolveHttpFixtureRequest(
			fixture,
			"http://127.0.0.1:3001",
			{
				method: "GET",
				url: "https://api.example.test/config?token=secret",
			},
		);

		expect(resolution).toMatchObject({
			action: "abort",
		});
		expect(resolution.action === "abort" ? resolution.error : "").toContain(
			"token=%5BREDACTED%5D",
		);
		expect(JSON.stringify(resolution)).not.toContain("secret");
	});

	test("allows unmatched cross-origin traffic only in explicit permissive mode", () => {
		const permissiveFixture = {
			...fixture,
			strict: false,
		};
		expect(
			resolveHttpFixtureRequest(permissiveFixture, "http://127.0.0.1:3001", {
				method: "GET",
				url: "https://unlisted.example.test/image.png",
			}),
		).toEqual({ action: "continue" });
	});
});
