import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import sharp from "sharp";
import { verifyColors } from "./color-checks.mjs";
import { verifyEffects } from "./effects-checks.mjs";
import { verifyEquipmentFits } from "./equipment-fit-checks.mjs";
import { verifyClothingFits } from "./fit-checks.mjs";
import { verifyGallery } from "./gallery-checks.mjs";
import { verifyMoods } from "./moods-checks.mjs";
import { verifyPixelRenderer } from "./pixel-checks.mjs";
import { verifyWardrobe } from "./wardrobe-checks.mjs";

const root = fileURLToPath(new URL("../..", import.meta.url));
const port = Number(process.env.BOT_TEST_PORT ?? 4350);
assert.ok(Number.isInteger(port) && port > 0 && port < 65536);
const origin = `http://127.0.0.1:${port}`;
const output = resolve(
	process.env.BOT_TEST_OUTPUT ?? `${tmpdir()}/flow-like-bot-browser`,
);
await mkdir(output, { recursive: true });
const server = spawn(
	process.execPath,
	[
		"node_modules/vite/bin/vite.js",
		"--config",
		"tests/bot-avatar-browser/vite.config.ts",
		"--port",
		String(port),
	],
	{ cwd: root, stdio: ["ignore", "pipe", "pipe"] },
);
let serverLog = "";
server.stdout.on("data", (chunk) => {
	serverLog += chunk;
});
server.stderr.on("data", (chunk) => {
	serverLog += chunk;
});
let browser;

async function pixels(element, name) {
	assert.ok(element, `Missing ${name}`);
	const screenshot = await element.screenshot({
		path: resolve(output, `${name}.png`),
	});
	return sharp(Buffer.from(screenshot))
		.removeAlpha()
		.raw()
		.toBuffer({ resolveWithObject: true });
}

function pixel(image, x, y) {
	const offset =
		(Math.floor((y * image.info.height) / 100) * image.info.width +
			Math.floor((x * image.info.width) / 100)) *
		image.info.channels;
	return Array.from(image.data.subarray(offset, offset + 3));
}

function brightness(rgb) {
	return rgb.reduce((sum, value) => sum + value, 0) / 3;
}

try {
	const deadline = Date.now() + 15000;
	while (true) {
		if (server.exitCode !== null) throw new Error(serverLog);
		try {
			if ((await fetch(origin)).ok) break;
		} catch {}
		if (Date.now() > deadline)
			throw new Error(`Preview did not start: ${serverLog}`);
		await new Promise((resolve) => setTimeout(resolve, 100));
	}
	const systemChrome =
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
	browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ??
			(existsSync(systemChrome) ? systemChrome : puppeteer.executablePath()),
	});
	const page = await browser.newPage();
	page.setDefaultTimeout(10000);
	const errors = [];
	page.on("pageerror", (error) => errors.push(error.message));
	await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
	await page.goto(`${origin}/?probes`, { waitUntil: "networkidle0" });
	await page.waitForSelector('[data-probe="pupils"] .fl-bot');
	const eyes = await pixels(
		await page.$('[data-probe="pupils"] .fl-bot'),
		"pupils",
	);
	let dark = 0;
	for (let y = 0; y < eyes.info.height; y++)
		for (let x = 0; x < eyes.info.width; x++) {
			const offset = (y * eyes.info.width + x) * eyes.info.channels;
			if (eyes.data[offset] > 45) continue;
			dark++;
			const px = ((x + 0.5) * 100) / eyes.info.width;
			const py = ((y + 0.5) * 100) / eyes.info.height;
			assert.ok(
				[35, 65].some(
					(cx) => ((px - cx) / 3) ** 2 + ((py - 50) / 3.36) ** 2 <= 1.08,
				),
				`Pupil escaped its eye at ${px},${py}`,
			);
		}
	assert.ok(dark > 20, "Pupils should remain visible at the gaze limit");
	const gold = await pixels(
		await page.$('[data-probe="gold"] .fl-bot'),
		"gold",
	);
	assert.ok(
		brightness(pixel(gold, 50, 39)) - brightness(pixel(gold, 50, 59)) > 60,
		"Gold must preserve the dark visor against the helmet",
	);
	const hami = await pixels(
		await page.$('[data-probe="hami"] .fl-bot'),
		"hami",
	);
	assert.deepEqual(
		pixel(hami, 13, 96),
		[252, 234, 216],
		"Cheeks must not spill outside the body",
	);
	await page.$eval('[data-probe="lava"] .fl-bot', (element) => {
		for (const animation of element.getAnimations({ subtree: true })) {
			animation.pause();
			animation.currentTime = 3000;
		}
	});
	const lava = await pixels(
		await page.$('[data-probe="lava"] .fl-bot'),
		"lava",
	);
	assert.deepEqual(
		pixel(lava, 50, 7),
		[243, 230, 243],
		"Lava must stay inside its vessel throughout the loop",
	);

	await page.goto(`${origin}/?review`, { waitUntil: "networkidle0" });
	await page.waitForFunction(() =>
		[...document.querySelectorAll("[data-review] .fl-bot")].every(
			(bot) => bot.getAttribute("data-motion") === "on",
		),
	);
	const motionKinds = [
		"orbi",
		"ember",
		"magma",
		"piyo",
		"floppy",
		"chip",
		"bit",
	];
	for (const kind of motionKinds) {
		const samples = await page.$eval(
			`[data-review="${kind}"] .fl-bot`,
			(bot) => {
				const animations = bot.getAnimations({ subtree: true });
				const parts = [
					...bot.querySelectorAll("[data-motion-part], .fl-bot-orbit-moon"),
				];
				return Array.from({ length: 9 }, (_, frame) => {
					for (const animation of animations) {
						animation.pause();
						animation.currentTime =
							(Number(animation.effect.getTiming().duration) * frame) / 10;
					}
					const bounds = bot.getBoundingClientRect();
					for (const moon of bot.querySelectorAll(".fl-bot-orbit-moon")) {
						const marker = moon.getBoundingClientRect();
						const distance = Math.hypot(
							marker.x + marker.width / 2 - bounds.x - bounds.width / 2,
							marker.y + marker.height / 2 - bounds.y - bounds.height / 2,
						);
						if (distance + marker.width / 2 > bounds.width / 2 + 1)
							throw new Error(
								"Orbi's orbit must stay inside the circular frame",
							);
					}
					return parts
						.map((part) => {
							const style = getComputedStyle(part);
							return [
								style.transform,
								style.opacity,
								style.offsetDistance,
							].join("|");
						})
						.join(";");
				});
			},
		);
		assert.ok(
			new Set(samples).size > 1,
			`${kind} needs a visible detail animation`,
		);
	}
	for (const [name, progress] of [
		["start", 0],
		["middle", 0.35],
		["late", 0.8],
	]) {
		await page.evaluate((fraction) => {
			for (const animation of document.getAnimations()) {
				animation.pause();
				animation.currentTime =
					Number(animation.effect.getTiming().duration) * fraction;
			}
		}, progress);
		await page.screenshot({
			path: resolve(output, `review-${name}.png`),
			fullPage: true,
		});
	}
	await page.goto(`${origin}/?review&sleep`, { waitUntil: "networkidle0" });
	assert.equal(
		await page.$$eval(
			".fl-bot-crew-motion, .fl-bot-orbit-plane, .fl-bot-orbit-moon",
			(parts) =>
				parts.every(
					(part) => getComputedStyle(part).animationPlayState === "paused",
				),
		),
		true,
		"Character details should rest while sleeping",
	);
	await page.goto(`${origin}/?review&paused&frame=square`, {
		waitUntil: "networkidle0",
	});
	assert.equal(
		await page.evaluate(() => document.getAnimations().length),
		0,
		"Pause must stop every detail animation",
	);
	await page.screenshot({
		path: resolve(output, "review-square.png"),
		fullPage: true,
	});
	await page.goto(
		`${origin}/?crew&pack=pirates-crew&pack=arcane-crew&pack=celebrations-crew`,
		{ waitUntil: "networkidle0" },
	);
	assert.equal(
		await page.$$eval(".crew-row figure", (figures) => figures.length),
		18,
	);
	await page.screenshot({
		path: resolve(output, "new-packs.png"),
		fullPage: true,
	});

	await page.goto(origin, { waitUntil: "networkidle0" });
	await page.waitForSelector("[data-pack-id]");
	assert.equal(
		await page.$$eval("[data-pack-id]", (elements) => elements.length),
		23,
	);
	assert.equal(
		await page.$$eval('[aria-label^="Choose "]', (elements) => elements.length),
		129,
	);
	await page.screenshot({ path: resolve(output, "packs.png"), fullPage: true });
	await page.click('[aria-label="Choose Mochi"]');
	await page.$$eval('[aria-label="Preview pose"] button', (buttons) =>
		buttons.find((button) => button.textContent === "Sleep")?.click(),
	);
	await page.waitForSelector(
		'.fl-bot-studio-stage [data-feature="closed-eyes"]',
	);
	assert.equal(
		await page.$$eval(
			'.fl-bot-studio-stage [data-part="eyes"] .sclera',
			(elements) => elements.length,
		),
		0,
	);
	assert.equal(
		await page.$eval('.fl-bot-studio-stage [data-part="eyes"]', (element) =>
			element.hasAttribute("transform"),
		),
		false,
	);
	await (await page.$(".fl-bot-studio-preview")).screenshot({
		path: resolve(output, "sleep.png"),
	});

	await page.goto(`${origin}/?owned=flow-crew`, { waitUntil: "networkidle0" });
	await page.waitForSelector('[aria-label="Choose Mochi"]');
	assert.equal(
		await page.$eval(
			'[aria-label="Choose Mochi"]',
			(element) =>
				element.disabled || element.getAttribute("aria-disabled") === "true",
		),
		false,
	);
	await page.$eval('[aria-label="Choose Mochi"]', (element) => element.click());
	assert.equal(
		await page.$eval(
			"[data-appearance]",
			(element) => JSON.parse(element.textContent).kind,
		),
		"mochi",
	);
	assert.equal(
		await page.$eval('[aria-label="Save bot"]', (button) => button.disabled),
		true,
	);
	await page.$eval('[aria-label="Save bot"]', (button) => button.click());
	assert.equal(
		await page.$eval("[data-saved-appearance]", (element) =>
			JSON.parse(element.textContent),
		),
		null,
	);
	await page.click('[aria-label="Choose Bubble"]');
	assert.equal(
		await page.$eval(
			"[data-appearance]",
			(element) => JSON.parse(element.textContent).kind,
		),
		"bubble",
	);
	assert.equal(
		await page.$eval('[aria-label="Save bot"]', (button) => button.disabled),
		false,
	);
	await page.click('[aria-label="Save bot"]');
	await page.waitForFunction(
		() =>
			JSON.parse(document.querySelector("[data-saved-appearance]").textContent)
				?.kind === "bubble",
	);
	const request = await page.$(
		'[data-pack-id="basics-crew"] .fl-bot-studio-pack-link',
	);
	assert.ok(request, "Locked packs need a host-driven detail action");
	await request.click();
	assert.match(
		await page.$eval("[data-requested-pack]", (element) => element.textContent),
		/basics-crew/,
	);
	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await page.emulateMediaFeatures([
		{ name: "prefers-reduced-motion", value: "reduce" },
	]);
	await page.screenshot({
		path: resolve(output, "mobile.png"),
		fullPage: true,
	});
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
		true,
	);
	await page.waitForFunction(
		() =>
			document
				.getAnimations()
				.filter((animation) => animation.playState === "running").length === 0,
	);
	await verifyGallery(page, { origin, output });
	await verifyMoods(page, { origin, output });
	await verifyEffects(page, { origin, output });
	await verifyPixelRenderer(page, { origin });
	await verifyClothingFits(page, { origin, output });
	await verifyEquipmentFits(page, { origin, output });
	await verifyColors(page, origin, output);
	await verifyWardrobe(page, { origin, output });
	assert.deepEqual(errors, []);
	console.log(`Bot browser checks passed. Screenshots: ${output}`);
} finally {
	await browser?.close();
	server.kill("SIGTERM");
}
