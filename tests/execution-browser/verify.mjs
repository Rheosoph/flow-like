import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { chromium } from "playwright-core";

const require = createRequire(import.meta.url);
const output =
	process.env.EXECUTION_QA_OUTPUT ||
	join(tmpdir(), "flow-like-execution-verification");
const origin = process.env.EXECUTION_QA_ORIGIN || "http://127.0.0.1:4348";
await mkdir(output, { recursive: true });
const browser = await chromium.launch({
	headless: true,
	executablePath:
		process.env.CHROME_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
});
const page = await browser.newPage({
	viewport: { width: 1440, height: 1000 },
	reducedMotion: "reduce",
});
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) =>
	route.request().url().startsWith(origin) ||
	/^(data:|blob:)/.test(route.request().url())
		? route.continue()
		: route.abort(),
);
const activity = () => page.getByRole("button", { name: /^Activity,/ });
const tray = () => page.getByRole("dialog", { name: "Execution activity" });
const screenshot = (name) =>
	page.screenshot({
		path: join(output, name),
		fullPage: true,
		animations: "disabled",
	});
const check = (name) => console.log(`PASS ${name}`);
const audit = async () => {
	await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
	const violations = await page.evaluate(async () => {
		const result = await window.axe.run(
			document.querySelector('[aria-label="Execution activity"]'),
			{ runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] } },
		);
		return result.violations.map(({ id, impact, nodes }) => ({
			id,
			impact,
			targets: nodes.map((node) => node.target),
		}));
	});
	assert.deepEqual(violations, [], "Activity accessibility violations");
};

try {
	await page.goto(origin, { waitUntil: "networkidle" });
	await activity().click();
	await tray().waitFor();
	assert.equal(await tray().locator("[data-task-id]").count(), 4);
	await screenshot("01-activity-light.png");
	await audit();
	check("all task states appear with accessible controls");

	await page.evaluate(() => document.documentElement.classList.add("dark"));
	await screenshot("02-activity-dark.png");
	await audit();
	await tray().getByRole("button", { name: "Respond", exact: true }).click();
	await tray()
		.getByRole("radio", { name: "Approve invoice", exact: true })
		.waitFor();
	await screenshot("03-feedback-dark.png");
	await audit();
	check("feedback opens with the real interaction controls");

	await tray()
		.getByRole("radio", { name: "Approve invoice", exact: true })
		.click();
	await page.evaluate(() => {
		window.executionQa.engine.failReply = true;
	});
	await tray().getByRole("button", { name: "Submit", exact: true }).click();
	await tray().getByRole("alert").waitFor();
	assert.match(
		await tray().getByRole("alert").innerText(),
		/could not be sent/,
	);
	await page.evaluate(() => {
		window.executionQa.engine.failReply = false;
	});
	await tray().getByRole("button", { name: "Submit", exact: true }).click();
	await tray().getByText("You’re all caught up", { exact: true }).waitFor();
	assert.deepEqual(
		await page.evaluate(() =>
			window.executionQa.engine.calls.filter((call) => call.action === "reply"),
		),
		[
			{
				action: "reply",
				taskId: "approval",
				requestId: "approval-question",
				value: { selected_id: "approve" },
			},
		],
	);
	check("feedback failures keep the answer available and retry submits once");
	await tray().getByRole("button", { name: "Back to activity" }).click();

	await page.evaluate(() => {
		window.executionQa.engine.failCancel = true;
	});
	await tray()
		.getByRole("button", {
			name: "Stop Prepare the weekly briefing",
			exact: true,
		})
		.click();
	await tray().getByRole("alert").waitFor();
	await page.evaluate(() => {
		window.executionQa.engine.failCancel = false;
	});
	await tray()
		.getByRole("button", {
			name: "Stop Prepare the weekly briefing",
			exact: true,
		})
		.click();
	await tray()
		.locator('[data-task-id="research"][data-task-status="cancelled"]')
		.waitFor();
	await tray()
		.getByRole("button", {
			name: "Dismiss Prepare the weekly briefing",
			exact: true,
		})
		.click();
	assert.equal(await tray().locator('[data-task-id="research"]').count(), 0);
	check("stop errors are visible, retry stops the run, and dismiss removes it");

	await tray()
		.getByRole("button", {
			name: "Open Summarize customer feedback",
			exact: true,
		})
		.click();
	await page.waitForURL("**/chat/feedback");
	await tray().waitFor({ state: "hidden" });
	assert.equal(await tray().count(), 0);
	await activity().click();
	await tray()
		.getByRole("button", { name: "Clear recent", exact: true })
		.click();
	assert.equal(await tray().locator("[data-task-id]").count(), 1);
	check(
		"opening a conversation preserves active work and clear recent leaves active runs",
	);

	await page.evaluate(() => {
		window.executionQa.engine.reset();
		document.documentElement.classList.remove("dark");
	});
	await page.setViewportSize({ width: 390, height: 844 });
	await screenshot("04-activity-mobile-light.png");
	const bounds = await tray().boundingBox();
	assert.ok(
		bounds &&
			bounds.x >= 0 &&
			bounds.x + bounds.width <= 390 &&
			bounds.y >= 0 &&
			bounds.y + bounds.height <= 844,
	);
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth > window.innerWidth,
		),
		false,
	);
	await tray().getByRole("button", { name: "Respond", exact: true }).click();
	await tray()
		.getByRole("radio", { name: "Approve invoice", exact: true })
		.waitFor();
	await screenshot("05-feedback-mobile-light.png");
	await audit();
	await page.keyboard.press("Escape");
	await tray().waitFor({ state: "hidden" });
	assert.equal(await tray().count(), 0);
	assert.equal(
		await activity().evaluate((element) => element === document.activeElement),
		true,
	);
	check("mobile tray stays in the viewport and Escape restores focus");
	await page.evaluate(() => window.executionQa.engine.addMachineTask());
	await activity().click();
	const machine = tray().locator('[data-task-id="page-read"]');
	await machine
		.getByText("Waiting for page or device", { exact: true })
		.waitFor();
	assert.equal(
		await machine.getByRole("button", { name: "Respond", exact: true }).count(),
		0,
	);
	await machine
		.getByRole("button", { name: "Open Read the inventory page", exact: true })
		.waitFor();
	await machine.scrollIntoViewIfNeeded();
	await screenshot("07-machine-waiting-mobile.png");
	await machine
		.getByRole("button", { name: "Stop Read the inventory page", exact: true })
		.click();
	await machine
		.getByRole("button", {
			name: "Dismiss Read the inventory page",
			exact: true,
		})
		.click();
	assert.equal(await machine.count(), 0);
	check(
		"page and device requests show waiting controls without rendering a human feedback form",
	);
	assert.deepEqual(errors, [], "Browser errors");
	console.log(`Screenshots: ${output}`);
} finally {
	await browser.close();
}
