import type { ChildProcess } from "node:child_process";
import { describe, expect, test, vi } from "vitest";
import { waitForServer } from "../../../scripts/doc-screenshot";
import type { DocScreenshotScenario } from "../types";

const server = {
	child: { exitCode: null, signalCode: null } as ChildProcess,
	url: new URL("http://127.0.0.1:3138"),
};
const scenario: DocScreenshotScenario = {
	name: "reference",
	path: "/debug/markdown?sample=chart",
	query: { capture: "docs" },
	steps: [{ type: "capture", name: "chart" }],
};

describe("document screenshot frontend readiness", () => {
	test("warms the selected scenario route and query instead of the home page", async () => {
		const fetch = vi
			.spyOn(globalThis, "fetch")
			.mockResolvedValue(new Response());
		try {
			await waitForServer(server, scenario, 1_000);
			expect(fetch).toHaveBeenCalledTimes(1);
			expect(String(fetch.mock.calls[0]?.[0])).toBe(
				"http://127.0.0.1:3138/debug/markdown?sample=chart&capture=docs",
			);
		} finally {
			fetch.mockRestore();
		}
	});

	test("rejects a route outside the local frontend before probing it", async () => {
		const fetch = vi
			.spyOn(globalThis, "fetch")
			.mockResolvedValue(new Response());
		try {
			await expect(
				waitForServer(
					server,
					{ ...scenario, path: "https://example.invalid" },
					1_000,
				),
			).rejects.toThrow("Navigation must stay on http://127.0.0.1:3138.");
			expect(fetch).not.toHaveBeenCalled();
		} finally {
			fetch.mockRestore();
		}
	});

	test("redacts sensitive query values in readiness failures", async () => {
		await expect(
			waitForServer(
				server,
				{ ...scenario, query: { token: "example-secret" } },
				0,
			),
		).rejects.toThrow(
			"Frontend did not become ready at http://127.0.0.1:3138/debug/markdown?sample=chart&token=%5BREDACTED%5D: not ready",
		);
	});
});
