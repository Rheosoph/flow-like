import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

export async function verifyOutfitBrowsing(page, { origin, output }) {
	const screenshots = resolve(output, "wardrobe");
	await mkdir(screenshots, { recursive: true });
	await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "no-preference" },
	]);
	await page.goto(`${origin}/?kind=mochi`, { waitUntil: "networkidle0" });
	await page.$$eval('[aria-label="Customization sections"] button', (buttons) =>
		buttons.find((button) => button.textContent.endsWith("Outfit"))?.click(),
	);
	assert.equal(
		await page.$$eval("[data-outfit-group]", (groups) => groups.length),
		24,
	);
	const beforeDraft = await page.$eval(
		"[data-appearance]",
		(element) => element.textContent,
	);
	const crown = '[aria-label="Choose Crown"]';
	const artwork = `${crown} [data-part="head"]`;
	const original = await page.$eval(artwork, (element) => element.innerHTML);
	await page.hover(crown);
	await page.waitForSelector(`${crown}[data-color-preview="0"]`);
	assert.notEqual(
		await page.$eval(artwork, (element) => element.innerHTML),
		original,
	);
	await page.screenshot({
		path: resolve(screenshots, "themed-outfits-desktop.png"),
	});
	await (await page.$('[data-outfit-group="essentials"]')).screenshot({
		path: resolve(screenshots, "hover-colors.png"),
	});
	await page.waitForSelector(`${crown}[data-color-preview="1"]`);
	assert.equal(
		await page.$eval("[data-appearance]", (element) => element.textContent),
		beforeDraft,
	);
	await page.mouse.move(0, 0);
	await page.waitForFunction(
		(selector) =>
			!document.querySelector(selector).hasAttribute("data-color-preview"),
		{},
		crown,
	);
	assert.equal(
		await page.$eval(artwork, (element) => element.innerHTML),
		original,
	);
	await page.focus(crown);
	await page.waitForSelector(`${crown}[data-color-preview="0"]`);
	await page.$eval(crown, (element) => element.blur());
	await page.waitForFunction(
		(selector) =>
			!document.querySelector(selector).hasAttribute("data-color-preview"),
		{},
		crown,
	);
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "reduce" },
	]);
	await page.hover(crown);
	await page.waitForSelector(`${crown}[data-color-preview="0"]`);
	await page.evaluate(
		() => new Promise((resolve) => setTimeout(resolve, 1650)),
	);
	assert.equal(
		await page.$eval(crown, (element) =>
			element.getAttribute("data-color-preview"),
		),
		"0",
	);
	await page.click(crown);
	const selected = await page.$eval("[data-appearance]", (element) =>
		JSON.parse(element.textContent),
	);
	assert.equal(selected.equipment.head, "crown");
	assert.equal(
		selected.colors,
		undefined,
		"Choosing an item must not persist preview colors",
	);
	await page.mouse.move(0, 0);
	await page.waitForFunction(
		(selector) =>
			!document.querySelector(selector).hasAttribute("data-color-preview"),
		{},
		crown,
	);
	await page.$$eval('[aria-label="Equipment slots"] button', (buttons) =>
		buttons.find((button) => button.textContent === "Clothing")?.click(),
	);
	const basicsClothes = await page.$eval(
		'[data-outfit-group="basics-crew"]',
		(group) => ({
			visible: group.querySelectorAll(".fl-bot-outfit-tile").length,
			listed: Number.parseInt(
				group.querySelector(".fl-bot-studio-outfit-heading > span").textContent,
				10,
			),
		}),
	);
	assert.ok(basicsClothes.listed > 6);
	assert.equal(
		basicsClothes.visible,
		basicsClothes.listed,
		"Theme groups must show every item",
	);
	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await page.$eval(".fl-bot-studio-panel", (panel) => panel.scrollIntoView());
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
		true,
	);
	await page.screenshot({
		path: resolve(screenshots, "themed-outfits-mobile.png"),
	});
}

export async function verifyWardrobe(page, { origin, output }) {
	const screenshots = resolve(output, "wardrobe");
	await mkdir(screenshots, { recursive: true });
	await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
	await page.goto(`${origin}/?wardrobe`, { waitUntil: "networkidle0" });
	const items = await page.$$eval("[data-equipment-id]", (figures) =>
		figures.map((figure) => figure.dataset.equipmentId),
	);
	assert.equal(
		new Set(items).size,
		items.length,
		"Every item should appear once in the wardrobe",
	);
	assert.ok(
		items.length > 400,
		"The expanded wardrobe should offer over four hundred items",
	);
	for (const kind of ["mochi", "ruler", "orbi", "drip", "bambu"]) {
		await page.select('[aria-label="Wardrobe character"]', kind);
		const failures = await page.$$eval("[data-equipment-id]", (figures) => {
			const failures = [];
			for (const figure of figures) {
				const { equipmentId: id, equipmentSlot: slot } = figure.dataset;
				const art = figure.querySelector(
					slot === "eyes" ? '[data-slot="eyes"]' : `[data-part="${slot}"]`,
				);
				if (!art?.childElementCount) failures.push(`${id}: missing artwork`);
				for (const element of art?.querySelectorAll("*") ?? []) {
					for (const attribute of element.attributes) {
						if (/NaN|undefined|Infinity/.test(attribute.value))
							failures.push(`${id}: invalid ${attribute.name}`);
						for (const reference of attribute.value.matchAll(
							/url\(#([^)]*)\)/g,
						)) {
							if (!document.getElementById(reference[1]))
								failures.push(`${id}: missing paint or clip`);
						}
					}
					if (
						element.tagName === "path" &&
						!Number.isFinite(element.getTotalLength())
					)
						failures.push(`${id}: invalid path`);
				}
			}
			return failures;
		});
		assert.deepEqual(failures, [], `Accessories must render on ${kind}`);
		if (kind === "mochi") {
			for (const section of await page.$$("[data-wardrobe-pack]")) {
				const id = await section.evaluate(
					(element) => element.dataset.wardrobePack,
				);
				await section.screenshot({ path: resolve(screenshots, `${id}.png`) });
			}
		}
	}
	await page.select('[aria-label="Wardrobe size"]', "32");
	const compactFailures = await page.$$eval(
		'[data-equipment-slot="hand"]',
		(figures) =>
			figures.flatMap((figure) => {
				const { equipmentId: id, compactSkill: skill } = figure.dataset;
				if (figure.querySelector('[data-part="hand"]')?.childElementCount)
					return [`${id}: hand art should hide at compact size`];
				if (skill && !figure.querySelector(`[data-skill="${skill}"]`))
					return [`${id}: missing compact skill`];
				return [];
			}),
	);
	assert.deepEqual(compactFailures, []);
	await page.goto(`${origin}/?kind=mochi`, { waitUntil: "networkidle0" });
	await page.$$eval('[aria-label="Customization sections"] button', (buttons) =>
		buttons.find((button) => button.textContent.endsWith("Outfit"))?.click(),
	);
	await page.type('[aria-label="Search outfits"]', "pirates");
	assert.ok(
		await page.$$eval(
			'.fl-bot-studio-panel [aria-label^="Choose "]',
			(tiles) => tiles.length > 1,
		),
	);
	await page.screenshot({
		path: resolve(screenshots, "outfit-search.png"),
		fullPage: true,
	});
	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
		true,
	);
	await page.screenshot({
		path: resolve(screenshots, "outfit-mobile.png"),
		fullPage: true,
	});
	await verifyOutfitBrowsing(page, { origin, output });
}
