import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";
const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { passed: [], errors: [], blocked: [] };
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	report.blocked.push(url.origin);
	return route.abort();
});
page.on("pageerror", (error) => report.errors.push(error.message));
page.on("console", (message) => {
	if (message.type() === "error") report.errors.push(message.text());
});
try {
	await page.goto("http://127.0.0.1:4318/activity-fixture", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("heading", {
			name: "Activity widgets · local fixture",
			exact: true,
		})
		.waitFor({ timeout: 60_000 });
	await page
		.getByText("Fixture invoice needs review", { exact: true })
		.waitFor();
	await page
		.getByTestId("activity-ai-usage")
		.getByText("$1.25", { exact: true })
		.waitFor();
	assert.equal(
		await page
			.getByTestId("activity-ai-usage")
			.getByText("$0.35", { exact: true })
			.count(),
		1,
	);
	const chart = await page.getByTestId("activity-run-activity").innerText();
	assert.ok(
		chart.includes("100") && chart.includes("243"),
		"Coverage states sampled and total execution counts",
	);
	assert.ok(
		!/success rate|succeeded|failed/i.test(chart),
		"Severity history does not invent run outcomes",
	);
	await page
		.getByTestId("activity-executions-by-app")
		.getByText("Fixture Chat", { exact: true })
		.waitFor();
	assert.ok(
		(await page.getByTestId("activity-needs-attention").innerText()).includes(
			"Error / Fatal",
		),
	);

	const visibleBars = await page
		.getByTestId("activity-run-activity")
		.locator('[title$="Error/Fatal"] > div')
		.evaluateAll(
			(bars) =>
				bars.filter((bar) => bar.getBoundingClientRect().height > 1).length,
		);
	assert.ok(
		visibleBars > 0,
		"Populated activity bars have a visible height under natural content sizing",
	);
	await page.screenshot({
		path: "/private/tmp/home-qa-activity-populated.png",
		fullPage: true,
	});
	report.passed.push(
		"Personal activity and app ranking show sample coverage; AI usage renders recorded costs; needs-attention combines severity records and notifications",
	);
	await page.getByLabel("Period", { exact: true }).selectOption("1");
	await page
		.getByTestId("activity-run-activity")
		.getByText("Today · UTC", { exact: true })
		.waitFor();
	await page.getByLabel("Scenario", { exact: true }).selectOption("empty");
	await page
		.getByTestId("activity-run-activity")
		.getByText("No sampled executions fall in this period.", { exact: true })
		.waitFor();
	await page
		.getByTestId("activity-executions-by-app")
		.getByText("No sampled executions fall in this period.", { exact: true })
		.waitFor();
	await page
		.getByText(
			"No unread workflow notifications in the latest 100 notifications.",
			{ exact: true },
		)
		.waitFor();
	await page.screenshot({
		path: "/private/tmp/home-qa-activity-empty.png",
		fullPage: true,
	});
	report.passed.push(
		"Time filter and empty data render without invented values or outcomes",
	);
	await page.getByLabel("Scenario", { exact: true }).selectOption("error");
	await page
		.getByTestId("activity-run-activity")
		.getByRole("button", { name: "Try again", exact: true })
		.waitFor();
	await page
		.getByTestId("activity-ai-usage")
		.getByRole("button", { name: "Try again", exact: true })
		.waitFor();
	await page.getByLabel("Scenario", { exact: true }).selectOption("populated");
	await page
		.getByTestId("activity-ai-usage")
		.getByText("$1.25", { exact: true })
		.waitFor();
	await page.setViewportSize({ width: 390, height: 844 });
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth > innerWidth,
		),
		false,
	);
	await page.screenshot({
		path: "/private/tmp/home-qa-activity-mobile.png",
		fullPage: true,
	});
	report.passed.push(
		"Access errors offer retry, recover when data returns, and fit a 390px viewport",
	);
	assert.deepEqual(report.errors, []);
} catch (error) {
	report.failure = String(error.stack ?? error);
	await page.screenshot({
		path: "/private/tmp/home-qa-activity-failure.png",
		fullPage: true,
	});
	console.log((await page.locator("body").innerText()).slice(-8000));
} finally {
	await writeFile(
		"/private/tmp/home-qa-activity-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
if (report.failure) process.exitCode = 1;
