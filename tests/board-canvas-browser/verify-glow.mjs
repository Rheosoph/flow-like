import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const output = `tmp/board-canvas-browser/glow/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				executablePath: await puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const errors = [];
const result = {};
const page = await browser.newPage({
	viewport: { width: 1280, height: 960 },
	reducedMotion: "no-preference",
});
page.setDefaultTimeout(15_000);
page.on("pageerror", (error) => errors.push(error.message));
const stats = () =>
	page.evaluate(() =>
		window.__glowHandles.map((handle) => JSON.parse(handle.stats())),
	);
const edgeStats = () =>
	page.evaluate(() =>
		window.__glowHandles
			.map((handle) => JSON.parse(handle.stats()))
			.find((value) => value.paintLayer === "edges"),
	);
const wait = (predicate) =>
	page.waitForFunction(predicate, null, { polling: 25 });

try {
	await page.addInitScript(() => {
		window.__glowHidden = false;
		window.__glowHandles = [];
		Object.defineProperty(document, "hidden", {
			configurable: true,
			get: () => window.__glowHidden,
		});
		Object.defineProperty(document, "visibilityState", {
			configurable: true,
			get: () => (window.__glowHidden ? "hidden" : "visible"),
		});
		window.__setGlowHidden = (hidden) => {
			window.__glowHidden = hidden;
			document.dispatchEvent(new Event("visibilitychange"));
		};
	});
	await page.route("**/board-canvas/board_canvas.js", (route) =>
		route.fulfill({
			contentType: "text/javascript",
			body: `export { default } from "/board-canvas/board_canvas.js?glowProbe=true";
import { start as startCanvas } from "/board-canvas/board_canvas.js?glowProbe=true";
export async function start(...args) {
  const handle = await startCanvas(...args);
  window.__glowHandles.push(handle);
  return handle;
}`,
		}),
	);
	await page.goto("http://127.0.0.1:4338/?boardRenderer=egui", {
		waitUntil: "domcontentloaded",
	});
	await page.locator('[data-board-renderer-status="active"]').waitFor();
	await wait(() =>
		window.__glowHandles.some(
			(handle) => JSON.parse(handle.stats()).activationPulse,
		),
	);
	result.first = await stats();
	assert.equal(
		result.first.find((value) => value.paintLayer === "edges").pulseCount,
		1,
	);
	assert.equal(
		result.first.find((value) => value.paintLayer === "nodes").pulseCount,
		0,
	);
	await wait(() =>
		window.__glowHandles.every(
			(handle) => !JSON.parse(handle.stats()).activationPulse,
		),
	);
	await page.waitForTimeout(150);
	result.ended = await stats();
	result.firstSweepFrames =
		result.ended.find((value) => value.paintLayer === "edges").frames -
		result.first.find((value) => value.paintLayer === "edges").frames;
	assert.ok(
		result.firstSweepFrames <= 32,
		`The 1.2-second sweep stays within 32 frames including startup allowance; observed ${result.firstSweepFrames}`,
	);
	await page.waitForTimeout(350);
	result.quiet = await stats();
	assert.deepEqual(
		result.quiet.map((value) => value.frames),
		result.ended.map((value) => value.frames),
		"The renderer is idle between finite sweeps",
	);
	await wait(() =>
		window.__glowHandles.some(
			(handle) =>
				JSON.parse(handle.stats()).pulseCount >= 2 &&
				JSON.parse(handle.stats()).activationPulse &&
				JSON.parse(handle.stats()).pulseEdgesDrawn > 0,
		),
	);
	result.recurring = await edgeStats();
	assert.equal(result.recurring.pulseCount, 2);
	assert.ok(
		result.recurring.pulseEdgesDrawn > 0 &&
			result.recurring.pulseEdgesDrawn <= 32,
	);
	await page.screenshot({ path: `${output}/recurring.png` });
	await page.emulateMedia({ reducedMotion: "reduce" });
	await wait(() =>
		window.__glowHandles.every(
			(handle) => !JSON.parse(handle.stats()).activationPulse,
		),
	);
	await page.waitForTimeout(150);
	result.reduced = await stats();
	await page.waitForTimeout(1500);
	result.reducedQuiet = await stats();
	assert.deepEqual(
		result.reducedQuiet.map((value) => [value.frames, value.pulseCount]),
		result.reduced.map((value) => [value.frames, value.pulseCount]),
		"Live reduced motion stops the sweep and leaves no animation loop",
	);
	await page.emulateMedia({ reducedMotion: "no-preference" });
	await wait(() =>
		window.__glowHandles.some(
			(handle) =>
				JSON.parse(handle.stats()).pulseCount >= 3 &&
				JSON.parse(handle.stats()).activationPulse,
		),
	);
	await page.evaluate(() => window.__setGlowHidden(true));
	await wait(() =>
		window.__glowHandles.every(
			(handle) => !JSON.parse(handle.stats()).activationPulse,
		),
	);
	result.hidden = await edgeStats();
	await page.waitForTimeout(1500);
	assert.equal(
		(await edgeStats()).pulseCount,
		result.hidden.pulseCount,
		"A hidden document does not restart the glow",
	);
	await page.evaluate(() => window.__setGlowHidden(false));
	await wait(() =>
		window.__glowHandles.some(
			(handle) =>
				JSON.parse(handle.stats()).pulseCount >= 4 &&
				JSON.parse(handle.stats()).activationPulse,
		),
	);
	result.resumed = await edgeStats();
	assert.equal(result.resumed.pulseCount, result.hidden.pulseCount + 1);
	assert.equal(
		result.resumed.geometryRebuilds,
		result.first.find((value) => value.paintLayer === "edges").geometryRebuilds,
		"Decorative sweeps reuse cached connection geometry",
	);
	assert.deepEqual(errors, []);
	console.log(
		"PASS finite recurring glow, quiet interval, edge-only ownership, reduced-motion cancel/restart, visibility cancel/restart",
	);
} finally {
	await browser.close();
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{
				browser: kind,
				result,
				errors,
				visibility:
					"Document visibility is simulated; media preference and native renderer are real",
			},
			null,
			2,
		),
	);
	console.log(`Browser closed; ${errors.length} page errors.`);
}
