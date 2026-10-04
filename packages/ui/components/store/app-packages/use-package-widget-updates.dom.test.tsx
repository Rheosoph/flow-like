import { expect, test } from "bun:test";

// Isolate module mocks from other package and builder tests.
if (process.env.FLOW_PACKAGE_WIDGET_UPDATES_TEST_CHILD === "1") {
	await import("./use-package-widget-updates.dom.fixture");
} else {
	test("package widget updates refresh releases, preserve scope, and retry failures", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_PACKAGE_WIDGET_UPDATES_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 15_000);
}
