import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const effects = [
	"pixel",
	"lcd",
	"crt",
	"glitch",
	"thermal",
	"riso",
	"raster",
	"comic",
	"watercolor",
	"blueprint",
	"sticker",
	"vinyl",
	"neon",
	"none",
	"offline",
];

async function inspectArtwork(page) {
	const failures = await page.$$eval(
		"[data-effects-preview] .fl-bot",
		(bots) => {
			const failures = [];
			const ids = new Set();
			for (const bot of bots) {
				const svg = bot.querySelector("svg");
				const name = `${bot.dataset.kind}/${bot.dataset.effect}`;
				if (!svg?.querySelector('[data-part="eyes"]')) {
					failures.push(`${name}: missing face`);
					continue;
				}
				for (const element of svg.querySelectorAll("*")) {
					if (element.id) {
						if (ids.has(element.id)) failures.push(`${name}: duplicate SVG ID`);
						ids.add(element.id);
					}
					for (const attribute of element.attributes) {
						if (/NaN|undefined|Infinity/.test(attribute.value))
							failures.push(`${name}: invalid ${attribute.name}`);
						for (const ref of attribute.value.matchAll(/url\(#([^)]*)\)/g)) {
							if (!svg.contains(document.getElementById(ref[1])))
								failures.push(`${name}: missing local ${ref[1]}`);
						}
					}
				}
			}
			return failures;
		},
	);
	assert.deepEqual(
		failures,
		[],
		"Every look must keep valid artwork and local SVG definitions",
	);
}

export async function verifyEffects(page, { origin, output }) {
	const screenshots = resolve(output, "effects");
	await mkdir(screenshots, { recursive: true });
	const previousViewport = page.viewport();
	const errors = [];
	const onError = (error) => errors.push(error.message);
	page.on("pageerror", onError);
	try {
		await page.setViewport({ width: 1540, height: 1200, deviceScaleFactor: 1 });
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		await page.goto(`${origin}/?effects&paused`, { waitUntil: "networkidle0" });
		await page.waitForSelector('[data-effect-card="pixel"] canvas');
		await page.waitForFunction(() =>
			[
				...document.querySelectorAll(
					'[data-effects-preview] .fl-bot[data-effect="pixel"]',
				),
			].every((bot) => bot.querySelector('svg[data-pixel-ready="true"]')),
		);
		assert.deepEqual(
			await page.$$eval("[data-effect-card]", (cards) =>
				cards.map((card) => card.dataset.effectCard),
			),
			effects,
		);
		const cardEffects = await page.$$eval("[data-effect-card]", (cards) =>
			cards.map((card) =>
				[...card.querySelectorAll(".fl-bot")].every(
					(bot) => bot.dataset.effect === card.dataset.effectCard,
				),
			),
		);
		assert.ok(
			cardEffects.every(Boolean),
			"Each look must apply to every character in its card",
		);
		const pixels = await page.$$eval("[data-pixel-size]", (cards) =>
			cards.map((card) => {
				const canvas = card.querySelector("canvas");
				return {
					size: Number(card.dataset.pixelSize),
					width: canvas.width,
					height: canvas.height,
					pixels: canvas.toDataURL(),
					rendering: getComputedStyle(canvas).imageRendering,
				};
			}),
		);
		assert.deepEqual(
			pixels.map((pixel) => pixel.size),
			[96, 64, 44, 28],
		);
		for (const pixel of pixels) {
			assert.equal(pixel.width, 24);
			assert.equal(pixel.height, 24);
			assert.equal(pixel.rendering, "pixelated");
			assert.equal(
				pixel.pixels,
				pixels[0].pixels,
				"Pixel art must retain the same grid at every display size",
			);
		}
		await inspectArtwork(page);
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"Paused effects must stop animation",
		);
		await page.screenshot({
			path: resolve(screenshots, "reference-sheet.png"),
			fullPage: true,
		});
		for (const effect of effects) {
			await page.select('[aria-label="Crew effect"]', effect);
			await page.waitForFunction(
				(effect) =>
					[...document.querySelectorAll("[data-effect-crew] .fl-bot")].every(
						(bot) => bot.dataset.effect === effect,
					),
				{},
				effect,
			);
			if (effect === "pixel")
				await page.waitForFunction(() =>
					[...document.querySelectorAll("[data-effect-crew] .fl-bot")].every(
						(bot) => bot.querySelector('svg[data-pixel-ready="true"]'),
					),
				);
			const crew = await page.$$eval("[data-effect-crew] .fl-bot", (bots) =>
				bots.map((bot) => ({
					expression: bot.dataset.expression,
					emote: bot
						.querySelector("[data-emote-icon]")
						?.getAttribute("data-emote-icon"),
					gear: ["head", "neck", "hand"].every(
						(part) =>
							bot.querySelector(`[data-part="${part}"]`)?.childElementCount > 0,
					),
				})),
			);
			assert.ok(
				crew.every(
					(bot) =>
						bot.expression === "wink" && bot.emote === "heart" && bot.gear,
				),
				`${effect}: expressions, emotes, and gear must survive effect changes`,
			);
			await inspectArtwork(page);
		}
		await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
		assert.equal(
			await page.evaluate(
				() => document.documentElement.scrollWidth > innerWidth,
			),
			false,
			"Effects must fit a phone viewport",
		);
		await page.screenshot({
			path: resolve(screenshots, "phone.png"),
			fullPage: true,
		});
		await page.click("[data-toggle-effects-motion]");
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "reduce" },
		]);
		await page.waitForFunction(() =>
			[...document.querySelectorAll("[data-effects-preview] .fl-bot")].every(
				(bot) => bot.dataset.motion === "off",
			),
		);
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"Reduced motion must stop every effect animation",
		);
		assert.deepEqual(errors, [], "Effects must not raise browser errors");
		console.log(
			`Effect checks passed: 15 looks, four groups, fixed 24×24 pixels, crew changes, phone layout, reduced motion. Screenshots: ${screenshots}`,
		);
	} finally {
		page.off("pageerror", onError);
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		if (previousViewport) await page.setViewport(previousViewport);
	}
}
