import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ??
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const report = { passed: [], errors: [], blocked: [] };
page.on("pageerror", (error) => report.errors.push(error.message));
page.on("response", (response) => {
	if (response.status() >= 400)
		console.log("HTTP", response.status(), response.url());
});
page.on("console", (message) => {
	if (message.type() === "error") report.errors.push(message.text());
});
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	report.blocked.push(url.origin);
	return route.abort();
});
const primary = page.getByTestId("pulse-4");
try {
	await page.goto("http://127.0.0.1:4318/workspace-fixture", {
		waitUntil: "domcontentloaded",
	});
	await primary
		.locator('[data-workspace-pulse="activity"]')
		.waitFor({ timeout: 60_000 });
	await primary.getByText("60", { exact: true }).waitFor();
	assert.equal(await primary.getByText("20", { exact: true }).count(), 1);
	assert.equal(await primary.getByRole("img").count(), 1);
	assert.equal(
		await page
			.getByTestId("pulse-6")
			.getByText("Worth a look", { exact: true })
			.count(),
		0,
	);
	assert.equal(
		await primary.locator('a[href^="/library/config/analytics?id="]').count(),
		2,
	);
	await primary
		.getByText("Sample: 60 of 240 account records", { exact: true })
		.click();
	assert.ok(
		(await primary.innerText()).includes(
			"Sample counts may omit earlier activity",
		),
	);
	await primary
		.getByText("Sample: 60 of 240 account records", { exact: true })
		.click();
	report.passed.push(
		"Real account sample volume and Error/Fatal record counts, app analytics links, optional attention, and disclosed sample coverage",
	);
	for (const width of [320, 390, 768, 1440, 2048]) {
		await page.setViewportSize({ width, height: 1000 });
		await page.waitForFunction(
			() => document.documentElement.scrollWidth <= innerWidth + 1,
		);
		await page.screenshot({
			path: `/private/tmp/home-workspace-${width}.png`,
			fullPage: width === 1440,
		});
	}
	report.passed.push(
		"Workspace pulse fits 320, 390, 768, 1440, and 2048px viewports",
	);
	await page.getByLabel("Days", { exact: true }).selectOption("1");
	await primary
		.getByText("Today · your account · UTC", { exact: true })
		.waitFor();
	await page.getByLabel("Days", { exact: true }).selectOption("30");
	await primary
		.getByText("Last 30 days · your account · UTC", { exact: true })
		.waitFor();
	for (const scenario of ["empty", "offline", "guest", "error"]) {
		const calls = Number(await page.getByTestId("usage-calls").innerText());
		await page.getByLabel("Scenario", { exact: true }).selectOption(scenario);
		await primary
			.locator(
				`[data-workspace-pulse="${scenario === "error" ? "unavailable" : "starter"}"]`,
			)
			.waitFor();
		await primary
			.getByText(
				scenario === "empty"
					? "Start with one useful app"
					: "3 apps, ready to open",
				{ exact: true },
			)
			.waitFor();
		assert.equal(await primary.getByRole("img").count(), 0);
		assert.equal(
			await primary.getByText("Flagged records", { exact: true }).count(),
			0,
		);
		if (["offline", "guest"].includes(scenario))
			assert.equal(
				Number(await page.getByTestId("usage-calls").innerText()),
				calls,
			);
		if (scenario === "error") {
			await primary.getByRole("button", { name: "Retry", exact: true }).click();
			await page.waitForFunction(
				(previous) =>
					Number(
						document.querySelector('[data-testid="usage-calls"]').textContent,
					) > previous,
				calls,
			);
		}
		await page.setViewportSize({ width: 390, height: 844 });
		await page.screenshot({
			path: `/private/tmp/home-workspace-${scenario}-390.png`,
		});
	}
	report.passed.push(
		"Fresh, offline, guest, and failed-history states show useful starter actions and real library count without invented metrics; guests/offline never query history",
	);
	await primary.getByRole("button", { name: "Build with FlowPilot" }).click();
	await page.waitForURL("**/chat");
	report.passed.push("Build with FlowPilot opens the actual chat route");
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} finally {
	await writeFile(
		"/private/tmp/home-workspace-qa-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
