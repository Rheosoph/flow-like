import { expect, test } from "bun:test";

// Keep the page's unrelated UI mocks out of other package and builder tests.
if (process.env.FLOW_APP_PACKAGE_WIDGET_UPDATES_TEST_CHILD === "1") {
	await import("./app-packages-page.widget-updates.dom.fixture");
} else {
	test("app package settings update published releases and local widget builds", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_APP_PACKAGE_WIDGET_UPDATES_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) throw new Error(result.stderr.toString("utf8"));
		expect(result.exitCode).toBe(0);
	}, 15_000);
}
