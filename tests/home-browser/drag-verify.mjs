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
const widgets = () => page.locator("[data-home-widget]");
const order = () =>
	widgets().evaluateAll((elements) =>
		elements.map((element) => element.getAttribute("data-home-widget")),
	);
const waitOrder = (expected) =>
	page.waitForFunction(
		(value) =>
			JSON.stringify(
				[...document.querySelectorAll("[data-home-widget]")].map((element) =>
					element.getAttribute("data-home-widget"),
				),
			) === JSON.stringify(value),
		expected,
	);
const panelClose = async () => {
	const close = page.getByRole("button", {
		name: "Close widget panel",
		exact: true,
	});
	if (await close.count()) await close.last().click();
};
const widgetById = (id) => page.locator(`[data-home-widget="${id}"]`);
const save = async () => {
	await page.getByRole("button", { name: "Save", exact: true }).click();
	await page.locator('[data-home-editor][data-editing="false"]').waitFor();
};
const edit = async () => {
	await page.getByRole("button", { name: "Customize", exact: true }).click();
	await panelClose();
};
const settle = () => page.waitForTimeout(400);
const pickup = async (widget) => {
	await widget.scrollIntoViewIfNeeded();
	const source = await widget.boundingBox();
	const handle = widget.getByRole("button", { name: /^Move / });
	await handle.hover();
	const box = await handle.boundingBox();
	await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
	await page.mouse.down();
	await page.mouse.move(box.x + box.width / 2 + 12, box.y + box.height / 2, {
		steps: 4,
	});
	await page.locator("[data-home-drag-preview]").waitFor();
	const ghost = await page.locator("[data-home-drag-preview]").boundingBox();
	assert.ok(
		Math.abs(ghost.width - source.width) <= 2,
		`Ghost preserves source width ${ghost.width}/${source.width}`,
	);
	assert.ok(
		Math.abs(ghost.height - source.height) <= 2,
		`Ghost preserves source height ${ghost.height}/${source.height}`,
	);
	return source;
};
try {
	await page.goto("http://127.0.0.1:4318/?persist=1", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("button", { name: "Customize", exact: true })
		.waitFor({ timeout: 60_000 });
	await edit();
	const initial = await order();
	const shortWidget = widgetById(initial[1]);
	const tallWidget = widgetById(initial[2]);
	await pickup(shortWidget);
	const target = await tallWidget.boundingBox();
	await page.mouse.move(
		target.x + target.width * 0.7,
		target.y + target.height * 0.9,
		{ steps: 22 },
	);
	await settle();
	const preview = await order();
	assert.notDeepEqual(
		preview,
		initial,
		"Unequal pointer drag changes insertion order",
	);
	await settle();
	assert.deepEqual(
		await order(),
		preview,
		"Stationary pointer preserves the insertion slot",
	);
	await page.screenshot({ path: "/private/tmp/home-polish-unequal-drag.png" });
	await page.mouse.up();
	await page.locator("[data-home-drag-preview]").waitFor({ state: "hidden" });
	assert.deepEqual(
		await order(),
		preview,
		"Drop commits exactly the preview order",
	);
	report.passed.push(
		"Unequal widget drag preserves source geometry and commits its stable preview slot",
	);
	// The pointer sensor suppresses the release click for 50ms after a drop.
	await settle();
	await page.getByRole("button", { name: "Undo layout change" }).click();
	await waitOrder(initial);
	assert.deepEqual(await order(), initial);
	await pickup(tallWidget);
	const first = await widgetById(initial[0]).boundingBox();
	await page.mouse.move(
		first.x + first.width * 0.75,
		first.y + first.height * 0.3,
		{ steps: 24 },
	);
	await settle();
	await page.keyboard.press("Escape");
	await page.mouse.up();
	await waitOrder(initial);
	assert.deepEqual(
		await order(),
		initial,
		"Escape restores pointer drag order",
	);
	report.passed.push(
		"Pointer Escape restores the original order after moving a taller widget",
	);
	const info = widgetById(initial[3]);
	await info.scrollIntoViewIfNeeded();
	const natural = (await info.boundingBox()).height;
	const resize = info.getByRole("button", { name: /^Resize / });
	await resize.hover();
	const handle = await resize.boundingBox();
	await page.mouse.move(
		handle.x + handle.width / 2,
		handle.y + handle.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(
		handle.x + handle.width / 2,
		handle.y + handle.height / 2 + 184,
		{ steps: 18 },
	);
	await page.mouse.up();
	assert.equal(await info.getAttribute("data-height-mode"), "fixed");
	await save();
	const savedSize = await page.evaluate(
		(id) => window.homeQa.saved.widgets.find((widget) => widget.id === id).size,
		initial[3],
	);
	assert.equal(savedSize.heightMode, "fixed");
	assert.ok(savedSize.height > natural + 140);
	await page.reload({ waitUntil: "domcontentloaded" });
	await page.getByRole("button", { name: "Customize", exact: true }).waitFor();
	assert.equal(await info.getAttribute("data-height-mode"), "fixed");
	assert.ok(
		Math.abs((await info.boundingBox()).height - savedSize.height) <= 20,
	);
	await edit();
	await info.getByRole("button", { name: /^Configure / }).click();
	assert.equal(
		await page.getByLabel("Height", { exact: true }).inputValue(),
		"custom",
	);
	await page.getByLabel("Height", { exact: true }).selectOption("auto");
	await panelClose();
	await settle();
	assert.equal(await info.getAttribute("data-height-mode"), "auto");
	assert.ok(
		(await info.boundingBox()).height < savedSize.height - 100,
		"Fit content restores natural height",
	);
	await save();
	await page.reload({ waitUntil: "domcontentloaded" });
	await page.getByRole("button", { name: "Customize", exact: true }).waitFor();
	assert.equal(await info.getAttribute("data-height-mode"), "auto");
	assert.equal(
		await page.evaluate(
			(id) =>
				window.homeQa.saved.widgets.find((widget) => widget.id === id).size
					.height,
			initial[3],
		),
		undefined,
	);
	report.passed.push(
		"Pointer resize survives save/reload; Fit content clears fixed pixels and survives another reload",
	);
	await page.goto("http://127.0.0.1:4318/?showcase=personal", {
		waitUntil: "domcontentloaded",
	});
	await page.getByRole("button", { name: "Customize", exact: true }).waitFor();
	await edit();
	const galleryInitial = await order();
	await pickup(widgets().first());
	await page.mouse.move(740, 700, { steps: 20 });
	await page.mouse.wheel(0, 660);
	await page.waitForTimeout(250);
	await page.mouse.move(740, 440, { steps: 12 });
	await settle();
	const scrolled = await page
		.locator("[data-home-canvas]")
		.evaluate((element) => {
			let total = 0;
			for (
				let parent = element.parentElement;
				parent;
				parent = parent.parentElement
			)
				total += parent.scrollTop;
			return total;
		});
	assert.ok(scrolled > 100, "The canvas scrolls during a pointer drag");
	const scrolledPreview = await order();
	await settle();
	assert.deepEqual(
		await order(),
		scrolledPreview,
		"Scrolling settles into a stable insertion slot",
	);
	assert.notDeepEqual(scrolledPreview, galleryInitial);
	await page.screenshot({ path: "/private/tmp/home-polish-scroll-drag.png" });
	await page.mouse.up();
	assert.deepEqual(
		await order(),
		scrolledPreview,
		"Scrolled drop commits the visible order",
	);
	report.passed.push(
		"Dragging across a scrolled, nonuniform gallery keeps its insertion slot and commits the preview order",
	);
	await settle();
	await page.getByRole("button", { name: "Add widget", exact: true }).click();
	await page
		.getByRole("textbox", { name: "Search widgets", exact: true })
		.fill("Milestone");
	const catalogHandle = await page
		.getByRole("button", { name: "Drag Milestone to home", exact: true })
		.boundingBox();
	await page.mouse.move(
		catalogHandle.x + catalogHandle.width / 2,
		catalogHandle.y + catalogHandle.height / 2,
	);
	await page.mouse.down();
	await page.mouse.move(catalogHandle.x - 24, catalogHandle.y, { steps: 5 });
	await page.mouse.move(600, 420, { steps: 20 });
	await page.locator('[data-home-placeholder="active"]').waitFor();
	await page.locator("[data-home-drag-preview] [data-widget-type]").waitFor();
	await settle();
	const catalogGhost = await page
		.locator("[data-home-drag-preview]")
		.boundingBox();
	const catalogSlot = await page
		.locator('[data-home-placeholder="active"]')
		.boundingBox();
	assert.ok(
		Math.abs(catalogGhost.width - catalogSlot.width) <= 2,
		"Catalog ghost width matches the actual slot",
	);
	assert.ok(
		Math.abs(catalogGhost.height - catalogSlot.height) <= 2,
		"Catalog ghost height matches the actual slot",
	);
	const catalogPreview = await order();
	await settle();
	assert.deepEqual(await order(), catalogPreview);
	await page.screenshot({
		path: "/private/tmp/home-polish-catalog-scrolled-drag.png",
	});
	await page.mouse.up();
	await page.locator("[data-home-drag-preview]").waitFor({ state: "hidden" });
	assert.deepEqual(await order(), catalogPreview);
	report.passed.push(
		"A catalog widget uses its real content and slot dimensions, with a stable preview and matching committed order",
	);
	assert.deepEqual(report.errors, []);
	assert.deepEqual(report.blocked, []);
} catch (error) {
	report.failure = error.stack;
	await page.screenshot({ path: "/private/tmp/home-polish-drag-failure.png" });
	throw error;
} finally {
	await writeFile(
		"/private/tmp/home-polish-drag-report.json",
		JSON.stringify(report, null, 2),
	);
	console.log(JSON.stringify(report, null, 2));
	await browser.close();
}
