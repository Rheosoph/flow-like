import { expect, test } from "bun:test";

// Bun module mocks survive mock.restore(). Keep the mounted editor fixture in
// a child process so its backend and UI stubs cannot alter other runtime tests.
if (process.env.FLOW_PAGE_BUILDER_TEST_CHILD === "1") {
	await import("./page-builder-surface.dom.fixture");
} else {
	test("page widget bindings load their saved board, refresh metadata on saves and on demand, and create and bind event nodes from the inspector", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_PAGE_BUILDER_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 15_000);
}
