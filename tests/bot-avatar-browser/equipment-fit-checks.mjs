import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const REVIEW_KINDS = [
	"fern",
	"kero",
	"bonsai",
	"bambu",
	"ruler",
	"pipette",
	"mallet",
	"orbi",
	"buoy",
	"clef",
	"shelly",
	"scope",
	"mochi",
];

export async function inspectHeadwearClearance(page) {
	return page.evaluate(() => {
		const samples = document.querySelectorAll("[data-equipment-fit-sample]");
		const failures = [];
		let count = 0;
		for (const sample of samples) {
			if (sample.dataset.headId === "none") continue;
			count++;
			const head = sample.querySelector('[data-part="head"]');
			const eyes = sample.querySelector('[data-part="eyes"]');
			const kind = sample.querySelector(".fl-bot")?.dataset.kind;
			const identity = {
				kind,
				item: sample.dataset.headId,
				eyes: sample.dataset.eyesId,
				size: Number(sample.dataset.size),
				frame: sample.dataset.frame,
			};
			if (!head?.childElementCount || !eyes?.getCTM()) {
				failures.push({
					...identity,
					reason: "Missing headwear or eye geometry",
				});
				continue;
			}
			const radius = Number(sample.dataset.eyeRadius) * 0.65;
			const probes = ["left", "right"].flatMap((side) => {
				const [x, y] = sample.dataset[side === "left" ? "eyeLeft" : "eyeRight"]
					.split(",")
					.map(Number);
				return [
					[0, 0],
					[-radius, 0],
					[radius, 0],
					[0, -radius],
					[0, radius],
				].map(([dx, dy]) => ({
					side,
					point: new DOMPoint(x + dx, y + dy).matrixTransform(eyes.getCTM()),
				}));
			});
			const hits = new Set();
			for (const element of head.querySelectorAll("*")) {
				if (
					!(element instanceof SVGGeometryElement) ||
					element.closest("defs, clipPath, mask")
				)
					continue;
				const style = getComputedStyle(element);
				if (
					style.fill === "none" ||
					style.fill === "transparent" ||
					Number(style.fillOpacity) === 0 ||
					style.visibility === "hidden" ||
					style.display === "none"
				)
					continue;
				let opacity = Number(style.fillOpacity);
				if (style.fill.startsWith("rgba("))
					opacity *= Number(style.fill.slice(5, -1).split(",").at(-1));
				for (
					let parent = element;
					parent && parent !== head.parentElement;
					parent = parent.parentElement
				)
					opacity *= Number(getComputedStyle(parent).opacity);
				// Transparent visor lenses may cover the eyes without obscuring them.
				if (opacity < 0.3) continue;
				const matrix = element.getCTM();
				if (
					!matrix ||
					![matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f].every(
						Number.isFinite,
					)
				) {
					hits.add("Invalid transform");
					continue;
				}
				for (const { side, point } of probes)
					if (element.isPointInFill(point.matrixTransform(matrix.inverse())))
						hits.add(side);
			}
			if (hits.size)
				failures.push({
					...identity,
					reason: "Headwear fill covers an inner eye probe",
					covered: [...hits],
				});
		}
		return { count, failures };
	});
}

export async function auditHeadwearFit(page, { origin, output, kinds }) {
	await page.goto(`${origin}/?equipment-fit&audit`, {
		waitUntil: "networkidle0",
	});
	const crew =
		kinds ??
		(await page.$$eval(
			'[aria-label="Equipment fit character"] option',
			(options) => options.map((option) => option.value),
		));
	const report = {
		characters: crew.length,
		pairs: 0,
		excluded: 0,
		failures: [],
	};
	for (const [index, kind] of crew.entries()) {
		await page.select('[aria-label="Equipment fit character"]', kind);
		await page.waitForFunction(
			(value) =>
				document.querySelector("[data-equipment-fit]")?.dataset
					.equipmentFitKind === value,
			{},
			kind,
		);
		const counts = await page.$eval("[data-equipment-fit]", (element) => ({
			supported: Number(element.dataset.supportedHeads),
			total: Number(element.dataset.totalHeads),
		}));
		const result = await inspectHeadwearClearance(page);
		assert.equal(
			result.count,
			counts.supported,
			`${kind}: every compatible head item must render`,
		);
		report.pairs += result.count;
		report.excluded += counts.total - counts.supported;
		report.failures.push(...result.failures);
		if ((index + 1) % 16 === 0 || index + 1 === crew.length)
			console.log(
				`Headwear fit audit: ${index + 1}/${crew.length} characters, ${report.pairs} pairs, ${report.failures.length} occlusions`,
			);
	}
	if (output) {
		await mkdir(output, { recursive: true });
		await writeFile(
			resolve(output, "equipment-fit-audit.json"),
			`${JSON.stringify(report, null, 2)}\n`,
		);
	}
	return report;
}

export async function verifyEquipmentFits(page, { origin, output }) {
	const screenshots = resolve(output, "equipment-fit");
	await mkdir(screenshots, { recursive: true });
	const previousViewport = page.viewport();
	const errors = [];
	const onError = (error) => errors.push(error.message);
	page.on("pageerror", onError);
	try {
		await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 });
		const report = await auditHeadwearFit(page, {
			origin,
			output: screenshots,
		});
		assert.deepEqual(
			report.failures,
			[],
			"Supported headwear must leave the inner eye area clear",
		);
		let samples = 0;
		for (const kind of REVIEW_KINDS) {
			await page.goto(`${origin}/?equipment-fit&kind=${kind}`, {
				waitUntil: "networkidle0",
			});
			const result = await inspectHeadwearClearance(page);
			assert.deepEqual(
				result.failures,
				[],
				`${kind}: combined headwear and eyewear must leave eyes clear`,
			);
			samples += result.count;
			for (const frame of ["circle", "square"])
				await (await page.$(`[data-fit-frame="${frame}"]`)).screenshot({
					path: resolve(screenshots, `${kind}-${frame}.png`),
				});
		}
		assert.deepEqual(
			errors,
			[],
			"Equipment previews must not raise browser errors",
		);
		console.log(
			`Equipment fit checks passed: ${report.pairs} supported head/character pairs, ${report.excluded} exclusions, ${samples} combined size/frame samples. Screenshots: ${screenshots}`,
		);
	} finally {
		page.off("pageerror", onError);
		if (previousViewport) await page.setViewport(previousViewport);
	}
}
