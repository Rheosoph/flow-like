import { describe, expect, test } from "vitest";

import { buildScreenshotUrl, redactScreenshotUrl } from "../runner";

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
