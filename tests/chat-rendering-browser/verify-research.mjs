import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { join } from "node:path";
import puppeteer from "puppeteer";

const origin = process.env.CHAT_PREVIEW_ORIGIN || "http://127.0.0.1:4341";
const output = process.env.RESEARCH_SCREENSHOT_DIR || tmpdir();
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_EXECUTABLE_PATH ||
		(process.platform === "darwin"
			? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
			: undefined),
	args: ["--no-sandbox"],
});
const page = await browser.newPage();
const errors = [];
const passed = [];
page.on("pageerror", (error) => errors.push(error.message));

const load = async (query = "") => {
	await page.goto(`${origin}/?fixture=research${query}`, {
		waitUntil: "networkidle0",
	});
	await page.waitForSelector("[data-fl-plan-group]");
};
const clickButton = async (label) => {
	for (const button of await page.$$("button")) {
		if (
			(await button.evaluate((element) => element.innerText)).trim() === label
		) {
			await button.click();
			return;
		}
	}
	throw new Error(`Button not found: ${label}`);
};
const rootTrigger = "[data-fl-plan-group] > button[aria-expanded]";
const sourceCount = () =>
	page.$$eval("[data-research-source]", (items) => items.length);
const noHorizontalOverflow = () =>
	page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);

try {
	await page.setViewport({ width: 1280, height: 1000, deviceScaleFactor: 1 });
	await load();
	await page.waitForSelector("[data-research-activity]");
	assert.equal(await sourceCount(), 8);
	assert.equal(
		await page.$$eval("[data-research-query]", (items) => items.length),
		3,
	);
	const researchText = await page.$eval(
		"[data-research-activity]",
		(element) => element.textContent,
	);
	for (const label of ["Read", "Reading", "Found", "Unavailable"])
		assert.ok(researchText.includes(label), `Missing state: ${label}`);
	assert.ok(
		researchText.includes("Panamera Komfort Langstrecke Vergleich BMW 7er"),
	);
	assert.ok(researchText.includes("BMW 7 Series long-distance road test"));
	const links = await page.$$eval("[data-research-activity] a", (items) =>
		items.map((item) => ({
			href: item.href,
			target: item.target,
			rel: item.rel,
			inButton: Boolean(item.closest("button")),
		})),
	);
	assert.equal(links.length, 8);
	assert.ok(
		links.every(
			(link) =>
				link.href.startsWith("https://") &&
				link.target === "_blank" &&
				link.rel.includes("noopener") &&
				!link.inButton,
		),
	);
	assert.ok(await noHorizontalOverflow());
	await page.screenshot({
		path: join(output, "research-desktop.png"),
		fullPage: true,
	});
	passed.push(
		"All searches and eight URLs render with read, found, active and unavailable states",
	);

	await page.click(rootTrigger);
	await page.waitForFunction(
		(selector) =>
			document.querySelector(selector)?.getAttribute("aria-expanded") ===
			"false",
		{},
		rootTrigger,
	);
	assert.equal(await page.$("[data-research-activity]"), null);
	const summary = await page.$eval(
		rootTrigger,
		(element) => element.textContent,
	);
	assert.ok(summary.includes("6"));
	await page.focus(rootTrigger);
	await page.keyboard.press("Space");
	await page.waitForSelector("[data-research-activity]");
	assert.equal(await sourceCount(), 8);
	passed.push(
		"Research collapses and reopens with the keyboard, retaining the website summary",
	);

	await clickButton("Finish research");
	await page.waitForFunction(
		(selector) =>
			document.querySelector(selector)?.getAttribute("aria-expanded") ===
			"false",
		{},
		rootTrigger,
	);
	await page.click(rootTrigger);
	await page.waitForSelector("[data-research-activity]");
	const finished = await page.$eval(
		"[data-research-activity]",
		(element) => element.textContent,
	);
	assert.ok(!finished.includes("Reading"));
	assert.ok(finished.includes("Unavailable"));
	assert.equal(await sourceCount(), 8);
	passed.push(
		"Completion collapses automatically and preserves every source and failed attempt",
	);

	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await load("&narrow");
	await page.waitForSelector("[data-research-activity]");
	assert.equal(await sourceCount(), 8);
	assert.ok(await noHorizontalOverflow());
	await page.screenshot({
		path: join(output, "research-mobile.png"),
		fullPage: true,
	});
	await page.focus('[aria-label="Sources"]');
	await page.keyboard.press("End");
	await page.waitForFunction(() => {
		const sources = document.querySelector('[aria-label="Sources"]');
		return (
			sources &&
			sources.scrollTop + sources.clientHeight >= sources.scrollHeight - 1
		);
	});
	assert.ok(
		await page.$eval("[data-research-source]:last-child", (element) => {
			const bounds = element.getBoundingClientRect();
			const viewport = element
				.closest('[aria-label="Sources"]')
				.getBoundingClientRect();
			return (
				bounds.bottom <= viewport.bottom + 1 && bounds.top >= viewport.top - 1
			);
		}),
	);
	passed.push(
		"Mobile renders without horizontal overflow and every source is reachable by keyboard",
	);
	await clickButton("Show all 8 pages");
	await page.waitForFunction(() => {
		const sources = document.querySelector('[aria-label="Sources"]');
		return sources && sources.clientHeight >= sources.scrollHeight - 1;
	});
	assert.ok(await noHorizontalOverflow());
	await clickButton("Show compact view");
	await page.waitForFunction(() => {
		const sources = document.querySelector('[aria-label="Sources"]');
		return sources && sources.clientHeight < sources.scrollHeight;
	});
	assert.equal(await sourceCount(), 8);
	passed.push(
		"Show all pages removes source scrolling and compact view restores it without losing pages",
	);

	await page.setViewport({ width: 1280, height: 1000, deviceScaleFactor: 1 });
	await load("&light&settled");
	assert.equal(
		await page.$eval(rootTrigger, (element) =>
			element.getAttribute("aria-expanded"),
		),
		"false",
	);
	await page.click(rootTrigger);
	await page.waitForSelector("[data-research-activity]");
	await page.screenshot({
		path: join(output, "research-light.png"),
		fullPage: true,
	});
	assert.equal(await sourceCount(), 8);
	await page.$$eval("[data-fl-plan-group] img", (items) => {
		for (const item of items) item.dispatchEvent(new Event("error"));
	});
	await page.waitForFunction(
		() => document.querySelectorAll("[data-fl-plan-group] img").length === 0,
	);
	assert.ok(await page.$("[data-research-activity] .lucide-globe"));
	passed.push("Settled history, light theme, and unavailable favicon fallback");

	assert.deepEqual(errors, []);
	console.log(
		JSON.stringify(
			{
				passed,
				screenshots: [
					"research-desktop.png",
					"research-mobile.png",
					"research-light.png",
				].map((name) => join(output, name)),
				pageErrors: errors,
			},
			null,
			2,
		),
	);
} catch (error) {
	console.error({
		pageErrors: errors,
		body: await page.evaluate(() => document.body.innerText),
	});
	throw error;
} finally {
	await browser.close();
}
