import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const browser = await chromium.launch({
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1050 } });
const errors = [];
const blocked = [];
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	blocked.push(url.origin);
	return route.abort();
});
page.on("pageerror", (error) => {
	errors.push(error.message);
	console.log("PAGE ERROR:", error.stack);
});
page.on("console", (message) => {
	if (message.type() === "error") {
		errors.push(message.text());
		console.log("CONSOLE ERROR:", message.text());
	}
});
const report = { passed: [], errors, blocked };
const widgets = () => page.locator("[data-home-widget]");
const order = () =>
	widgets().evaluateAll((elements) =>
		elements.map((element) => element.getAttribute("data-home-widget")),
	);
const closePanel = async () => {
	const button = page.getByRole("button", {
		name: "Close widget panel",
		exact: true,
	});
	if (await button.count()) await button.last().click();
};
const top = () =>
	page.locator("[data-home-canvas]").evaluate((element) => {
		let parent = element.parentElement;
		while (parent) {
			parent.scrollTop = 0;
			parent = parent.parentElement;
		}
	});
const overflow = async (width) => {
	const result = await page.evaluate(() => ({
		page: document.documentElement.scrollWidth > innerWidth,
		canvas: [...document.querySelectorAll("[data-home-canvas]")].map(
			(element) => ({
				client: element.clientWidth,
				scroll: element.scrollWidth,
			}),
		),
	}));
	assert.equal(result.page, false, `${width}px page must fit`);
	assert.ok(
		result.canvas.every((value) => value.scroll <= value.client + 1),
		`${width}px canvas must fit`,
	);
};
try {
	await page.goto("http://127.0.0.1:4318/", { waitUntil: "domcontentloaded" });
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	console.log("Loaded home editor");
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("button", { name: "Add Embed an app", exact: true })
		.click();
	await page.getByLabel("Title", { exact: true }).fill("Report A");
	await page
		.getByRole("checkbox", { name: "Knowledge Chat", exact: true })
		.check();
	await page.getByLabel("Open", { exact: true }).selectOption("route");
	await page.getByLabel("Page route", { exact: true }).fill("/reports");
	await page
		.getByLabel("Query parameters", { exact: true })
		.fill("period=month&team=A%26B");
	await page.getByLabel("Width", { exact: true }).selectOption("6");
	await page.getByRole("button", { name: "Add widget", exact: true }).click();
	await page
		.getByRole("button", { name: "Add Embed an app", exact: true })
		.click();
	await page.getByLabel("Title", { exact: true }).fill("Report B");
	await page
		.getByRole("checkbox", { name: "Knowledge Chat", exact: true })
		.check();
	await page.getByLabel("Open", { exact: true }).selectOption("route");
	await page.getByLabel("Page route", { exact: true }).fill("/reports");
	await page
		.getByLabel("Query parameters", { exact: true })
		.fill("period=week&team=other");
	await page.getByLabel("Width", { exact: true }).selectOption("6");
	assert.equal(
		await page.evaluate(() => window.homeQa.counters.bootstrap),
		0,
		"Edit mode must not load or run embedded app pages",
	);
	report.passed.push(
		"App picker, route picker, query configuration, two embeds, and no native runtime in edit mode",
	);
	console.log("Configured two app embeds");
	await page.screenshot({
		path: "/private/tmp/home-qa-editor-desktop.png",
		fullPage: true,
	});
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	console.log(
		"Saved home editor",
		await page.evaluate(() => window.homeQa.counters),
	);
	const embeds = page.locator('[data-widget-type="app-embed"]');
	await embeds
		.nth(0)
		.getByText("Fixture Reports", { exact: true })
		.waitFor({ timeout: 60_000 });
	await embeds
		.nth(1)
		.getByText("Fixture Reports", { exact: true })
		.waitFor({ timeout: 60_000 });
	assert.equal(await page.evaluate(() => window.homeQa.counters.saves), 1);
	const hrefA = await embeds
		.nth(0)
		.getByRole("link", { name: "Open app" })
		.getAttribute("href");
	const hrefB = await embeds
		.nth(1)
		.getByRole("link", { name: "Open app" })
		.getAttribute("href");
	assert.equal(
		new URL(hrefA, "http://fixture").searchParams.get("team"),
		"A&B",
	);
	assert.equal(
		new URL(hrefB, "http://fixture").searchParams.get("team"),
		"other",
	);
	await embeds
		.nth(0)
		.getByRole("button", { name: "Show details in this widget" })
		.click();
	await embeds.nth(0).getByText("Fixture Details", { exact: true }).waitFor();
	assert.equal(
		await embeds.nth(1).getByText("Fixture Reports", { exact: true }).count(),
		1,
	);
	assert.equal(
		await embeds
			.nth(1)
			.getByRole("link", { name: "Open app" })
			.getAttribute("href"),
		hrefB,
	);
	assert.equal(page.url(), "http://127.0.0.1:4318/");
	assert.equal(
		new URL(
			await embeds
				.nth(0)
				.getByRole("link", { name: "Open app" })
				.getAttribute("href"),
			"http://fixture",
		).searchParams.get("item"),
		"42",
	);
	report.passed.push(
		"Saved production native page runtime and independent route/query navigation without host URL changes",
	);
	await embeds.nth(0).scrollIntoViewIfNeeded();
	await page.screenshot({
		path: "/private/tmp/home-qa-embeds-desktop.png",
		fullPage: true,
	});

	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await closePanel();
	await top();
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
	const originalOrder = await order();
	const firstMove = widgets()
		.first()
		.getByRole("button", { name: /^Move / });
	await firstMove.focus();
	await page.keyboard.press("Space");
	await page.waitForFunction(
		() =>
			!!document.querySelector(
				'button[aria-pressed="true"][aria-roledescription="sortable"]',
			),
	);
	await page.keyboard.press("ArrowDown");
	await page.waitForTimeout(250);
	await page.keyboard.press("Space");
	await page.waitForFunction(
		(original) =>
			document
				.querySelector("[data-home-widget]")
				?.getAttribute("data-home-widget") !== original,
		originalOrder[0],
	);
	assert.notDeepEqual(await order(), originalOrder);
	await page.getByRole("button", { name: "Undo layout change" }).click();
	assert.deepEqual(await order(), originalOrder);
	await page.getByRole("button", { name: "Redo layout change" }).click();
	assert.notDeepEqual(await order(), originalOrder);
	await page.getByRole("button", { name: "Undo layout change" }).click();
	report.passed.push(
		"Keyboard drag reorder and undo/redo restore exact widget order",
	);
	await page
		.getByRole("button", { name: "Resize Report A", exact: true })
		.focus();
	await page.keyboard.press("ArrowRight");
	await embeds.nth(0).getByText("7 × 6", { exact: true }).waitFor();
	await page.getByRole("button", { name: "Undo layout change" }).click();
	await embeds.nth(0).getByText("6 × 6", { exact: true }).waitFor();
	report.passed.push("Keyboard resize changes width and undo restores size");
	await page.getByRole("button", { name: "Layout options" }).click();
	await page.getByRole("menuitem", { name: "Reset to default" }).click();
	await page.getByRole("button", { name: "Reset draft", exact: true }).click();
	assert.equal(await widgets().count(), 4);
	await closePanel();
	await page.getByRole("button", { name: "Undo layout change" }).click();
	assert.equal(await widgets().count(), 6);
	await page.getByRole("button", { name: "Cancel", exact: true }).click();
	await page
		.getByRole("button", { name: "Discard changes", exact: true })
		.click();
	assert.equal(await widgets().count(), 6);
	assert.equal(await page.evaluate(() => window.homeQa.counters.resets), 0);
	report.passed.push(
		"Reset draft is undoable and cancel preserves saved home without writing reset",
	);
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await closePanel();
	await page
		.getByRole("button", { name: "Configure Report A", exact: true })
		.click();
	await page.getByLabel("Title", { exact: true }).fill("Report A revised");
	await page.evaluate(() => {
		window.homeQa.failSave = true;
	});
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page
		.getByText("Fixture save failed. Your changes are still here.", {
			exact: true,
		})
		.waitFor();
	assert.equal(
		await page.getByLabel("Title", { exact: true }).inputValue(),
		"Report A revised",
	);
	await page.evaluate(() => window.homeQa.remount());
	await page
		.getByRole("button", { name: "Resume editing", exact: true })
		.click();
	await closePanel();
	await page
		.getByRole("button", { name: "Configure Report A revised", exact: true })
		.click();
	assert.equal(
		await page.getByLabel("Title", { exact: true }).inputValue(),
		"Report A revised",
	);
	await page.evaluate(() => {
		window.homeQa.failSave = false;
	});
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.equal(
		await page.evaluate(
			() =>
				window.homeQa
					.getSaved()
					.widgets.find((widget) => widget.title === "Report A revised")?.config
					.query,
		),
		"period=month&team=A%26B",
	);
	report.passed.push(
		"Failed save retains changes, remount restores profile draft, retry saves it",
	);
	console.log(
		"Keyboard, history, reset, discard, save failure, and draft restore passed",
	);
	await page.waitForFunction(
		() => !document.querySelector('[data-sonner-toast][data-visible="true"]'),
	);
	for (const width of [1480, 768, 390]) {
		await page.setViewportSize({ width, height: width === 390 ? 844 : 1050 });
		await top();
		await overflow(width);
		await page.screenshot({
			path: `/private/tmp/home-qa-view-${width}.png`,
			fullPage: true,
		});
	}
	report.passed.push(
		"1480px, 768px, and 390px viewports have no page or canvas horizontal overflow",
	);
	const firstApp = page
		.locator('[data-widget-type="app-collection"] a')
		.first();
	assert.ok(
		(await firstApp.boundingBox()).height >= 64,
		"Mobile app cards keep readable row height",
	);
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("dialog", { name: "Widget catalog", exact: true })
		.waitFor();
	for (let index = 0; index < 20; index++) {
		await page.keyboard.press("Tab");
		assert.equal(
			await page.evaluate(
				() => !!document.activeElement?.closest('[role="dialog"]'),
			),
			true,
			"Mobile focus must remain in widget dialog",
		);
	}
	await page.keyboard.press("Escape");
	await page.getByRole("dialog").waitFor({ state: "hidden" });
	await page
		.getByRole("button", { name: "Configure Report A revised", exact: true })
		.click();
	await page
		.getByRole("dialog", { name: "Widget settings", exact: true })
		.waitFor();
	await page.screenshot({
		path: "/private/tmp/home-qa-editor-mobile.png",
		fullPage: true,
	});
	await page.keyboard.press("Escape");
	await page.getByRole("dialog").waitFor({ state: "hidden" });
	assert.equal(
		await page.evaluate(() =>
			document.activeElement?.getAttribute("aria-label"),
		),
		"Configure Report A revised",
	);
	report.passed.push(
		"Mobile widget catalog traps keyboard focus; Escape closes settings and restores trigger focus",
	);
	await page.getByRole("button", { name: "Cancel", exact: true }).click();
	await page.setViewportSize({ width: 1480, height: 1050 });
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await closePanel();
	await page.getByRole("button", { name: "Layout options" }).click();
	await page.getByRole("menuitem", { name: "Reset to default" }).click();
	await page.getByRole("button", { name: "Reset draft", exact: true }).click();
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.equal(await widgets().count(), 4);
	assert.equal(await page.evaluate(() => window.homeQa.counters.resets), 1);
	report.passed.push("Save after reset calls the inheritance reset callback");
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await page
		.getByRole("textbox", { name: "Search widgets", exact: true })
		.fill("Milestone");
	const dragHandle = page.getByRole("button", {
		name: "Drag Milestone to home",
		exact: true,
	});
	const from = await dragHandle.boundingBox();
	const into = await widgets().first().boundingBox();
	await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
	await page.mouse.down();
	await page.mouse.move(from.x - 30, from.y, { steps: 5 });
	await page.mouse.move(into.x + into.width / 2, into.y + into.height / 2, {
		steps: 15,
	});
	await page.screenshot({
		path: "/private/tmp/home-qa-catalog-drag.png",
		fullPage: true,
	});
	await page.mouse.up();
	await page
		.getByRole("button", { name: "Configure Milestone", exact: true })
		.waitFor();
	assert.equal(await widgets().count(), 5);
	await closePanel();
	const resizeHandle = page.getByRole("button", {
		name: "Resize Milestone",
		exact: true,
	});
	await resizeHandle.scrollIntoViewIfNeeded();
	const resizeFrom = await resizeHandle.boundingBox();
	const canvasWidth = await page
		.locator("[data-home-canvas]")
		.evaluate((element) => element.clientWidth);
	const gridColumns = Number(
		await page.locator("[data-home-canvas]").getAttribute("data-grid-columns"),
	);
	await page.mouse.move(
		resizeFrom.x + resizeFrom.width / 2,
		resizeFrom.y + resizeFrom.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(
		resizeFrom.x + resizeFrom.width / 2 + (canvasWidth + 16) / gridColumns,
		resizeFrom.y + resizeFrom.height / 2 + 136,
		{ steps: 12 },
	);
	await page.mouse.up();
	const resized = widgets().filter({
		has: page.getByRole("button", { name: "Configure Milestone", exact: true }),
	});
	await resized.getByText("5 × 3", { exact: true }).waitFor();
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
	assert.deepEqual(
		await page.evaluate(
			() =>
				window.homeQa
					.getSaved()
					.widgets.find((widget) => widget.title === "Milestone").size,
		),
		{ columns: 5, rows: 3 },
	);
	report.passed.push(
		"Pointer catalog drag adds a widget; corner resize commits width and height when saved",
	);
	assert.deepEqual(
		errors,
		[],
		"Production editor and native runtime must not emit browser errors",
	);
} catch (error) {
	report.failure = String(error.stack ?? error);
	console.log(report.failure);
	await page.screenshot({
		path: "/private/tmp/home-qa-failure.png",
		fullPage: true,
	});
	console.log((await page.locator("body").innerText()).slice(-7000));
} finally {
	await writeFile(
		"/private/tmp/home-qa-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
if (report.failure) process.exitCode = 1;
