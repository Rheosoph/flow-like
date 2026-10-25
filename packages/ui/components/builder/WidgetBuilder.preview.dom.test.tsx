import { expect, test } from "bun:test";

// Keep renderer and backend mocks isolated from other builder tests.
if (process.env.FLOW_BUILDER_PREVIEW_TEST_CHILD === "1") {
	await import("./WidgetBuilder.preview.dom.fixture");
} else {
	test("builder preview follows JSON edits after a workflow changes the surface", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_BUILDER_PREVIEW_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 15_000);
}
