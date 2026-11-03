import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const rendererPath = `/@fs${fileURLToPath(new URL("../../packages/ui/components/bots/bot-pixel.tsx", import.meta.url))}`;

export async function verifyPixelRenderer(page, { origin }) {
	await page.goto(`${origin}/?effects&paused`, { waitUntil: "networkidle0" });
	await page.select('[aria-label="Crew effect"]', "pixel");
	await page.waitForSelector(
		'[data-effect-crew] .fl-bot-pixel[data-ready="true"]',
	);
	const result = await page.evaluate(async (modulePath) => {
		const { drawBotPixels } = await import(modulePath);
		const sample = document.querySelector("[data-effect-crew] .fl-bot");
		const source = sample.querySelector("svg");
		const canvas = sample.querySelector("canvas");
		const pixels = () =>
			Array.from(canvas.getContext("2d").getImageData(0, 0, 24, 24).data).join(
				",",
			);
		const draw = () => drawBotPixels(source, canvas, "#292536");
		draw();
		const original = pixels();
		sample.style.setProperty("--bot-gaze-x", "-5px");
		sample.style.setProperty("--bot-face-x", "-2px");
		draw();
		const gazeChanged = pixels() !== original;
		sample.style.setProperty("--bot-gaze-x", "0px");
		sample.style.setProperty("--bot-face-x", "0px");
		draw();
		const beforeBlink = pixels();
		const eyes = source.querySelector('[data-part="eyes"]');
		eyes.style.transform = "scaleY(.1)";
		eyes.style.transformOrigin = "50px 60px";
		draw();
		const blinkChanged = pixels() !== beforeBlink;
		eyes.style.removeProperty("transform");
		eyes.style.removeProperty("transform-origin");
		draw();
		const beforeGear = pixels();
		const hand = source.querySelector('[data-part="hand"]');
		hand.style.display = "none";
		draw();
		const gearChanged = pixels() !== beforeGear;
		hand.style.removeProperty("display");
		draw();
		const beforeBubble = pixels();
		source
			.querySelector('[data-part="emote"]')
			.setAttribute("data-visible", "false");
		draw();
		const bubbleChanged = pixels() !== beforeBubble;
		source
			.querySelector('[data-part="emote"]')
			.setAttribute("data-visible", "true");

		const fixture = document.createElement("div");
		fixture.style.cssText =
			"position:fixed;left:0;top:0;width:100px;height:100px";
		fixture.innerHTML = `<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100"><defs><pattern id="pixel-test-pattern" width="12" height="12" patternUnits="userSpaceOnUse" patternTransform="rotate(35)"><rect width="12" height="12" fill="#ffffff"/><path d="M0 0V12" stroke="#ff0000" stroke-width="6" transform="translate(3 0)"/></pattern><linearGradient id="pixel-test-fade"><stop stop-color="white" stop-opacity="1"/><stop offset="1" stop-color="white" stop-opacity="0"/></linearGradient><mask id="pixel-test-mask"><rect width="100" height="100" fill="url(#pixel-test-fade)"/></mask><filter id="pixel-test-variant"><feColorMatrix type="saturate" values="0"/><feComponentTransfer><feFuncR type="table" tableValues="0 1"/><feFuncG type="table" tableValues="0 .5"/><feFuncB type="table" tableValues="0 0"/></feComponentTransfer></filter></defs><rect data-test-base width="100" height="100" fill="url(#pixel-test-pattern)"/></svg>`;
		document.body.append(fixture);
		const fixtureSvg = fixture.querySelector("svg");
		const fixtureCanvas = document.createElement("canvas");
		fixtureCanvas.width = fixtureCanvas.height = 24;
		const observer = new MutationObserver(() => {});
		observer.observe(fixtureSvg, {
			subtree: true,
			attributes: true,
			childList: true,
		});
		drawBotPixels(fixtureSvg, fixtureCanvas, "#222222");
		const drawMutations = observer.takeRecords().length;
		const read = (x, y) =>
			Array.from(fixtureCanvas.getContext("2d").getImageData(x, y, 1, 1).data);
		const patternPixels = fixtureCanvas
			.getContext("2d")
			.getImageData(0, 0, 24, 24).data;
		let red = 0;
		for (let i = 0; i < patternPixels.length; i += 4)
			if (patternPixels[i] > 200 && patternPixels[i + 1] < 50) red++;
		const base = fixtureSvg.querySelector("[data-test-base]");
		base.setAttribute("fill", "#0000ff");
		const overlay = base.cloneNode();
		overlay.setAttribute("fill", "#ff0000");
		overlay.setAttribute("mask", "url(#pixel-test-mask)");
		fixtureSvg.append(overlay);
		drawBotPixels(fixtureSvg, fixtureCanvas, "#222222");
		const maskLeft = read(3, 12);
		const maskRight = read(20, 12);
		overlay.remove();
		base.setAttribute("fill", "#ffffff");
		base.setAttribute("filter", "url(#pixel-test-variant)");
		drawBotPixels(fixtureSvg, fixtureCanvas, "#222222");
		const variant = read(12, 12);
		observer.disconnect();
		fixture.remove();

		const def = source.querySelector("defs");
		const pattern = document.createElementNS(
			"http://www.w3.org/2000/svg",
			"pattern",
		);
		pattern.setAttribute("id", "pixel-paused-pattern");
		pattern.setAttribute("width", "12");
		pattern.setAttribute("height", "12");
		pattern.setAttribute("patternUnits", "userSpaceOnUse");
		pattern.setAttribute("patternTransform", "rotate(35)");
		pattern.innerHTML =
			'<rect width="12" height="12" fill="#aa99dd"/><path d="M0 0V12" stroke="#ffffff" stroke-width="4" transform="translate(3 0)"/>';
		def.append(pattern);
		source
			.querySelector("[data-pixel-outline]")
			.setAttribute("fill", "url(#pixel-paused-pattern)");
		await new Promise((resolve) => setTimeout(resolve, 80));
		let repaints = 0;
		const stability = new MutationObserver((records) => {
			repaints += records.filter(
				(record) => record.attributeName === "data-pixel-ready",
			).length;
		});
		stability.observe(source, { attributes: true });
		await new Promise((resolve) => setTimeout(resolve, 250));
		stability.disconnect();
		const durations = [];
		for (let i = 0; i < 12; i++) {
			const start = performance.now();
			draw();
			durations.push(performance.now() - start);
		}
		return {
			gazeChanged,
			blinkChanged,
			gearChanged,
			bubbleChanged,
			drawMutations,
			red,
			maskLeft,
			maskRight,
			variant,
			repaints,
			averageDrawMs: durations.reduce((a, b) => a + b, 0) / durations.length,
		};
	}, rendererPath);
	for (const feature of [
		"gazeChanged",
		"blinkChanged",
		"gearChanged",
		"bubbleChanged",
	])
		assert.equal(
			result[feature],
			true,
			`${feature}: pixel art must follow the live rig`,
		);
	assert.equal(
		result.drawMutations,
		0,
		"Drawing must not mutate SVG transforms",
	);
	assert.ok(
		result.red > 20 && result.red < 550,
		"Rotated, translated stroke patterns must retain both colors",
	);
	assert.deepEqual(result.maskLeft, [255, 0, 0, 255]);
	assert.deepEqual(result.maskRight, [0, 0, 255, 255]);
	assert.deepEqual(result.variant, [255, 136, 0, 255]);
	assert.equal(
		result.repaints,
		0,
		"Paused pattern previews must not continuously repaint",
	);
	await page.goto(`${origin}/?gallery&paused&outfit`, {
		waitUntil: "networkidle0",
	});
	const crew = await page.evaluate(async (modulePath) => {
		const { drawBotPixels } = await import(modulePath);
		const failures = [];
		const durations = [];
		const bots = [...document.querySelectorAll("[data-gallery] .fl-bot")];
		for (const bot of bots) {
			const canvas = document.createElement("canvas");
			canvas.width = canvas.height = 24;
			const start = performance.now();
			try {
				if (!drawBotPixels(bot.querySelector("svg"), canvas, "#292536"))
					failures.push(bot.dataset.kind);
			} catch (error) {
				failures.push(`${bot.dataset.kind}: ${error.message}`);
			}
			durations.push(performance.now() - start);
		}
		return { count: bots.length, failures, maxDrawMs: Math.max(...durations) };
	}, rendererPath);
	assert.ok(
		crew.count >= 129,
		"The pixel geometry check must cover the complete crew",
	);
	assert.deepEqual(
		crew.failures,
		[],
		"Every equipped character must support the pixel renderer",
	);
	console.log(
		`Pixel renderer checks passed: ${crew.count} equipped characters, live rig, mask fades, pattern transforms, variants, paused stability; ${result.averageDrawMs.toFixed(2)} ms average equipped draw.`,
	);
	return result;
}
