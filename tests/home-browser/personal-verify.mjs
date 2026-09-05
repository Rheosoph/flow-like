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
	args: ["--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const report = { passed: [], errors: [], blocked: [] };
page.on("pageerror", (error) => report.errors.push(error.message));
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
const widgets = () => page.locator("[data-home-widget]");
const section = (title) =>
	widgets().filter({
		has: page.getByRole("heading", { name: title, exact: true }),
	});
const scrollTo = async (target) => {
	await target.evaluate((element) =>
		element.scrollIntoView({ block: "start" }),
	);
	await page.waitForTimeout(300);
};
const overflow = async (width, theme) => {
	const result = await page.evaluate(() => ({
		page: document.documentElement.scrollWidth,
		width: innerWidth,
		canvas: [...document.querySelectorAll("[data-home-canvas]")].map(
			(element) => [element.clientWidth, element.scrollWidth],
		),
	}));
	assert.ok(
		result.page <= result.width + 1,
		`${width}px ${theme} page overflow`,
	);
	assert.ok(
		result.canvas.every(([client, scroll]) => scroll <= client + 1),
		`${width}px ${theme} canvas overflow`,
	);
};
try {
	await page.goto("http://127.0.0.1:4318/?showcase=personal", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	await section("A thoughtful start").waitFor();
	assert.equal(await widgets().count(), 19);
	for (const theme of ["dark", "light"]) {
		await page.evaluate(
			(mode) =>
				document.documentElement.classList.toggle("dark", mode === "dark"),
			theme,
		);
		for (const width of [1480, 768, 390, 320, 2048]) {
			await page.setViewportSize({ width, height: width < 768 ? 844 : 1050 });
			await scrollTo(widgets().first());
			await overflow(width, theme);
			await page.screenshot({
				path: `/private/tmp/home-polish-personal-${theme}-${width}-top.png`,
			});
			for (const [key, title] of [
				["content", "A thoughtful start"],
				["info", "From idea to useful app"],
				["bottom", "A few helpful facts"],
			]) {
				await scrollTo(section(title));
				await overflow(width, theme);
				if ([390, 1480].includes(width))
					await page.screenshot({
						path: `/private/tmp/home-polish-personal-${theme}-${width}-${key}.png`,
					});
			}
		}
	}
	report.passed.push(
		"All 19 personal widget examples fit the page and canvas at 320, 390, 768, 1480 and 2048 pixels in dark and light modes",
	);
	await page.setViewportSize({ width: 1480, height: 1050 });
	await page.evaluate(() => document.documentElement.classList.add("dark"));
	const checklist = section("A thoughtful start");
	await checklist
		.getByRole("checkbox", { name: "Try your first app", exact: true })
		.check();
	await checklist.getByText("2 of 3 complete", { exact: true }).waitFor();
	await section("A few useful answers")
		.getByText("Can I change this later?", { exact: true })
		.click();
	await section("A few useful answers")
		.getByText("You can add, move, and resize widgets whenever you need.", {
			exact: true,
		})
		.waitFor();
	report.passed.push(
		"Checklist updates persist through the real save callback and FAQ answers expand",
	);
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Close widget panel", exact: true })
		.last()
		.click();
	const composer = section("Make room for your next idea");
	await composer
		.getByRole("button", {
			name: "Configure Make room for your next idea",
			exact: true,
		})
		.click();
	await page.getByLabel("Height", { exact: true }).selectOption("2");
	await page
		.getByRole("button", { name: "Close widget panel", exact: true })
		.last()
		.click();
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	await page.waitForFunction(
		() => !document.querySelector('[data-sonner-toast][data-visible="true"]'),
	);
	await scrollTo(composer);
	const scrolled = await composer.evaluate((element) => {
		const container = [...element.querySelectorAll("div")].find(
			(node) =>
				getComputedStyle(node).overflowY === "auto" &&
				node.scrollHeight > node.clientHeight + 1,
		);
		if (!container) return false;
		container.scrollTop = container.scrollHeight;
		return container.scrollTop > 0;
	});
	assert.equal(
		scrolled,
		true,
		"A deliberately short composer has a working vertical scroll area",
	);
	const chip = composer.getByRole("button", {
		name: "Explain a node",
		exact: true,
	});
	const chipBox = await chip.boundingBox();
	const frameBox = await composer.boundingBox();
	assert.ok(
		chipBox.y >= frameBox.y &&
			chipBox.y + chipBox.height <= frameBox.y + frameBox.height + 1,
		"Bottom prompt chips remain reachable in a short composer",
	);
	await page.screenshot({
		path: "/private/tmp/home-polish-personal-fixed-composer.png",
	});
	report.passed.push(
		"A fixed two-row FlowPilot composer scrolls to its bottom prompt chips without clipping them",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = error.stack;
	await page.screenshot({
		path: "/private/tmp/home-polish-personal-failure.png",
	});
	throw error;
} finally {
	await writeFile(
		"/private/tmp/home-polish-personal-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
