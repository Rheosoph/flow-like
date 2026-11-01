import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const detailSelector =
	"[data-detail-motion], .fl-bot-crew-motion, .fl-bot-orbit-plane, .fl-bot-orbit-moon";
const restSelector = `${detailSelector}, .fl-bot-character[data-character-motion="float"], .fl-bot-body[data-body-motion="rotate"]`;

async function inspectGallery(page, expectedKinds, expression) {
	const inspection = await page.$$eval("[data-gallery] .fl-bot", (bots) => {
		const failures = [];
		const ids = new Set();
		const geometry = new Set([
			"d",
			"points",
			"transform",
			"x",
			"y",
			"x1",
			"y1",
			"x2",
			"y2",
			"cx",
			"cy",
			"r",
			"rx",
			"ry",
			"width",
			"height",
			"stroke-width",
		]);
		for (const bot of bots) {
			const kind = bot.getAttribute("data-kind");
			if (!bot.querySelector('[data-part="eyes"]')?.childElementCount)
				failures.push(`${kind}: missing eyes`);
			for (const element of bot.querySelectorAll("svg *")) {
				if (element.id) {
					if (ids.has(element.id)) failures.push(`${kind}: duplicate SVG ID`);
					ids.add(element.id);
				}
				for (const attribute of element.attributes) {
					if (
						geometry.has(attribute.name) &&
						/NaN|undefined|Infinity/.test(attribute.value)
					)
						failures.push(`${kind}: invalid ${attribute.name}`);
					for (const reference of attribute.value.matchAll(
						/url\(#([^)]*)\)/g,
					)) {
						if (!document.getElementById(reference[1]))
							failures.push(`${kind}: missing SVG reference ${reference[1]}`);
					}
				}
				if (element.tagName === "path") {
					try {
						if (!Number.isFinite(element.getTotalLength()))
							failures.push(`${kind}: invalid path length`);
					} catch {
						failures.push(`${kind}: unreadable path`);
					}
				}
			}
		}
		return {
			failures,
			kinds: bots.map((bot) => bot.getAttribute("data-kind")),
			expressions: bots.map((bot) => bot.getAttribute("data-expression")),
		};
	});
	assert.deepEqual(inspection.failures, [], "Gallery SVGs must remain valid");
	assert.deepEqual(
		inspection.kinds,
		expectedKinds,
		"Every catalog bot must render",
	);
	assert.ok(inspection.expressions.every((value) => value === expression));
}

async function showPack(page, id) {
	await page.$eval(`[data-gallery-pack="${id}"]`, (section) =>
		section.scrollIntoView({ block: "center", behavior: "instant" }),
	);
	await page.waitForFunction(
		(packId) => {
			const bots = document.querySelectorAll(
				`[data-gallery-pack="${packId}"] .fl-bot`,
			);
			return (
				bots.length > 0 &&
				[...bots].every((bot) => bot.getAttribute("data-motion") === "on")
			);
		},
		{},
		id,
	);
}

/** Exercise the full catalog without baking its current size into the browser test. */
export async function verifyGallery(page, { origin, output }) {
	const screenshots = resolve(output, "gallery");
	await mkdir(screenshots, { recursive: true });
	const previousViewport = page.viewport();
	const errors = [];
	const onError = (error) => errors.push(error.message);
	page.on("pageerror", onError);
	try {
		await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		const open = async (query = "") => {
			await page.goto(`${origin}/?gallery${query}`, {
				waitUntil: "networkidle0",
			});
			await page.waitForSelector("[data-gallery-pack] .fl-bot");
		};
		await open("&paused");
		const packs = await page.$$eval("[data-gallery-pack]", (sections) =>
			sections.map((section) => ({
				id: section.getAttribute("data-gallery-pack"),
				kinds: [...section.querySelectorAll(".fl-bot")].map((bot) =>
					bot.getAttribute("data-kind"),
				),
			})),
		);
		assert.ok(packs.length > 0, "The gallery needs catalog packs");
		const kinds = packs.flatMap((pack) => pack.kinds);
		assert.equal(
			new Set(kinds).size,
			kinds.length,
			"Default packs must not repeat bots",
		);
		await inspectGallery(page, kinds, "neutral");
		for (const mode of ["neutral", "outfit"]) {
			if (mode === "outfit") await open("&paused&outfit");
			await inspectGallery(page, kinds, "neutral");
			for (const [index, pack] of packs.entries()) {
				const section = await page.$(`[data-gallery-pack="${pack.id}"]`);
				assert.ok(section);
				await section.screenshot({
					path: resolve(
						screenshots,
						`${String(index + 1).padStart(2, "0")}-${pack.id}-${mode}.png`,
					),
				});
			}
		}
		await open("&paused");
		const uniforms = await page.$$eval(
			'[aria-label="Uniform"] option',
			(options) =>
				options.map((option) => option.value).filter((id) => id !== "classic"),
		);
		for (const uniform of uniforms) {
			await page.select('[aria-label="Uniform"]', uniform);
			await inspectGallery(page, kinds, "neutral");
			for (const pack of [
				"office-crew",
				"service-crew",
				"workplace-crew",
				"healthcare-crew",
			]) {
				const section = await page.$(`[data-gallery-pack="${pack}"]`);
				assert.ok(section);
				await section.screenshot({
					path: resolve(screenshots, `${pack}-uniform-${uniform}.png`),
				});
			}
		}
		await open("&paused");
		const expressionSelect = ".gallery-controls label:nth-child(2) select";
		for (const expression of [
			"neutral",
			"happy",
			"concerned",
			"surprised",
			"curious",
			"sad",
			"wink",
			"love",
			"angry",
			"focused",
			"oops",
		]) {
			await page.select(expressionSelect, expression);
			await page.waitForFunction(
				(value) =>
					[...document.querySelectorAll("[data-gallery] .fl-bot")].every(
						(bot) => bot.getAttribute("data-expression") === value,
					),
				{},
				expression,
			);
			await inspectGallery(page, kinds, expression);
		}

		await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
		await open("&paused&outfit&size=32");
		await inspectGallery(page, kinds, "neutral");
		const compact = await page.evaluate(() => ({
			overflow: document.documentElement.scrollWidth > innerWidth,
			wrongSize: [...document.querySelectorAll("[data-gallery] .fl-bot")].some(
				(bot) => {
					const box = bot.getBoundingClientRect();
					return (
						Math.abs(box.width - 32) > 0.1 || Math.abs(box.height - 32) > 0.1
					);
				},
			),
			handItems: document.querySelectorAll(
				'[data-gallery] [data-part="hand"] > *',
			).length,
		}));
		assert.deepEqual(compact, {
			overflow: false,
			wrongSize: false,
			handItems: 0,
		});
		await page.screenshot({
			path: resolve(screenshots, "compact-32.png"),
			fullPage: true,
		});

		await page.setViewport({ width: 1200, height: 1000, deviceScaleFactor: 1 });
		await open();
		let checkedDetails = 0;
		for (const pack of packs) {
			await showPack(page, pack.id);
			const samples = await page.$eval(
				`[data-gallery-pack="${pack.id}"]`,
				(section, selector) =>
					[...section.querySelectorAll(selector)].map((part, index) => {
						const animations = part.getAnimations();
						const states = Array.from({ length: 11 }, (_, step) => {
							for (const animation of animations) {
								const timing = animation.effect.getTiming();
								animation.pause();
								animation.currentTime =
									timing.delay + Number(timing.duration) * (1 + step / 11);
							}
							const style = getComputedStyle(part);
							return [
								style.transform,
								style.opacity,
								style.offsetDistance,
							].join("|");
						});
						return {
							name: `${part.closest(".fl-bot").getAttribute("data-kind")}/${part.getAttribute("data-detail-motion") ?? part.getAttribute("data-motion-part") ?? part.classList.value}/${index}`,
							animated: animations.length > 0,
							changing: new Set(states).size > 1,
						};
					}),
				detailSelector,
			);
			for (const sample of samples) {
				assert.ok(sample.animated, `${sample.name} needs an active animation`);
				assert.ok(
					sample.changing,
					`${sample.name} must visibly change over a cycle`,
				);
				checkedDetails++;
			}
			if (
				["ocean-crew", "music-crew", "cosmos-crew", "fantasy-crew"].includes(
					pack.id,
				)
			) {
				const section = await page.$(`[data-gallery-pack="${pack.id}"]`);
				assert.ok(section);
				for (const milliseconds of [0, 1200, 2400]) {
					await section.evaluate((element, time) => {
						for (const animation of element.getAnimations({ subtree: true })) {
							animation.pause();
							animation.currentTime = time;
						}
					}, milliseconds);
					await section.screenshot({
						path: resolve(
							screenshots,
							`${pack.id}-motion-${milliseconds}ms.png`,
						),
					});
				}
			}
		}
		assert.ok(
			checkedDetails > 0,
			"Detail animation coverage must not be empty",
		);

		await open("&outfit");
		const floating = await page.$$eval(
			'[data-character-motion="float"]',
			(rigs) =>
				rigs.map((rig) => ({
					kind: rig.closest(".fl-bot").getAttribute("data-kind"),
					pack: rig
						.closest("[data-gallery-pack]")
						.getAttribute("data-gallery-pack"),
				})),
		);
		assert.ok(
			floating.length > 0,
			"The gallery needs floating rigs to exercise",
		);
		for (const floatingBot of floating) {
			await showPack(page, floatingBot.pack);
			const deltas = await page.$eval(
				`.fl-bot[data-kind="${floatingBot.kind}"]`,
				(bot) => {
					const rig = bot.querySelector('[data-character-motion="float"]');
					for (const animation of bot.getAnimations({ subtree: true })) {
						animation.pause();
						animation.currentTime = 0;
					}
					const animation = rig.getAnimations()[0];
					if (!animation) return null;
					const parts = ["body", "eyes", "face", "head", "neck", "hand"].map(
						(name) => bot.querySelector(`[data-part="${name}"]`),
					);
					const position = (part) => {
						const matrix = part.getCTM();
						return [matrix.e, matrix.f];
					};
					const before = parts.map(position);
					animation.currentTime =
						Number(animation.effect.getTiming().duration) / 2;
					return parts.map((part, index) =>
						position(part).map((value, axis) => value - before[index][axis]),
					);
				},
			);
			assert.ok(deltas, `${floatingBot.kind} needs a float animation`);
			assert.ok(
				Math.abs(deltas[0][1]) > 0.1,
				`${floatingBot.kind} must move vertically`,
			);
			for (const delta of deltas) {
				assert.ok(delta.every(Number.isFinite));
				assert.ok(
					delta.every(
						(value, axis) => Math.abs(value - deltas[0][axis]) < 0.01,
					),
					`${floatingBot.kind}: face and outfit must travel with the body`,
				);
			}
		}

		await open("&sleep");
		for (const pack of packs) {
			await showPack(page, pack.id);
			const awake = await page.$eval(
				`[data-gallery-pack="${pack.id}"]`,
				(section, selector) =>
					[...section.querySelectorAll(selector)]
						.filter((part) =>
							getComputedStyle(part)
								.animationPlayState.split(",")
								.some((state) => state.trim() !== "paused"),
						)
						.map((part) => part.closest(".fl-bot").getAttribute("data-kind")),
				restSelector,
			);
			assert.deepEqual(awake, [], `${pack.id}: sleeping details must rest`);
		}
		await open("&paused");
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"animated=false must stop every animation",
		);
		await open();
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "reduce" },
		]);
		await page.waitForFunction(() =>
			[...document.querySelectorAll("[data-gallery] .fl-bot")].every(
				(bot) => bot.getAttribute("data-motion") === "off",
			),
		);
		assert.equal(
			await page.evaluate(() => document.getAnimations().length),
			0,
			"Reduced motion must stop every animation",
		);
		assert.deepEqual(errors, [], "The gallery must not raise browser errors");
		console.log(
			`Gallery checks passed: ${kinds.length} bots, ${packs.length} packs, ${checkedDetails} animated parts, ${floating.length} floating rigs. Screenshots: ${screenshots}`,
		);
	} finally {
		page.off("pageerror", onError);
		await page.emulateMediaFeatures([
			{ name: "prefers-reduced-motion", value: "no-preference" },
		]);
		if (previousViewport) await page.setViewport(previousViewport);
	}
}
