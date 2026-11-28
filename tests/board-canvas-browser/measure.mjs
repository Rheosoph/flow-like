import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium } from "playwright";
import puppeteer from "puppeteer";

const production = process.env.BOARD_CANVAS_FIXTURE === "production";
const origin =
	process.env.BOARD_CANVAS_URL ??
	`http://127.0.0.1:${production ? 4339 : 4338}`;
const output = resolve(
	process.env.BOARD_CANVAS_MEASURE_OUTPUT ??
		`tmp/board-canvas-browser/${production ? "production-component" : "hybrid"}-timings`,
);
const trialCount = Number(process.env.BOARD_CANVAS_TRIALS ?? 3);
const durationMs = Number(process.env.BOARD_CANVAS_DURATION_MS ?? 3000);
const renderers = process.env.BOARD_CANVAS_RENDERERS?.split(",") ?? [
	"react-flow",
	"egui",
];
assert(
	renderers.length > 0 &&
		new Set(renderers).size === renderers.length &&
		renderers.every((renderer) => ["react-flow", "egui"].includes(renderer)),
	"Renderers must be react-flow, egui, or both separated by a comma",
);
const scenarios =
	production && process.env.BOARD_CANVAS_ACTIVITY !== "0"
		? ["idle", "simulated-active"]
		: ["idle"];
assert(
	Number.isInteger(trialCount) && trialCount > 0,
	"Trial count must be a positive integer",
);
assert(
	Number.isFinite(durationMs) && durationMs >= 1000,
	"Pan duration must be at least 1000 ms",
);
console.log(
	"Run without concurrent browser suites, builds or typechecks. Results describe this fixture and environment only.",
);
await mkdir(output, { recursive: true });
const args = ["--enable-webgl", "--enable-unsafe-swiftshader"];
const browser = await chromium.launch({
	headless: true,
	executablePath: process.env.CHROME_PATH ?? (await puppeteer.executablePath()),
	args,
});
const report = {
	startedAt: new Date().toISOString(),
	browser: browser.version(),
	origin,
	args,
	trialCount,
	durationMs,
	renderers,
	fixture: production
		? "Production FlowNode, layer, comment and media components with production canvas/gesture layer; simulated backend; Next development server; hybrid LOD enabled"
		: "Production canvas/gesture layer with simplified fixture node components; simulated authoritative backend and activity; Vite production build; hybrid LOD enabled",
	activityPattern: production
		? "Simulated-active keeps exec-b and native running, exec-a completed, integer-a heatmap/lint and integer-b log marks enabled. Static runtime state with normal component animations; no backend event throughput is measured."
		: "After pan measurements, 100 baseline native nodes are marked running and then cleared. This is an activity presentation sanity check, not an activity throughput timing.",
	viewport: {
		width: 1600,
		height: 1100,
		deviceScaleFactor: 2,
		x: 20,
		y: 10,
		zoom: 0.1,
	},
	method:
		"Middle-button pans by exactly 240×100 CSS pixels, 60 Hz target input rate; requestAnimationFrame intervals include renderer and host work. Renderer order is recorded. Start and end viewports are asserted for every trial, with stable zoom 0.1 before starting. No automatic competing-system-load detection.",
	results: [],
};
const save = () =>
	writeFile(`${output}/results.json`, JSON.stringify(report, null, 2));
try {
	for (const renderer of renderers) {
		const page = await browser.newPage({
			viewport: { width: 1600, height: 1100 },
			deviceScaleFactor: 2,
		});
		const errors = [];
		const assets = new Set();
		page.on("pageerror", (error) => errors.push(error.message));
		page.on("response", (response) => {
			if (/\.(?:js|wasm)(?:\?|$)/.test(response.url()))
				assets.add(response.url());
		});
		const url = new URL(origin);
		url.searchParams.set("boardRenderer", renderer);
		url.searchParams.set("nodes", "10000");
		const startupStarted = performance.now();
		console.log(
			`Starting ${renderer} (${production ? "production components" : "fixture components"})`,
		);
		await page.goto(url.href, {
			waitUntil: "domcontentloaded",
			timeout: 120000,
		});
		await page.waitForFunction(
			({ renderer, production }) =>
				(production ? window.__boardParity : window.__boardCanvasHarness) &&
				document.querySelector("[data-board-renderer]")?.dataset
					.boardRenderer === renderer,
			{ renderer, production },
			{ timeout: 120000 },
		);
		await page.evaluate(() => document.fonts.ready);
		const startupReadyMs = performance.now() - startupStarted;
		await page.evaluate((production) => {
			const api = production
				? window.__boardParity
				: window.__boardCanvasHarness;
			window.__boardMeasure = {
				snapshot: () => {
					const state = api.snapshot();
					return {
						viewport: state.viewport,
						presentation: state.presentation,
						runs: state.runs,
					};
				},
				viewport: () =>
					production ? api.viewport(20, 10, 0.1) : api.setViewport(20, 10, 0.1),
				activity: async (enabled) => {
					if (production) {
						await api.runtime(enabled);
						await api.runtimeNode(enabled ? "running" : "none");
						api.marks(enabled);
					} else await api.setActivity(enabled ? 100 : 0);
				},
			};
		}, production);
		const mounted = await page.evaluate(() => ({
			nodes: document.querySelectorAll(".react-flow__node").length,
			edges: document.querySelectorAll(".react-flow__edge").length,
		}));
		const environment = await page.evaluate(() => {
			const gl = document.createElement("canvas").getContext("webgl2");
			const ext = gl?.getExtension("WEBGL_debug_renderer_info");
			const result = {
				userAgent: navigator.userAgent,
				devicePixelRatio,
				webgl: gl
					? {
							renderer: gl.getParameter(
								ext ? ext.UNMASKED_RENDERER_WEBGL : gl.RENDERER,
							),
							vendor: gl.getParameter(
								ext ? ext.UNMASKED_VENDOR_WEBGL : gl.VENDOR,
							),
							version: gl.getParameter(gl.VERSION),
						}
					: null,
			};
			gl?.getExtension("WEBGL_lose_context")?.loseContext();
			return result;
		});
		const entry = {
			renderer,
			startupReadyMs,
			mounted,
			environment,
			presentation: (
				await page.evaluate(() => window.__boardMeasure.snapshot())
			).presentation,
			scenarios: [],
			errors,
			assets: [],
		};
		report.results.push(entry);
		await save();
		for (const scenario of scenarios) {
			await page.mouse.move(0, 0);
			await page.evaluate(
				(enabled) => window.__boardMeasure.activity(enabled),
				scenario !== "idle",
			);
			await page.waitForTimeout(500);
			const state = await page.evaluate(() => window.__boardMeasure.snapshot());
			if (scenario !== "idle")
				assert.ok(
					state.runs.includes("fixture-run") &&
						state.runs.includes("fixture-native-run"),
					"Simulated runtime state is active",
				);
			const measured = {
				name: scenario,
				presentation: state.presentation,
				runs: state.runs,
				trials: [],
			};
			entry.scenarios.push(measured);
			for (let trial = 0; trial < trialCount; trial++) {
				await page.mouse.move(0, 0);
				let before;
				let stable = false;
				// The production fixture applies its initial viewport asynchronously.
				// Let that settle before timing, and reject any later viewport reset.
				for (let attempt = 0; attempt < 5; attempt++) {
					await page.evaluate(() => window.__boardMeasure.viewport());
					await page.waitForTimeout(1000);
					const first = (
						await page.evaluate(() => window.__boardMeasure.snapshot())
					).viewport;
					await page.waitForTimeout(350);
					before = (await page.evaluate(() => window.__boardMeasure.snapshot()))
						.viewport;
					if (
						[first, before].every(
							(viewport) =>
								viewport.zoom === 0.1 && viewport.x === 20 && viewport.y === 10,
						)
					) {
						stable = true;
						break;
					}
				}
				assert(stable, "Measurement viewport settles before the pan");
				assert.deepEqual(before, { x: 20, y: 10, zoom: 0.1 });
				await page.mouse.move(810, 600);
				await page.mouse.down({ button: "middle" });
				await page.evaluate(() => {
					const trace = {
						samples: [],
						running: true,
						last: undefined,
						started: performance.now(),
					};
					window.__hybridPanTrace = trace;
					const frame = (now) => {
						if (!trace.running) return;
						if (trace.last !== undefined) trace.samples.push(now - trace.last);
						trace.last = now;
						requestAnimationFrame(frame);
					};
					requestAnimationFrame(frame);
				});
				const started = performance.now();
				let moves = 0;
				while (performance.now() - started < durationMs) {
					const t = (performance.now() - started) / durationMs;
					await page.mouse.move(810 + t * 240, 600 + t * 100);
					moves++;
					const delay = moves * (1000 / 60) - (performance.now() - started);
					if (delay > 0)
						await new Promise((resolve) => setTimeout(resolve, delay));
				}
				await page.mouse.move(1050, 700);
				moves++;
				await page.mouse.up({ button: "middle" });
				const trace = await page.evaluate(() => {
					const trace = window.__hybridPanTrace;
					trace.running = false;
					const sorted = [...trace.samples].sort((a, b) => a - b);
					return {
						elapsedMs: performance.now() - trace.started,
						frames: sorted.length,
						medianMs: sorted[Math.floor(sorted.length * 0.5)],
						p95Ms: sorted[Math.floor(sorted.length * 0.95)],
						maxMs: sorted.at(-1),
						over50ms: sorted.filter((value) => value > 50).length,
						samples: trace.samples,
					};
				});
				await page.mouse.move(0, 0);
				await page.waitForTimeout(500);
				const after = (
					await page.evaluate(() => window.__boardMeasure.snapshot())
				).viewport;
				assert.equal(after.zoom, 0.1, "Pan retains the measured zoom");
				assert(
					Math.abs(after.x - before.x - 240) < 0.01 &&
						Math.abs(after.y - before.y - 100) < 0.01,
					`Pan follows the same 240×100 extent: ${JSON.stringify({ before, after })}`,
				);
				assert(trace.frames > 0, "Pan collects animation frame intervals");
				assert.equal(
					await page
						.locator("[data-board-renderer]")
						.getAttribute("data-board-renderer"),
					renderer,
					"Measured renderer remains active",
				);
				measured.trials.push({
					trial: trial + 1,
					targetDurationMs: durationMs,
					moves,
					before,
					after,
					...trace,
				});
				console.log(
					JSON.stringify({
						renderer,
						scenario,
						trial: trial + 1,
						medianMs: trace.medianMs,
						p95Ms: trace.p95Ms,
						frames: trace.frames,
					}),
				);
				await save();
			}
		}
		// Hover promotion has a 180 ms retention delay; wait beyond it before comparing activity membership.
		await page.mouse.move(0, 0);
		await page.waitForTimeout(500);
		const beforeActivity = (
			await page.evaluate(() => window.__boardMeasure.snapshot())
		).presentation;
		await page.evaluate(() => window.__boardMeasure.activity(true));
		await page.waitForTimeout(500);
		entry.activity = (
			await page.evaluate(() => window.__boardMeasure.snapshot())
		).presentation;
		if (!production) {
			assert.equal(entry.activity.activeNodes, 100);
			if (renderer === "egui")
				assert.equal(entry.activity.domNodes, beforeActivity.domNodes + 100);
		}
		await page.evaluate(() => window.__boardMeasure.activity(false));
		await page.waitForTimeout(500);
		entry.cleared = (
			await page.evaluate(() => window.__boardMeasure.snapshot())
		).presentation;
		if (!production) {
			assert.equal(entry.cleared.activeNodes, 0);
			if (renderer === "egui")
				assert.equal(entry.cleared.domNodes, beforeActivity.domNodes);
		}
		entry.finalRenderer = await page
			.locator("[data-board-renderer]")
			.getAttribute("data-board-renderer");
		assert.equal(
			entry.finalRenderer,
			renderer,
			"Measured renderer remains active after runtime updates",
		);
		assert.deepEqual(errors, []);
		entry.assets = [...assets];
		await save();
		await page.close();
	}
	report.finishedAt = new Date().toISOString();
	await save();
} catch (error) {
	report.failure = String(error);
	await save();
	throw error;
} finally {
	await browser.close();
}
