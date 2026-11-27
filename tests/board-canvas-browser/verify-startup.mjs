import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const origin = process.env.BOARD_CANVAS_URL ?? "http://127.0.0.1:4338";
const output =
	process.env.BOARD_STARTUP_OUTPUT ??
	`tmp/board-canvas-browser/startup/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				executablePath: await puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const results = [];
const errors = [];
const cases = ["hidden", "collapsed", "stalled", "reduced-motion"].filter(
	(name) =>
		!process.env.BOARD_STARTUP_CASE || process.env.BOARD_STARTUP_CASE === name,
);
const stats = (page) =>
	page.evaluate(() =>
		window.__startupHandles
			.filter((record) => !record.destroyed)
			.map((record) => record.actualStats()),
	);
const renderer = (page) =>
	page.locator("[data-board-renderer]").getAttribute("data-board-renderer");
const waitRenderer = (page, expected) =>
	page.waitForFunction(
		(value) =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === value,
		expected,
	);

try {
	for (const name of cases) {
		const context = await browser.newContext({
			viewport: { width: 1280, height: 960 },
			deviceScaleFactor: 2,
			reducedMotion: name === "reduced-motion" ? "reduce" : "no-preference",
		});
		try {
			// Headless Chromium/WebKit keep every tab visible. Simulate the browser's
			// visibility signal and suspended RAF callbacks; the WASM renderer is real.
			await context.addInitScript(
				({ name }) => {
					window.__startupHidden = name === "hidden";
					window.__startupCollapsed = name === "collapsed";
					window.__startupStalled = name === "stalled";
					window.__startupHandles = [];
					Object.defineProperty(document, "hidden", {
						configurable: true,
						get: () => window.__startupHidden,
					});
					Object.defineProperty(document, "visibilityState", {
						configurable: true,
						get: () => (window.__startupHidden ? "hidden" : "visible"),
					});
					const request = window.requestAnimationFrame.bind(window);
					const cancel = window.cancelAnimationFrame.bind(window);
					let next = 0;
					const pending = new Map();
					const schedule = (id, entry) => {
						if (window.__startupHidden || entry.native !== undefined) return;
						entry.native = request((time) => {
							entry.native = undefined;
							if (window.__startupHidden) return;
							pending.delete(id);
							entry.callback(time);
						});
					};
					window.requestAnimationFrame = (callback) => {
						const id = ++next;
						const entry = { callback, native: undefined };
						pending.set(id, entry);
						schedule(id, entry);
						return id;
					};
					window.cancelAnimationFrame = (id) => {
						const entry = pending.get(id);
						if (entry?.native !== undefined) cancel(entry.native);
						pending.delete(id);
					};
					window.__revealStartup = () => {
						window.__startupHidden = false;
						window.__startupCollapsed = false;
						for (const record of window.__startupHandles)
							record.canvas.style.display = "";
						document.dispatchEvent(new Event("visibilitychange"));
						for (const [id, entry] of pending) schedule(id, entry);
					};
				},
				{ name },
			);
			await context.route("**/board-canvas/board_canvas.js", (route) =>
				route.fulfill({
					contentType: "text/javascript",
					body: `export { default } from "/board-canvas/board_canvas.js?startupProbe=true";
import { start as startCanvas } from "/board-canvas/board_canvas.js?startupProbe=true";
export async function start(canvas, ...args) {
  if (window.__startupCollapsed) canvas.style.display = "none";
  const handle = await startCanvas(canvas, ...args);
  const originalStats = handle.stats.bind(handle);
  const originalDestroy = handle.destroy.bind(handle);
  const record = { canvas, destroyed: false, pulseCalls: 0, actualStats: () => ({ ...JSON.parse(originalStats()), pulseCalls: record.pulseCalls }) };
  window.__startupHandles.push(record);
  if (handle.pulse) {
    const originalPulse = handle.pulse.bind(handle);
    handle.pulse = () => { record.pulseCalls++; return originalPulse(); };
  }
  handle.stats = () => {
    const value = record.actualStats();
    if (window.__startupStalled) value.frames = 0;
    return JSON.stringify(value);
  };
  handle.destroy = () => { record.destroyed = true; return originalDestroy(); };
  return handle;
}`,
				}),
			);
			const page = await context.newPage();
			page.setDefaultTimeout(20_000);
			page.on("pageerror", (error) =>
				errors.push({ name, message: error.message }),
			);
			await page.goto(`${origin}/?boardRenderer=egui`, {
				waitUntil: "domcontentloaded",
			});
			await page.waitForFunction(
				() => window.__startupHandles.length === 2,
				null,
				{ polling: 100 },
			);
			const initial = await stats(page);
			let suspended;
			if (name === "hidden" || name === "collapsed") {
				await page.waitForTimeout(11_500);
				suspended = await stats(page);
				assert.equal(
					await renderer(page),
					"loading",
					"A hidden or collapsed startup must not expire its paint budget",
				);
				if (name === "hidden")
					assert.ok(
						(await stats(page)).every((value) => value.frames === 0),
						"Hidden tabs have not run a native UI frame",
					);
				await page.evaluate(() => window.__revealStartup());
				await waitRenderer(page, "egui");
			} else if (name === "stalled") {
				const fallback = page.locator(
					'[data-board-renderer-status="fallback"]',
				);
				await fallback.waitFor();
				await fallback.locator("button").first().click();
				assert.match(
					await page.locator("[data-board-renderer-panel]").innerText(),
					/The native canvas did not paint in time/,
				);
				await waitRenderer(page, "react-flow");
				assert.equal(
					await page.evaluate(() =>
						localStorage.getItem("flow-board-renderer"),
					),
					"egui",
				);
				await page.evaluate(() => {
					window.__startupStalled = false;
				});
				await page
					.getByRole("button", { name: "Retry WASM", exact: true })
					.click();
				await waitRenderer(page, "egui");
			} else {
				await waitRenderer(page, "egui");
			}
			if (name !== "reduced-motion") {
				await page.waitForFunction(
					() =>
						window.__startupHandles.some(
							(record) =>
								!record.destroyed && record.actualStats().activationPulse,
						),
					null,
					{ timeout: 2000, polling: 25 },
				);
			}
			const active = await stats(page);
			assert.equal(
				await page.locator('[data-board-renderer-status="active"]').count(),
				1,
				"The badge reports the actual active renderer",
			);
			assert.ok(active.every((value) => value.frames > 0));
			if (suspended) {
				assert.ok(
					active.every(
						(value, index) => value.frames > suspended[index].frames,
					),
					"Activation requires a fresh frame after the canvas becomes drawable",
				);
			}
			if (name === "reduced-motion") {
				assert.ok(
					active.every(
						(value) =>
							!value.activationPulse &&
							value.pulseEdgesDrawn === 0 &&
							value.pulseCalls === 0,
					),
					"Reduced motion skips the activation effect",
				);
			} else {
				assert.ok(
					active.some((value) => value.activationPulse),
					"Activation starts the bounded connection pulse",
				);
			}
			await page.waitForTimeout(1400);
			const ended = await stats(page);
			assert.ok(
				ended.every(
					(value) => !value.activationPulse && value.pulseEdgesDrawn === 0,
				),
				"Activation pulse ends",
			);
			await page.waitForTimeout(350);
			const idle = await stats(page);
			assert.deepEqual(
				idle.map((value) => value.frames),
				ended.map((value) => value.frames),
				"The canvas is idle between scheduled finite sweeps",
			);
			await page.screenshot({ path: `${output}/${name}.png` });
			results.push({
				name,
				initial,
				suspended,
				active,
				ended,
				idle,
				visibility:
					"simulated document visibility and RAF suspension; actual WASM canvas",
			});
			console.log(
				`PASS ${name}: startup, native paint, bounded pulse, idle frames`,
			);
		} finally {
			await context.close();
		}
	}
	assert.deepEqual(errors, []);
} finally {
	await browser.close();
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ browser: kind, results, errors }, null, 2),
	);
	console.log(
		`Browser closed; ${results.length}/${cases.length} startup cases completed, ${errors.length} page errors.`,
	);
}
