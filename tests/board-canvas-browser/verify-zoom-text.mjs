import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const origin = process.env.BOARD_CANVAS_URL ?? "http://127.0.0.1:4338";
const output =
	process.env.BOARD_CANVAS_TEXT_OUTPUT ??
	`tmp/board-canvas-browser/zoom-text/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				executablePath:
					process.env.CHROME_PATH ?? (await puppeteer.executablePath()),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const page = await browser.newPage({
	viewport: { width: 1600, height: 1100 },
	deviceScaleFactor: 2,
	reducedMotion: "reduce",
});
const errors = [];
const assets = new Set();
const results = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("response", (response) => {
	if (/\.(wasm|js)(\?|$)/.test(response.url())) assets.add(response.url());
});
try {
	await page.addInitScript(() => {
		window.__textHandles = [];
	});
	await page.route("**/board-canvas/board_canvas.js", (route) =>
		route.fulfill({
			contentType: "text/javascript",
			body: `export { default } from "/board-canvas/board_canvas.js?textProbe=1";
import { start as startCanvas } from "/board-canvas/board_canvas.js?textProbe=1";
export async function start(...args) {
  const handle = await startCanvas(...args);
  window.__textHandles.push(handle);
  return handle;
}`,
		}),
	);
	await page.goto(`${origin}/?boardRenderer=egui&nodes=10000`, {
		waitUntil: "networkidle",
	});
	await page.waitForFunction(() => window.__boardCanvasHarness?.ready(), null, {
		timeout: 60000,
	});
	await page.mouse.move(0, 0);
	for (const zoom of [0.25, 0.5, 1, 1.2, 2.5]) {
		await page.evaluate(
			(zoom) =>
				window.__boardCanvasHarness.setViewport(
					500 - 300 * zoom,
					300 - 900 * zoom,
					zoom,
				),
			zoom,
		);
		await page.waitForTimeout(400);
		assert.equal(
			await page.locator('.react-flow__node[data-id="extra-11"]').count(),
			0,
			"The comparison node's labels must be painted natively",
		);
		const snapshot = await page.evaluate(() => {
			const value = window.__boardCanvasHarness.snapshot();
			return {
				viewport: value.viewport,
				presentation: value.presentation,
				node: window.__boardCanvasHarness.node("extra-11"),
				input: window.__boardCanvasHarness.pin("extra-11", "extra-11.in"),
				output: window.__boardCanvasHarness.pin("extra-11", "extra-11.out"),
				native: window.__textHandles.map((handle) =>
					JSON.parse(handle.stats()),
				),
			};
		});
		assert.equal(snapshot.viewport.zoom, zoom);
		await page.screenshot({ path: `${output}/zoom-${zoom}.png` });
		results.push({ zoom, ...snapshot });
	}
	await page.waitForTimeout(350);
	const quiet = await page.evaluate(() =>
		window.__textHandles.map((handle) => JSON.parse(handle.stats())),
	);
	assert.deepEqual(
		quiet.map(({ paintLayer, frames, textLayoutCount }) => ({
			paintLayer,
			frames,
			textLayoutCount,
		})),
		results.at(-1).native.map(({ paintLayer, frames, textLayoutCount }) => ({
			paintLayer,
			frames,
			textLayoutCount,
		})),
		"Settled native text must not keep laying out or repainting while idle",
	);
	assert.deepEqual(errors, []);
	console.log(JSON.stringify({ kind, output, results, errors }));
} finally {
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{ kind, origin, assets: [...assets], results, errors },
			null,
			2,
		),
	);
	await browser.close();
}
