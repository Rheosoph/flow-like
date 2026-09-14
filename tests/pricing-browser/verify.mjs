import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import puppeteer from "puppeteer";

const profile = await mkdtemp(join(tmpdir(), "flow-like-pricing-verify-"));
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
	console.log(`Verified: ${message}`);
};
try {
	const page = await browser.newPage();
	page.setDefaultTimeout(120000);
	page.on("pageerror", (error) => errors.push(error.message));
	await page.setRequestInterception(true);
	page.on("request", (request) =>
		/^(http:\/\/127\.0\.0\.1:4330\/|data:|blob:)/.test(request.url())
			? request.continue()
			: request.abort(),
	);
	const visit = async (query) => {
		console.log(`Opening ${query}`);
		await page.goto(`http://127.0.0.1:4330/?${query}`, {
			waitUntil: "networkidle0",
		});
		await page.waitForSelector("[data-preview-ready]");
	};
	const clickText = async (selector, text) => {
		const element = await page.waitForFunction(
			(selector, text) =>
				[...document.querySelectorAll(selector)].find(
					(node) => node.textContent.trim() === text,
				),
			{},
			selector,
			text,
		);
		await element.asElement().click();
		await element.dispose();
	};
	const noOverflow = async (label) =>
		check(
			await page.evaluate(() => {
				const dialog = document.querySelector('[role="dialog"]');
				return (
					document.documentElement.scrollWidth <= innerWidth + 1 &&
					(!dialog || dialog.scrollWidth <= dialog.clientWidth + 1)
				);
			}),
			`${label} fits viewport`,
		);
	for (const width of [390, 768, 1440]) {
		await page.setViewport({ width, height: 900 });
		await visit("view=subscription&keep=preserved");
		await page.waitForSelector('[aria-label="Current plan"]');
		await page.evaluate(() => history.replaceState({ __NA: true }, ""));
		check(
			(await page.$('[aria-label="Current usage"]')) === null,
			"Plan tab omits usage contents",
		);
		await clickText("button", "View usage");
		await page.waitForSelector('[aria-label="Current usage"]');
		check(
			new URL(page.url()).searchParams.get("tab") === "usage",
			"Usage selection updates URL",
		);
		check(
			new URL(page.url()).searchParams.get("keep") === "preserved",
			"Tab changes preserve other query parameters",
		);
		check(
			await page.evaluate(() => history.state.__NA),
			"Next router state is retained",
		);
		check(
			await page.evaluate(
				() =>
					document.activeElement?.getAttribute("role") === "tab" &&
					document.activeElement.textContent === "Usage",
			),
			"View usage moves focus to selected tab",
		);
		check(
			(await page.$$eval(
				'[aria-label="Current usage"] article',
				(nodes) => nodes.filter((node) => node.checkVisibility()).length,
			)) === 3,
			"Three main usage meters are visible",
		);
		await page.keyboard.press("ArrowLeft");
		await page.waitForFunction(
			() => !document.querySelector('[aria-label="Current usage"]'),
		);
		check(
			!new URL(page.url()).searchParams.has("tab"),
			"Keyboard navigation selects Plan and billing",
		);
		await clickText("button", "Annual");
		check(
			await page.evaluate(() =>
				["€190", "€590", "€2,160"].every((value) =>
					document.body.innerText.includes(value),
				),
			),
			"Annual prices display correctly",
		);
		await clickText("summary", "Compare all limits");
		check(
			(await page.$$eval("details[open] tbody tr", (rows) => rows.length)) >= 8,
			"Full plan limits remain available",
		);
		await noOverflow(`Plan comparison at ${width}px`);
		await visit("view=subscription&tab=usage");
		await page.waitForSelector('[aria-label="Current usage"]');
		check(
			(await page.$eval(
				'[role="tab"][aria-selected="true"]',
				(node) => node.textContent,
			)) === "Usage",
			"Usage deep link selects the correct tab",
		);
	}
	await page.setViewport({ width: 390, height: 844 });
	await visit("view=upgrade");
	await page.waitForSelector('[data-testid="recommended-upgrade"]');
	check(
		await page.$eval('[data-testid="recommended-upgrade"] button', (node) => {
			const rect = node.getBoundingClientRect();
			return rect.top >= 0 && rect.bottom <= innerHeight && !node.disabled;
		}),
		"Mobile upgrade action is visible before scrolling",
	);
	check(
		await page.evaluate(() =>
			[...document.querySelectorAll('[role="dialog"] details')].every(
				(node) => !node.open,
			),
		),
		"Alternative plans and free tips begin collapsed",
	);
	await noOverflow("Mobile upgrade");
	await clickText("summary", "See other plans");
	await noOverflow("Expanded mobile upgrade");
	await page.keyboard.press("Escape");
	await page.waitForSelector('[role="dialog"]', { hidden: true });
	check(true, "Upgrade dialog closes with Escape");

	await visit("view=history");
	await clickText("summary", "Operation history and pending usage");
	await page.waitForFunction(() =>
		document.body.innerText.includes("Invoice Review"),
	);
	const before = await page.$eval(
		"tbody",
		(node) => node.getBoundingClientRect().height,
	);
	await clickText("button", "Cost and token details");
	await page.waitForSelector('[role="dialog"]');
	await page.waitForFunction(() =>
		document.querySelector('[role="dialog"]')?.textContent.includes("€0.0042"),
	);
	await noOverflow("Mobile operation details");
	check(
		(await page.$eval(
			"tbody",
			(node) => node.getBoundingClientRect().height,
		)) === before,
		"Details panel preserves table row heights",
	);
	await page.keyboard.press("Escape");
	await page.waitForSelector('[role="dialog"]', { hidden: true });
	check(
		await page.evaluate(() =>
			document.activeElement?.textContent.includes("Cost and token details"),
		),
		"Closing details restores focus to its trigger",
	);
	check(errors.length === 0, `No browser exceptions: ${errors.join("; ")}`);
	console.log(
		`Passed ${checks} browser interaction checks across mobile, tablet and desktop.`,
	);
} finally {
	await browser.close();
	await rm(profile, { recursive: true, force: true });
}
