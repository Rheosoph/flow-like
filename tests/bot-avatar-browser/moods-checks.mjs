import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const moods = {
	"likes-you": ["happy", "heart", "idle"],
	cheerful: ["happy", "note", "idle"],
	love: ["love", "heart", "idle"],
	wink: ["wink", "spark", "idle"],
	surprised: ["surprised", "alert", "idle"],
	thinking: ["curious", "question", "right"],
	sad: ["sad", "rain", "idle"],
	angry: ["angry", "anger", "idle"],
	oops: ["oops", "drop", "oops"],
	sleeping: ["neutral", "zzz", "sleep"],
};
const states = {
	working: ["focused", "loading", "center"],
	idea: ["surprised", "idea", "center"],
	done: ["happy", "check", "happy"],
	error: ["oops", "error", "oops"],
	"needs-you": ["neutral", "alert", "center"],
	offline: ["neutral", "zzz", "sleep"],
};

async function readCards(page, attribute) {
	return page.$$eval(
		`[${attribute}]`,
		(cards, key) =>
			Object.fromEntries(
				cards.map((card) => {
					const bot = card.querySelector(".fl-bot");
					return [
						card.getAttribute(key),
						[
							bot.getAttribute("data-expression"),
							bot
								.querySelector("[data-emote-icon]")
								?.getAttribute("data-emote-icon"),
							bot.getAttribute("data-pose"),
						],
					];
				}),
			),
		attribute,
	);
}

async function inspectSvg(page) {
	const failures = await page.$$eval("[data-moods-preview] .fl-bot", (bots) => {
		const failures = [];
		const ids = new Set();
		for (const bot of bots) {
			const name = `${bot.dataset.kind}/${bot.dataset.agentState}/${bot.dataset.mood ?? ""}`;
			const svg = bot.querySelector("svg");
			for (const element of svg.querySelectorAll("*")) {
				if (element.id) {
					if (ids.has(element.id)) failures.push(`${name}: duplicate SVG ID`);
					ids.add(element.id);
				}
				for (const attribute of element.attributes) {
					for (const reference of attribute.value.matchAll(
						/url\(#([^)]*)\)/g,
					)) {
						const target = document.getElementById(reference[1]);
						if (!target || !svg.contains(target))
							failures.push(`${name}: broken local SVG reference`);
					}
				}
				if (element.tagName === "path") {
					try {
						if (!Number.isFinite(element.getTotalLength()))
							failures.push(`${name}: invalid path`);
					} catch {
						failures.push(`${name}: unreadable path`);
					}
				}
			}
			const bubble = bot.querySelector("[data-emote]");
			if (!bubble) {
				failures.push(`${name}: missing thought bubble`);
				continue;
			}
			if (
				bubble.closest("[data-visible]")?.getAttribute("data-visible") !==
				"true"
			)
				failures.push(`${name}: hidden paused bubble`);
			const box = bubble.getBoundingClientRect();
			const frame = svg.getBoundingClientRect();
			if (
				box.width <= 0 ||
				box.height <= 0 ||
				box.left < frame.left - 0.5 ||
				box.top < frame.top - 0.5 ||
				box.right > frame.right + 0.5 ||
				box.bottom > frame.bottom + 0.5
			)
				failures.push(`${name}: thought bubble exceeds viewport`);
		}
		return failures;
	});
	assert.deepEqual(failures, [], "Mood SVGs and bubble bounds must be valid");
}

export async function verifyMoods(page, { origin, output }) {
	const screenshots = resolve(output, "moods");
	await mkdir(screenshots, { recursive: true });
	const previousViewport = page.viewport();
	const errors = [];
	const onError = (error) => errors.push(error.message);
	page.on("pageerror", onError);
	try {
		await page.setViewport({ width: 1200, height: 1200, deviceScaleFactor: 1 });
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		const open = async (query = "") => {
			await page.goto(`${origin}/?moods${query}`, {
				waitUntil: "networkidle0",
			});
			await page.waitForSelector("[data-mood-card] [data-emote-icon]");
		};
		await open("&paused");
		assert.deepEqual(await readCards(page, "data-mood-card"), moods);
		assert.deepEqual(await readCards(page, "data-state-card"), states);
		await inspectSvg(page);
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"Paused previews must stop every animation",
		);
		const retro = await page.$$eval("[data-retro-card] .fl-bot", (bots) =>
			bots.map((bot) => bot.dataset.effect),
		);
		assert.deepEqual(retro, [
			"lcd",
			"lcd",
			"lcd",
			"pixel",
			"pixel",
			"pixel",
			"pixel",
			"pixel",
		]);
		const compact = await page.$$eval(".moods-compact .fl-bot", (bots) =>
			bots.map((bot) => ({
				width: bot.getBoundingClientRect().width,
				height: bot.getBoundingClientRect().height,
				emote: bot
					.querySelector("[data-emote-icon]")
					?.getAttribute("data-emote-icon"),
			})),
		);
		assert.equal(compact.length, 6);
		for (const bot of compact) {
			assert.equal(bot.width, 32);
			assert.equal(bot.height, 32);
			assert.ok(bot.emote);
		}
		await page.screenshot({
			path: resolve(screenshots, "reference-sheet.png"),
			fullPage: true,
		});

		await page.select('[aria-label="Preview agent state"]', "offline");
		const readProbe = () =>
			page.$eval("[data-state-probe] .fl-bot", (bot) => ({
				state: bot.dataset.agentState,
				expression: bot.dataset.expression,
				pose: bot.dataset.pose,
				effect: bot.dataset.effect,
				emote: bot
					.querySelector("[data-emote-icon]")
					?.getAttribute("data-emote-icon"),
			}));
		assert.deepEqual(await readProbe(), {
			state: "offline",
			expression: "neutral",
			pose: "sleep",
			effect: "offline",
			emote: "zzz",
		});
		await page.select('[aria-label="Preview agent state"]', "idle");
		assert.deepEqual(await readProbe(), {
			state: "idle",
			expression: "love",
			pose: "idle",
			effect: "pixel",
			emote: "heart",
		});

		await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
		assert.equal(
			await page.evaluate(
				() => document.documentElement.scrollWidth > innerWidth,
			),
			false,
			"The sheet must fit a phone viewport",
		);
		await inspectSvg(page);
		await page.screenshot({
			path: resolve(screenshots, "phone.png"),
			fullPage: true,
		});

		await page.setViewport({ width: 1200, height: 1200, deviceScaleFactor: 1 });
		await open();
		await page.$eval('[data-state-card="working"]', (card) =>
			card.scrollIntoView({ block: "center", behavior: "instant" }),
		);
		await page.waitForFunction(
			() =>
				document
					.querySelector('[data-state-card="working"] .fl-bot')
					?.getAttribute("data-motion") === "on",
		);
		const loading = await page.$$eval(
			'[data-state-card="working"] [data-loading-dot]',
			(dots) =>
				dots.map((dot) => {
					const style = getComputedStyle(dot);
					return {
						name: style.animationName,
						delay: style.animationDelay,
						animations: dot.getAnimations().length,
					};
				}),
		);
		assert.deepEqual(loading, [
			{ name: "fl-bot-loading-dot", delay: "0s", animations: 1 },
			{ name: "fl-bot-loading-dot", delay: "0.16s", animations: 1 },
			{ name: "fl-bot-loading-dot", delay: "0.32s", animations: 1 },
		]);
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "reduce" },
		]);
		await page.waitForFunction(() =>
			[...document.querySelectorAll("[data-moods-preview] .fl-bot")].every(
				(bot) => bot.getAttribute("data-motion") === "off",
			),
		);
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"Reduced motion must stop every animation",
		);
		assert.deepEqual(errors, [], "Mood previews must not raise browser errors");
		console.log(
			`Mood checks passed: 10 feelings, 6 agent states, LCD/pixel previews, 32px icons, state restoration, loading motion. Screenshots: ${screenshots}`,
		);
	} finally {
		page.off("pageerror", onError);
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		if (previousViewport) await page.setViewport(previousViewport);
	}
}
