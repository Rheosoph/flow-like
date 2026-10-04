import { expect, test } from "bun:test";

// Keep module mocks in a child process so they cannot change other editor tests.
if (process.env.FLOW_WIDGET_RELOAD_TEST_CHILD === "1") {
	await import("./use-micro-widget-reload.dom.fixture");
} else {
	test("project widget updates preserve live edits, share pending state, and retry failed saves", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_WIDGET_RELOAD_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 15_000);
}
