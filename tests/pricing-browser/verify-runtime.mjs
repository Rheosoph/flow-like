import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer";

const profile = await mkdtemp(join(tmpdir(), "flow-like-runtime-verify-"));
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
	userDataDir: profile,
});
const errors = [];
let checks = 0;
const check = (value, message) => {
	assert.ok(value, message);
	checks++;
};
try {
	const page = await browser.newPage();
	page.setDefaultTimeout(60000);
	page.on("pageerror", (error) => errors.push(error.message));
	await page.setRequestInterception(true);
	page.on("request", (request) =>
		/^(http:\/\/127\.0\.0\.1:4330\/|data:|blob:)/.test(request.url())
			? request.continue()
			: request.abort(),
	);
	const visit = async (query) => {
		await page.goto(`http://127.0.0.1:4330/?${query}`, {
			waitUntil: "networkidle0",
		});
		await page.waitForSelector("[data-runtime-calculator]");
		await page.click("[data-runtime-calculator] summary");
	};
	const text = () =>
		page.$eval("[data-runtime-calculator]", (node) => node.textContent);
	const setDuration = async (value) => {
		await page.click("[data-runtime-calculator] input", { clickCount: 3 });
		await page.keyboard.press("Backspace");
		if (value) await page.type("[data-runtime-calculator] input", value);
	};
	const values = () =>
		page.$$eval("[data-runtime-calculator] dd > span", (nodes) =>
			nodes.map((node) => Number(node.textContent.replace(/[^0-9]/g, ""))),
		);
	const preset = async (label) => {
		const handle = await page.waitForFunction(
			(label) =>
				[...document.querySelectorAll("[data-runtime-calculator] button")].find(
					(node) => node.textContent === label,
				),
			{},
			label,
		);
		await handle.asElement().click();
		await handle.dispose();
	};
	for (const width of [390, 1440]) {
		await page.setViewport({ width, height: 900 });
		await visit("view=subscription");
		check(
			(await text()).includes("5,500 settled cloud runs"),
			"Personal estimates use genuine workflow rows",
		);
		check(
			(await text()).includes("Estimate based on your recent usage"),
			"Observed average is labeled",
		);
		await setDuration("30");
		assert.deepEqual(await values(), [60, 2400, 7200, 18000]);
		checks++;
		check(
			(await text()).includes("Example at 30 seconds"),
			"Manual values are distinguished from usage",
		);
		await preset("5 sec");
		assert.deepEqual(await values(), [360, 10000, 25000, 100000]);
		checks++;
		check(
			(await text()).match(/Cloud-start limit reached first/g).length === 3,
			"Fast workflows obey start caps",
		);
		await preset("2 min");
		assert.deepEqual(await values(), [15, 600, 1800, 4500]);
		checks++;
		await preset("Use my average");
		check(
			(await text()).includes("Estimate based on your recent usage"),
			"Personal average can be restored",
		);
		for (const invalid of ["0", "-5", ""]) {
			await setDuration(invalid);
			check(
				await page.$eval(
					"[data-runtime-calculator] input",
					(node) => node.getAttribute("aria-invalid") === "true",
				),
				"Invalid durations are rejected",
			);
			check(
				(await values()).length === 0,
				"Invalid input hides numeric estimates",
			);
		}
		await setDuration("0.5");
		assert.deepEqual(await values(), [1000, 10000, 25000, 100000]);
		checks++;
		check(
			await page.evaluate(
				() => document.documentElement.scrollWidth <= innerWidth + 1,
			),
			"Calculator fits viewport",
		);
		console.log(`Runtime calculator passed at ${width}px`);
	}
	await visit("view=usage");
	check(
		await page.$eval('article[aria-label="Cloud runtime"]', (node) =>
			node.textContent.includes("About 1,760 more cloud runs"),
		),
		"Runtime meter shows remaining-run estimate",
	);
	await setDuration("30");
	assert.deepEqual(await values(), [576, 2400]);
	checks++;
	check(
		(await text()).includes("reserved for pending work"),
		"Available estimate explains pending capacity",
	);
	await visit("view=subscription&scenario=empty&tier=FREE");
	check(
		(await text()).includes("Start with a 30-second example"),
		"New accounts get an explicit example",
	);
	check(
		!(await text()).includes("Use my average"),
		"No invented personal history",
	);
	assert.deepEqual(await values(), [60, 2400, 7200, 18000]);
	checks++;
	check(errors.length === 0, `No browser exceptions: ${errors.join(";")}`);
	const output = resolve(
		process.env.SCREENSHOT_OUTPUT ?? "output/pricing-runtime-calculator",
	);
	await mkdir(output, { recursive: true });
	await writeFile(
		join(output, "app-runtime-estimate-results.json"),
		JSON.stringify({ checks, errors }, null, 2),
	);
	console.log(`Passed ${checks} runtime calculator browser checks.`);
} finally {
	await browser.close();
	await rm(profile, { recursive: true, force: true });
}
