import assert from "node:assert/strict";
import { resolve } from "node:path";
import sharp from "sharp";

async function colorsSection(page) {
	await page.$$eval('[aria-label="Customization sections"] button', (buttons) =>
		buttons.find((button) => button.textContent.endsWith("Colors"))?.click(),
	);
	await page.waitForSelector('[aria-label="Edit Body color"]');
}

async function appearance(page) {
	return page.$eval("[data-appearance]", (element) =>
		JSON.parse(element.textContent),
	);
}

async function pick(page, label) {
	const selector = `[aria-label="${label}"]`;
	await page.waitForSelector(selector);
	if (
		/^Edit .* color$/.test(label) &&
		(await page.$eval(
			selector,
			(button) => button.getAttribute("aria-expanded") === "true",
		))
	)
		return;
	await page.click(selector);
}

export async function verifyColors(page, origin, output) {
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "no-preference" },
	]);
	await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
	await page.goto(`${origin}/?kind=mochi`, { waitUntil: "networkidle0" });
	await colorsSection(page);
	assert.equal(
		await page.$$eval(
			'button[aria-label^="Edit "][aria-label$=" color"]',
			(buttons) => buttons.length,
		),
		7,
	);
	await pick(page, "Edit Body color");
	await pick(page, "Essentials color pack");
	await pick(page, "Choose Coral for Body");
	assert.deepEqual((await appearance(page)).colors.body, {
		kind: "swatch",
		colorId: "color-coral",
	});
	await pick(page, "Save bot");
	await page.waitForFunction(
		() =>
			JSON.parse(document.querySelector("[data-saved-appearance]").textContent)
				?.colors?.body?.colorId === "color-coral",
	);
	await pick(page, "Holographic color pack");
	assert.equal(
		await page.$$eval(".fl-bot-color-packs small", (labels) =>
			labels.every((label) => label.textContent === "Included"),
		),
		true,
	);
	assert.equal(await page.$(".fl-bot-color-lock-note"), null);
	assert.equal(await page.$("[data-preview-only]"), null);
	await pick(page, "Choose Aurora for Body");
	assert.equal((await appearance(page)).colors.body.colorId, "holo-aurora");
	assert.equal(
		await page.$eval('[aria-label="Save bot"]', (button) => button.disabled),
		false,
	);
	await pick(page, "Save bot");
	await page.waitForFunction(
		() =>
			JSON.parse(document.querySelector("[data-saved-appearance]").textContent)
				?.colors?.body?.colorId === "holo-aurora",
	);
	await page.waitForSelector(
		'.fl-bot-studio-stage [data-paint-slot="body"][data-paint-kind="gradient"]',
	);
	await pick(page, "View Holographic color pack");
	assert.match(
		await page.$eval("[data-requested-pack]", (element) => element.textContent),
		/colors-holographic/,
	);
	await pick(page, "Color lab color pack");
	const defaultHex = '[aria-label="Hex color for Body"]';
	await page.focus(defaultHex);
	await page.$eval(defaultHex, (input) => input.select());
	await page.keyboard.type("#bada55");
	await page.$$eval("button", (buttons) =>
		buttons
			.find((button) => button.textContent.trim() === "Apply custom color")
			?.click(),
	);
	assert.deepEqual((await appearance(page)).colors.body, {
		kind: "hex",
		value: "#BADA55",
		packId: "color-lab",
	});
	assert.equal(
		await page.$eval('[aria-label="Save bot"]', (button) => button.disabled),
		false,
	);
	assert.equal(await page.$(".fl-bot-color-lock-note"), null);
	assert.equal(await page.$('[aria-label="Save requirements"]'), null);
	await pick(page, "Save bot");
	await page.waitForFunction(
		() =>
			JSON.parse(document.querySelector("[data-saved-appearance]").textContent)
				?.colors?.body?.value === "#BADA55",
	);
	await page.screenshot({
		path: resolve(output, "colors-default-open.png"),
		fullPage: true,
	});
	await pick(page, "Reset Body color");
	assert.equal(
		await page.$eval('[aria-label="Save bot"]', (button) => button.disabled),
		false,
	);

	await page.goto(`${origin}/?kind=mochi&owned=all`, {
		waitUntil: "networkidle0",
	});
	await colorsSection(page);
	await pick(page, "Edit Body color");
	await pick(page, "Holographic color pack");
	await pick(page, "Choose Aurora for Body");
	assert.equal((await appearance(page)).colors.body.colorId, "holo-aurora");
	await page.screenshot({
		path: resolve(output, "colors-holographic.png"),
		fullPage: true,
	});
	await pick(page, "Edit Background color");
	await pick(page, "Essentials color pack");
	await pick(page, "Choose Cream for Background");
	await pick(page, "Edit Body color");
	await pick(page, "Patterns color pack");
	await pick(page, "Choose Starry night for Body");
	assert.equal((await appearance(page)).colors.body.colorId, "pattern-stars");
	await page.screenshot({
		path: resolve(output, "colors-patterns.png"),
		fullPage: true,
	});
	await pick(page, "Color lab color pack");
	const hexSelector = '[aria-label="Hex color for Body"]';
	await page.focus(hexSelector);
	await page.$eval(hexSelector, (input) => input.select());
	await page.keyboard.type("#GGGGGG");
	const beforeInvalid = await appearance(page);
	await page.$$eval("button", (buttons) =>
		buttons
			.find((button) => button.textContent.trim() === "Apply custom color")
			?.click(),
	);
	assert.deepEqual(await appearance(page), beforeInvalid);
	await page.$eval(hexSelector, (input) => input.select());
	await page.keyboard.type("#3a9");
	await page.$$eval("button", (buttons) =>
		buttons
			.find((button) => button.textContent.trim() === "Apply custom color")
			?.click(),
	);
	assert.deepEqual((await appearance(page)).colors.body, {
		kind: "hex",
		value: "#33AA99",
		packId: "color-lab",
	});
	await pick(page, "Save bot");
	await page.waitForFunction(
		() =>
			JSON.parse(document.querySelector("[data-saved-appearance]").textContent)
				?.colors?.body?.value === "#33AA99",
	);
	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await page.screenshot({
		path: resolve(output, "colors-mobile.png"),
		fullPage: true,
	});
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
		true,
	);
	await pick(page, "Reset Body color");
	assert.equal((await appearance(page)).colors?.body, undefined);
	assert.equal(
		(await appearance(page)).colors.background.colorId,
		"color-cream",
	);
	await page.$$eval("button", (buttons) =>
		buttons
			.find((button) => button.textContent.trim() === "Reset colors")
			?.click(),
	);
	assert.deepEqual((await appearance(page)).colors ?? {}, {});

	await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
	await page.goto(`${origin}/?paints&paused`, { waitUntil: "networkidle0" });
	await page.waitForSelector('[data-paint-probe="solid"] .fl-bot');
	const ids = await page.$$eval("svg [id]", (elements) =>
		elements.map((element) => element.id),
	);
	assert.equal(
		new Set(ids).size,
		ids.length,
		"Paint definitions need unique IDs in every avatar",
	);
	assert.equal(
		await page.$$eval("stop", (stops) =>
			stops.some((stop) => stop.getAttribute("stop-color")?.includes("url(")),
		),
		false,
	);
	const solid = await page.$('[data-paint-probe="solid"] .fl-bot');
	const pixels = await sharp(Buffer.from(await solid.screenshot()))
		.removeAlpha()
		.raw()
		.toBuffer({ resolveWithObject: true });
	const at =
		(Math.floor(pixels.info.height * 0.45) * pixels.info.width +
			Math.floor(pixels.info.width * 0.5)) *
		pixels.info.channels;
	assert.deepEqual(
		[...pixels.data.subarray(at, at + 3)],
		[241, 140, 128],
		"Essentials must paint the selected body color",
	);
	assert.equal(
		await page.evaluate(() => document.getAnimations().length),
		0,
		"Paused paint previews must remain still",
	);
	await page.screenshot({
		path: resolve(output, "color-paints.png"),
		fullPage: true,
	});
	await page.goto(`${origin}/?paints`, { waitUntil: "networkidle0" });
	await page.waitForSelector(
		'[data-paint-probe="holo"] .fl-bot[data-motion="on"]',
	);
	const gradientFrames = await page.$eval(
		'[data-paint-probe="holo"] .fl-bot',
		(bot) => {
			const stops = [...bot.querySelectorAll(".fl-bot-paint-stop")];
			const animations = bot.getAnimations({ subtree: true });
			return [0, 0.25, 0.5, 0.75].map((progress) => {
				for (const animation of animations) {
					animation.pause();
					animation.currentTime =
						Number(animation.effect.getTiming().duration) * progress;
				}
				return stops.map((stop) => getComputedStyle(stop).stopColor).join(";");
			});
		},
	);
	assert.ok(
		new Set(gradientFrames).size > 1,
		"Holographic paints should animate their color stops",
	);
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "reduce" },
	]);
	await page.waitForFunction(() => document.getAnimations().length === 0);
}
