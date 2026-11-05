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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const load = async (query = "") => {
	await page.goto(`${origin}/?fixture=research${query}`, {
		waitUntil: "networkidle0",
	});
	await page.waitForSelector("[data-fl-plan-group]");
};
const findButton = async (label) => {
	for (const button of await page.$$("button")) {
		if (
			(await button.evaluate((element) => element.innerText)).trim() === label
		)
			return button;
	}
	throw new Error(`Button not found: ${label}`);
};
const clickButton = async (label) => (await findButton(label)).click();
const rootTrigger = "[data-fl-plan-group] > button[aria-expanded]";
const sourceCount = () =>
	page.$$eval("[data-research-source]", (items) => items.length);
const queryCount = () =>
	page.$$eval("[data-research-query]", (items) => items.length);
const researchText = () =>
	page.$eval("[data-research-activity]", (element) => element.textContent);
const noHorizontalOverflow = () =>
	page.evaluate(() => document.documentElement.scrollWidth <= innerWidth);
const nestedScrollers = () =>
	page.$$eval("[data-research-activity] *", (elements) =>
		elements
			.filter(
				(element) =>
					/(auto|scroll)/.test(getComputedStyle(element).overflowY) &&
					element.scrollHeight > element.clientHeight,
			)
			.map((element) => element.getAttribute("aria-label") ?? element.tagName),
	);
const waitForSourceCount = (count) =>
	page.waitForFunction(
		(expected) =>
			document.querySelectorAll("[data-research-source]").length === expected,
		{},
		count,
	);

try {
	await page.setViewport({ width: 1280, height: 1000, deviceScaleFactor: 1 });
	await load();
	await page.waitForSelector("[data-research-activity]");
	assert.equal(await queryCount(), 4);
	assert.equal(await sourceCount(), 4);
	const compact = await researchText();
	for (const label of ["Read", "Found", "Show all 8 pages"])
		assert.ok(compact.includes(label), `Missing in compact view: ${label}`);
	assert.ok(compact.includes("Panamera Komfort Langstrecke Vergleich BMW 7er"));
	assert.ok(compact.includes("06"), "Visible searches keep their position");
	assert.ok(!compact.includes("Panamera vs BMW 5 Series"));
	await clickButton("Show 2 earlier searches");
	assert.equal(await queryCount(), 6);
	assert.ok(
		(await researchText()).includes(
			"Porsche Panamera vs BMW 5 Series 7 Series ride comfort",
		),
	);
	await clickButton("Hide earlier searches");
	assert.equal(await queryCount(), 4);
	await clickButton("Show all 8 pages");
	await waitForSourceCount(8);
	const expanded = await researchText();
	for (const label of ["Read", "Reading", "Found", "Unavailable"])
		assert.ok(expanded.includes(label), `Missing state: ${label}`);
	assert.ok(expanded.includes("BMW 7 Series long-distance road test"));
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
	assert.deepEqual(await nestedScrollers(), []);
	assert.ok(await noHorizontalOverflow());
	await page.screenshot({
		path: join(output, "research-desktop.png"),
		fullPage: true,
	});
	passed.push(
		"Searches and sources fold to a preview and expand to every query, URL and state",
	);

	await page.evaluate(() => {
		const transcript = document.createElement("div");
		transcript.style.height = "3000px";
		document.body.append(transcript);
	});
	const sourcesTop = await page.$eval(
		'[aria-label="Sources"]',
		(element) => element.getBoundingClientRect().top + scrollY,
	);
	await page.evaluate(
		(top) => scrollTo({ top: top - 100, behavior: "instant" }),
		sourcesTop,
	);
	await sleep(100);
	const pointer = await page.$eval('[aria-label="Sources"]', (element) => {
		const bounds = element.getBoundingClientRect();
		return { x: bounds.x + bounds.width / 2, y: bounds.y + 60 };
	});
	const beforeWheel = await page.evaluate(() => scrollY);
	await page.mouse.move(pointer.x, pointer.y);
	for (let tick = 0; tick < 10; tick++) {
		await page.mouse.wheel({ deltaY: 100 });
		await sleep(40);
	}
	await page.waitForFunction(
		(start) => scrollY >= start + 900,
		{},
		beforeWheel,
	);
	passed.push("Wheel over the research trail scrolls the transcript");

	await load();
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
	assert.equal(await sourceCount(), 4);
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
	await clickButton("Show all 8 pages");
	await waitForSourceCount(8);
	const finished = await researchText();
	assert.ok(!finished.includes("Reading"));
	assert.ok(finished.includes("Unavailable"));
	passed.push(
		"Completion collapses automatically and preserves every source and failed attempt",
	);

	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await load("&narrow");
	await page.waitForSelector("[data-research-activity]");
	assert.equal(await sourceCount(), 4);
	assert.deepEqual(await nestedScrollers(), []);
	assert.ok(await noHorizontalOverflow());
	await page.screenshot({
		path: join(output, "research-mobile.png"),
		fullPage: true,
	});
	await clickButton("Show all 8 pages");
	await waitForSourceCount(8);
	assert.ok(await noHorizontalOverflow());
	const fold = await findButton("Show fewer pages");
	await fold.evaluate((element) =>
		element.scrollIntoView({ block: "end", behavior: "instant" }),
	);
	const foldTop = await fold.evaluate(
		(element) => element.getBoundingClientRect().top,
	);
	await fold.click();
	await waitForSourceCount(4);
	const foldedTop = await (await findButton("Show all 8 pages")).evaluate(
		(element) => element.getBoundingClientRect().top,
	);
	assert.ok(
		Math.abs(foldedTop - foldTop) <= 1,
		`Fold toggle moved from ${foldTop} to ${foldedTop}`,
	);
	passed.push(
		"Mobile renders without nested scrolling and folding keeps the toggle in place",
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
	assert.equal(await sourceCount(), 4);
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
