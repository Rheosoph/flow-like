import { expect, test } from "bun:test";

// Isolate persistent Bun module mocks from other builder and runtime tests.
if (process.env.FLOW_PAGE_BUILDER_JSON_TEST_CHILD === "1") {
	await import("./page-builder-json.dom.fixture");
} else {
	test("Raw JSON updates the page canvas and saves the applied snapshot", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_PAGE_BUILDER_JSON_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 30_000);
}
