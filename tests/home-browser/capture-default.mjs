import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const prefix = process.env.HOME_CAPTURE_PREFIX || "/private/tmp/home-default";
const widths = (process.env.HOME_CAPTURE_WIDTHS || "1480,390")
	.split(",")
	.map(Number);
const scenarios = (process.env.HOME_CAPTURE_SCENARIOS || "returning").split(
	",",
);
const themes = (process.env.HOME_CAPTURE_THEMES || "dark").split(",");
const browser = await chromium.launch({
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	headless: true,
	args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { captures: [], errors: [], blocked: [] };
page.on("pageerror", (error) =>
	report.errors.push(error.stack || error.message),
);
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

try {
	await page.goto("http://127.0.0.1:4318/default-fixture", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	for (const scenario of scenarios) {
		await page
			.getByLabel("Profile state", { exact: true })
			.selectOption(scenario);
		await page.waitForFunction(
			(value) => window.defaultHomeQa?.scenario === value,
			scenario,
		);
		await page.locator("[data-home-greeting] h1").waitFor();
		if (scenario === "returning" || scenario === "fresh")
			await page
				.locator("[data-home-greeting] h1")
				.filter({ hasText: "Felix" })
				.waitFor();
		await page.waitForTimeout(500);
		for (const theme of themes) {
			await page.evaluate(
				(value) =>
					document.documentElement.classList.toggle("dark", value === "dark"),
				theme,
			);
			for (const width of widths) {
				await page.setViewportSize({ width, height: width < 768 ? 844 : 1050 });
				await page
					.locator("[data-home-widget]")
					.first()
					.evaluate((element) => {
						element.scrollIntoView({ block: "start" });
					});
				await page.waitForTimeout(300);
				const metrics = await page.evaluate(() => {
					const canvas = document.querySelector("[data-home-canvas]");
					let scroller = canvas.parentElement;
					while (
						scroller &&
						!(
							scroller.scrollHeight > scroller.clientHeight + 1 &&
							["auto", "scroll"].includes(getComputedStyle(scroller).overflowY)
						)
					)
						scroller = scroller.parentElement;
					window.captureScroller = scroller || document.scrollingElement;
					const base = canvas.getBoundingClientRect();
					return {
						width: innerWidth,
						pageWidth: document.documentElement.scrollWidth,
						canvasWidth: canvas.clientWidth,
						canvasScrollWidth: canvas.scrollWidth,
						scrollHeight: window.captureScroller.scrollHeight,
						clientHeight: window.captureScroller.clientHeight,
						widgets: [...document.querySelectorAll("[data-home-widget]")].map(
							(element) => {
								const rect = element.getBoundingClientRect();
								return {
									type: element.getAttribute("data-widget-type"),
									x: rect.x,
									y: rect.y - base.y,
									width: rect.width,
									height: rect.height,
									text: element.innerText.slice(0, 400),
								};
							},
						),
					};
				});
				assert.ok(metrics.pageWidth <= width + 1, "Page fits viewport");
				assert.ok(
					metrics.canvasScrollWidth <= metrics.canvasWidth + 1,
					"Canvas fits viewport",
				);
				const capture = { scenario, theme, ...metrics, screenshots: [] };
				report.captures.push(capture);
				const step = metrics.clientHeight - 100;
				const count =
					Math.ceil((metrics.scrollHeight - metrics.clientHeight) / step) + 1;
				for (let index = 0; index < count; index++) {
					await page.evaluate((value) => {
						window.captureScroller.scrollTop = value;
					}, index * step);
					await page.waitForTimeout(250);
					await page.waitForFunction(() =>
						[...document.querySelectorAll("[data-home-widget] img")].every(
							(image) => image.loading === "lazy" || image.complete,
						),
					);
					const path = `${prefix}-${scenario}-${theme}-${width}-${String(index + 1).padStart(2, "0")}.png`;
					await page.screenshot({ path });
					capture.screenshots.push(path);
				}
			}
		}
	}
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} finally {
	await writeFile(`${prefix}-report.json`, JSON.stringify(report, null, 2));
	console.log(
		JSON.stringify(
			{
				report: `${prefix}-report.json`,
				captures: report.captures.map(
					({ scenario, theme, width, scrollHeight, screenshots }) => ({
						scenario,
						theme,
						width,
						scrollHeight,
						screenshots: screenshots.length,
					}),
				),
				errors: report.errors,
				blocked: report.blocked,
			},
			null,
			2,
		),
	);
	await browser.close();
}
