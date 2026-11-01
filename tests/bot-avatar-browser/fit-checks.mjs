import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";
import sharp from "sharp";

const cutouts = [
	{ kind: "wrench", point: [50, 94], background: true, name: "handle hole" },
	{
		kind: "puzzlo",
		point: [50, 96],
		background: true,
		name: "lower puzzle socket",
	},
	{ kind: "buoy", point: [50, 54], background: true, name: "ring opening" },
	{
		kind: "molar",
		point: [50, 94],
		background: true,
		name: "gap between roots",
	},
	{ kind: "sol", point: [50, 96], background: false, name: "lower sun ray" },
	{
		kind: "magma",
		point: [50, 86],
		background: false,
		name: "lower lava drip",
	},
	{ kind: "bonsai", point: [50, 83], background: false, name: "exposed trunk" },
];

function pixel(image, x, y) {
	const column = Math.max(0, Math.min(image.info.width - 1, Math.floor(x)));
	const row = Math.max(0, Math.min(image.info.height - 1, Math.floor(y)));
	const offset = (row * image.info.width + column) * image.info.channels;
	return [...image.data.subarray(offset, offset + 3)];
}

function distance(a, b) {
	return Math.max(...a.map((channel, index) => Math.abs(channel - b[index])));
}

async function inspectNeckGeometry(page, kind) {
	const result = await page.$$eval(
		'[data-wardrobe] [data-equipment-slot="neck"]',
		(items) => {
			const failures = [];
			for (const item of items) {
				const id = item.dataset.equipmentId;
				const svg = item.querySelector("svg");
				const clothing = svg?.querySelector('[data-part="neck"]');
				if (!clothing?.childElementCount) {
					failures.push(`${id}: missing clothing`);
					continue;
				}
				for (const element of clothing.querySelectorAll("*")) {
					for (const attribute of element.attributes) {
						if (/NaN|undefined|Infinity/.test(attribute.value))
							failures.push(`${id}: invalid ${attribute.name}`);
						for (const ref of attribute.value.matchAll(/url\(#([^)]*)\)/g))
							if (!svg.contains(document.getElementById(ref[1])))
								failures.push(`${id}: missing local ${ref[1]}`);
					}
					if (element instanceof SVGGeometryElement) {
						const length = element.getTotalLength();
						if (!Number.isFinite(length)) failures.push(`${id}: invalid path`);
					}
					if (element instanceof SVGGraphicsElement) {
						const matrix = element.getCTM();
						if (
							!matrix ||
							![
								matrix.a,
								matrix.b,
								matrix.c,
								matrix.d,
								matrix.e,
								matrix.f,
							].every(Number.isFinite)
						)
							failures.push(`${id}: invalid garment transform`);
					}
				}
			}
			return { count: items.length, failures };
		},
	);
	assert.ok(result.count >= 143, `${kind}: every neck item must be checked`);
	assert.deepEqual(
		result.failures,
		[],
		`${kind}: clothing geometry must remain valid`,
	);
	return result.count;
}

export async function verifyClothingFits(page, { origin, output }) {
	const screenshots = resolve(output, "clothing-fit");
	await mkdir(screenshots, { recursive: true });
	const previousViewport = page.viewport();
	const errors = [];
	const onError = (error) => errors.push(error.message);
	page.on("pageerror", onError);
	try {
		await page.setViewport({ width: 1440, height: 1100, deviceScaleFactor: 1 });
		for (const effect of ["none", "pixel"]) {
			for (const { kind, point, background, name } of cutouts) {
				await page.goto(
					`${origin}/?fit&kind=${kind}&effect=${effect}&frame=square`,
					{ waitUntil: "networkidle0" },
				);
				await page.waitForSelector(`[data-fit-character="${kind}"]`);
				if (effect === "pixel")
					await page.waitForFunction(() =>
						[
							...document.querySelectorAll("[data-fit-character] .fl-bot"),
						].every((bot) =>
							bot.querySelector('.fl-bot-pixel[data-ready="true"]'),
						),
					);
				const row = await page.$(`[data-fit-character="${kind}"]`);
				assert.ok(row);
				const positions = await row.evaluate((element) => {
					const row = element.getBoundingClientRect();
					return [...element.querySelectorAll("[data-fit-equipment]")].map(
						(card) => {
							const box = card.querySelector(".fl-bot").getBoundingClientRect();
							return {
								id: card.dataset.fitEquipment,
								x: box.x - row.x,
								y: box.y - row.y,
								width: box.width,
								height: box.height,
							};
						},
					);
				});
				assert.equal(
					positions.length,
					9,
					"The bare character and eight garment families must be compared",
				);
				const screenshot = await row.screenshot({
					path: resolve(screenshots, `${kind}-${effect}.png`),
				});
				const image = await sharp(Buffer.from(screenshot))
					.removeAlpha()
					.raw()
					.toBuffer({ resolveWithObject: true });
				const sample = (position, [x, y]) =>
					pixel(
						image,
						position.x + (position.width * x) / 100,
						position.y + (position.height * y) / 100,
					);
				const bare = positions.find((position) => position.id === "none");
				assert.ok(bare);
				const expected = sample(bare, point);
				const canvas = sample(bare, [2, 2]);
				if (background)
					assert.ok(
						distance(expected, canvas) <= 4,
						`${kind}: the bare ${name} must be clear`,
					);
				else
					assert.ok(
						distance(expected, canvas) > 40,
						`${kind}: the probe must hit the ${name}`,
					);
				for (const position of positions) {
					assert.ok(
						distance(sample(position, point), expected) <= 4,
						`${kind}/${position.id}/${effect}: clothing must preserve the ${name}`,
					);
				}
			}
		}
		let garments = 0;
		for (const kind of ["mallet", "clef", "pipette"]) {
			await page.goto(`${origin}/?wardrobe&kind=${kind}`, {
				waitUntil: "networkidle0",
			});
			garments = await inspectNeckGeometry(page, kind);
		}
		assert.deepEqual(
			errors,
			[],
			"Clothing previews must not raise browser errors",
		);
		console.log(
			`Clothing fit checks passed: seven cutout/anatomy controls, eight garment families, vector and pixel looks; all ${garments} neck items on Mallet, Clef, and Pipette. Screenshots: ${screenshots}`,
		);
	} finally {
		page.off("pageerror", onError);
		if (previousViewport) await page.setViewport(previousViewport);
	}
}
