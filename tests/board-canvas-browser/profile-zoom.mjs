import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const engine = process.env.BOARD_ZOOM_BROWSER ?? "chromium";
const production = process.env.BOARD_ZOOM_FIXTURE === "production";
const origin =
	process.env.BOARD_ZOOM_URL ?? `http://127.0.0.1:${production ? 4339 : 4338}`;
const renderers = (process.env.BOARD_ZOOM_RENDERERS ?? "egui,react-flow").split(
	",",
);
const minimaps = (process.env.BOARD_ZOOM_MINIMAPS ?? "normal").split(",");
const duration = Number(process.env.BOARD_ZOOM_DURATION_MS ?? 1800);
const steps = Number(process.env.BOARD_ZOOM_STEPS ?? 90);
const cpu = engine === "chromium" && process.env.BOARD_ZOOM_CPU !== "0";
const pauseNative = process.env.BOARD_ZOOM_NATIVE === "paused";
const hideBackground = process.env.BOARD_ZOOM_BACKGROUND === "hidden";
const hideRichContent = process.env.BOARD_ZOOM_DOM === "hidden";
const promoteWorldPlanes = process.env.BOARD_ZOOM_PLANES === "composited";
const promoteRichNodes = process.env.BOARD_ZOOM_DOM === "composited";
const wasmOverridePath = process.env.BOARD_ZOOM_WASM_PATH
	? resolve(process.env.BOARD_ZOOM_WASM_PATH)
	: undefined;
const wasmOverride = wasmOverridePath
	? await readFile(wasmOverridePath)
	: undefined;
const output = resolve(
	process.env.BOARD_ZOOM_OUTPUT ??
		`tmp/board-canvas-browser/zoom-profile/${engine}`,
);
assert.ok(renderers.every((value) => ["egui", "react-flow"].includes(value)));
assert.ok(
	minimaps.every((value) =>
		["normal", "css-hidden", "raf-paused"].includes(value),
	),
);
assert.ok(duration >= 500 && Number.isInteger(steps) && steps > 0);
await mkdir(output, { recursive: true });
const args = ["--enable-webgl", "--enable-unsafe-swiftshader"];
const browser =
	engine === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				executablePath:
					process.env.CHROME_PATH ?? (await puppeteer.executablePath()),
				args,
			});
const report = {
	started: new Date().toISOString(),
	browser: browser.version(),
	engine,
	origin,
	wasmOverridePath,
	fixture: production
		? "Actual production node/comment/media components, simulated backend, Next dev"
		: "Production canvas and gesture layer, simplified node fixture, simulated backend, Vite build",
	method:
		"10,000-node fixture; real positive-delta wheel events zoom from 1 to 0.02 around a stationary pointer over the canvas. WebKit wheel deltas are rounded upward to an integer before device-scale calibration to avoid truncation; Chromium retains fractional deltas. No viewport setter is used during the measured interval. Counts and native stats are sampled at zoom thresholds; sample overhead is recorded. CPU profiling and method wrappers add diagnostic overhead.",
	minimapNote:
		"css-hidden suppresses compositing only. raf-paused identifies the native minimap's RAF callback through its actual canvas drawImage, then skips that callback on a fresh page; subscriptions remain active. Neither variant claims to unmount the minimap or eliminate all subscription overhead. React Flow only runs normal/CSS-hidden.",
	motion: process.env.BOARD_ZOOM_MOTION === "1" ? "no-preference" : "reduce",
	duration,
	steps,
	cpu,
	pauseNative,
	hideBackground,
	hideRichContent,
	promoteWorldPlanes,
	promoteRichNodes,
	viewport: { width: 1600, height: 1100, deviceScaleFactor: 2 },
	results: [],
};
const save = () =>
	writeFile(`${output}/results.json`, JSON.stringify(report, null, 2));
const quantile = (values, fraction) =>
	values[Math.min(values.length - 1, Math.floor(values.length * fraction))];
function summarizeCpu(profile) {
	const nodes = new Map(profile.nodes.map((node) => [node.id, node]));
	const parents = new Map();
	for (const node of profile.nodes)
		for (const id of node.children ?? []) parents.set(id, node.id);
	const functions = new Map();
	const keyFor = (node) =>
		`${node.callFrame.functionName || "(anonymous)"}@${node.callFrame.url}:${node.callFrame.lineNumber + 1}`;
	for (let index = 0; index < (profile.samples?.length ?? 0); index++) {
		const delta = (profile.timeDeltas?.[index] ?? 0) / 1000;
		const leaf = profile.samples[index];
		const seen = new Set();
		for (let id = leaf; id !== undefined; id = parents.get(id)) {
			const node = nodes.get(id);
			if (!node) break;
			const key = keyFor(node);
			let row = functions.get(key);
			if (!row) {
				row = {
					function: node.callFrame.functionName || "(anonymous)",
					url: node.callFrame.url,
					line: node.callFrame.lineNumber + 1,
					selfMs: 0,
					inclusiveMs: 0,
				};
				functions.set(key, row);
			}
			if (id === leaf) row.selfMs += delta;
			if (!seen.has(key)) row.inclusiveMs += delta;
			seen.add(key);
		}
	}
	return {
		sampledMs: (profile.timeDeltas ?? []).reduce((a, b) => a + b, 0) / 1000,
		self: [...functions.values()]
			.sort((a, b) => b.selfMs - a.selfMs)
			.slice(0, 40),
		inclusive: [...functions.values()]
			.sort((a, b) => b.inclusiveMs - a.inclusiveMs)
			.slice(0, 40),
	};
}

try {
	for (const renderer of renderers)
		for (const minimap of minimaps) {
			if (renderer === "react-flow" && minimap === "raf-paused") continue;
			const name = `${renderer}-${minimap}`;
			const context = await browser.newContext({
				deviceScaleFactor: report.viewport.deviceScaleFactor,
				viewport: { width: 1600, height: 1100 },
				reducedMotion: report.motion,
			});
			const page = await context.newPage();
			page.setDefaultTimeout(30_000);
			const errors = [];
			const assets = new Set();
			page.on("pageerror", (error) => errors.push(error.message));
			page.on("response", (response) => {
				if (/\.(js|wasm)(\?|$)/.test(response.url()))
					assets.add(response.url());
			});
			try {
				await page.addInitScript(() => {
					const probe = {
						handles: [],
						trace: undefined,
						minimapCallbacks: new WeakSet(),
						learnedMinimapCallbacks: 0,
						minimapPaused: false,
						suppressedMinimapFrames: 0,
						currentRaf: undefined,
						nativeCallbacks: new WeakSet(),
						pauseNative: false,
					};
					window.__zoomProbe = probe;
					const originalRaf = requestAnimationFrame.bind(window);
					window.requestAnimationFrame = (callback) =>
						originalRaf((now) => {
							if (probe.minimapPaused && probe.minimapCallbacks.has(callback)) {
								probe.suppressedMinimapFrames++;
								return;
							}
							const previous = probe.currentRaf;
							probe.currentRaf = callback;
							const started = performance.now();
							try {
								return callback(now);
							} finally {
								if (probe.trace?.running)
									probe.trace.rafWork.push({
										at: started - probe.trace.started,
										ms: performance.now() - started,
										kind: probe.nativeCallbacks.has(callback)
											? "native"
											: probe.minimapCallbacks.has(callback)
												? "minimap"
												: "other",
									});
								probe.currentRaf = previous;
							}
						});
					for (const type of [
						window.WebGLRenderingContext,
						window.WebGL2RenderingContext,
					]) {
						if (!type) continue;
						const draw = type.prototype.drawElements;
						type.prototype.drawElements = function (...args) {
							if (probe.currentRaf) probe.nativeCallbacks.add(probe.currentRaf);
							return draw.apply(this, args);
						};
					}
					const originalDraw = CanvasRenderingContext2D.prototype.drawImage;
					CanvasRenderingContext2D.prototype.drawImage = function (...args) {
						if (!this.canvas.closest?.(".react-flow__minimap"))
							return originalDraw.apply(this, args);
						if (
							probe.currentRaf &&
							!probe.minimapCallbacks.has(probe.currentRaf)
						) {
							probe.minimapCallbacks.add(probe.currentRaf);
							probe.learnedMinimapCallbacks++;
						}
						const started = performance.now();
						try {
							return originalDraw.apply(this, args);
						} finally {
							if (probe.trace?.running)
								probe.trace.minimapDraws.push({
									at: started - probe.trace.started,
									ms: performance.now() - started,
								});
						}
					};
					probe.register = (handle) => {
						const entry = {
							handle,
							alive: true,
							readStats: handle.stats.bind(handle),
						};
						probe.handles.push(entry);
						for (const method of [
							"set_scene",
							"set_viewport",
							"set_positions",
							"set_overlay",
							"set_palette",
							"set_lod",
							"destroy",
						]) {
							const original = handle[method].bind(handle);
							handle[method] = (...args) => {
								if (method === "set_viewport") probe.viewportZoom = args[2];
								if (
									probe.pauseNative &&
									probe.trace?.running &&
									["set_viewport", "set_overlay", "set_positions"].includes(
										method,
									)
								)
									return;
								const started = performance.now();
								try {
									return original(...args);
								} finally {
									if (method === "destroy") entry.alive = false;
									if (probe.trace?.running)
										probe.trace.nativeCalls.push({
											method,
											at: started - probe.trace.started,
											ms: performance.now() - started,
											characters:
												typeof args[0] === "string"
													? args[0].length
													: undefined,
											viewport: method === "set_viewport" ? args : undefined,
										});
								}
							};
						}
					};
					probe.stats = () =>
						probe.handles
							.filter((entry) => entry.alive)
							.map((entry) => {
								const stats = entry.readStats();
								return typeof stats === "string" ? JSON.parse(stats) : stats;
							});
					probe.dom = () => ({
						nodes: document.querySelectorAll(".react-flow__node").length,
						edges: document.querySelectorAll(".react-flow__edge").length,
						total: document.getElementsByTagName("*").length,
						minimapElements: document.querySelectorAll(".react-flow__minimap *")
							.length,
						nativeCanvases: document.querySelectorAll(
							'[data-testid="egui-board-edges"], [data-testid="egui-board-canvas"]',
						).length,
					});
					probe.zoom = () => {
						if (probe.viewportZoom !== undefined) return probe.viewportZoom;
						const style = document.querySelector(
							".react-flow__viewport",
						)?.style;
						const nativeZoom = style?.getPropertyValue("--board-camera-zoom");
						if (nativeZoom) return Number(nativeZoom);
						const transform = style?.transform ?? "";
						return Number(transform.match(/scale\(([^)]+)\)/)?.[1]);
					};
					const addLongTasks = (entries) => {
						const trace = probe.trace;
						if (!trace) return;
						for (const entry of entries)
							if (
								entry.startTime >= trace.started &&
								(!trace.ended || entry.startTime <= trace.ended)
							)
								trace.longTasks.push({
									at: entry.startTime - trace.started,
									duration: entry.duration,
									name: entry.name,
								});
					};
					probe.longTaskSupported =
						PerformanceObserver.supportedEntryTypes.includes("longtask");
					if (probe.longTaskSupported) {
						probe.observer = new PerformanceObserver((list) =>
							addLongTasks(list.getEntries()),
						);
						probe.observer.observe({ type: "longtask", buffered: false });
					}
					probe.begin = () => {
						const trace = {
							started: performance.now(),
							running: true,
							samples: [],
							milestones: [],
							nativeCalls: [],
							minimapDraws: [],
							longTasks: [],
							rafWork: [],
							wheelEvents: [],
						};
						probe.trace = trace;
						const levels = [1, 0.75, 0.5, 0.25, 0.1, 0.05, 0.02];
						let previous;
						const frame = (now) => {
							if (!trace.running) return;
							const zoom = probe.zoom();
							if (previous !== undefined)
								trace.samples.push({
									at: now - trace.started,
									ms: now - previous,
									zoom,
								});
							previous = now;
							while (levels.length && zoom <= levels[0] + 0.000001) {
								const start = performance.now();
								trace.milestones.push({
									threshold: levels.shift(),
									zoom,
									at: now - trace.started,
									dom: probe.dom(),
									native: probe.stats(),
									samplingMs: performance.now() - start,
								});
							}
							requestAnimationFrame(frame);
						};
						requestAnimationFrame(frame);
					};
					window.addEventListener(
						"wheel",
						(event) => {
							probe.lastWheel = {
								deltaY: event.deltaY,
								deltaMode: event.deltaMode,
							};
							if (probe.trace?.running)
								probe.trace.wheelEvents.push({
									at: performance.now() - probe.trace.started,
									deltaY: event.deltaY,
									deltaMode: event.deltaMode,
									ctrl: event.ctrlKey,
									target:
										event.target instanceof Element
											? event.target.className?.toString()
											: undefined,
									noWheel:
										event.target instanceof Element
											? Boolean(event.target.closest(".nowheel"))
											: false,
								});
						},
						{ capture: true, passive: true },
					);
					probe.finish = () => {
						probe.trace.running = false;
						probe.trace.ended = performance.now();
						addLongTasks(probe.observer?.takeRecords() ?? []);
						return {
							...probe.trace,
							elapsed: probe.trace.ended - probe.trace.started,
							learnedMinimapCallbacks: probe.learnedMinimapCallbacks,
							suppressedMinimapFrames: probe.suppressedMinimapFrames,
						};
					};
				});
				await page.route("**/board-canvas/board_canvas.js", (route) =>
					route.fulfill({
						contentType: "text/javascript",
						body: `export {default} from "/board-canvas/board_canvas.js?zoomProfile=1";
import {start as nativeStart} from "/board-canvas/board_canvas.js?zoomProfile=1";
export async function start(...args) { const handle=await nativeStart(...args); window.__zoomProbe.register(handle); return handle; }`,
					}),
				);
				if (wasmOverride)
					await page.route("**/board-canvas/board_canvas_bg.wasm", (route) =>
						route.fulfill({
							contentType: "application/wasm",
							body: wasmOverride,
						}),
					);
				const url = new URL(origin);
				url.searchParams.set("boardRenderer", renderer);
				url.searchParams.set("nodes", "10000");
				console.log(`Starting ${name}`);
				await page.goto(url.href, {
					waitUntil: "domcontentloaded",
					timeout: 120_000,
				});
				await page.waitForFunction(
					({ renderer, production }) =>
						(production ? window.__boardParity : window.__boardCanvasHarness) &&
						document
							.querySelector("[data-board-renderer]")
							?.getAttribute("data-board-renderer") === renderer,
					{ renderer, production },
					{ timeout: 120_000 },
				);
				await page.evaluate(() => document.fonts.ready);
				await page.waitForTimeout(500);
				const snapshot = () =>
					page.evaluate((production) => {
						const state = (
							production ? window.__boardParity : window.__boardCanvasHarness
						).snapshot();
						const graph = state.board?.nodes ?? state.nodes;
						return {
							graphNodes: Object.keys(graph).length,
							graphEdges: state.presentation.edges ?? state.edges.length,
							presentation: state.presentation,
							viewport: state.viewport,
							bounds: state.bounds,
							dom: window.__zoomProbe.dom(),
							native: window.__zoomProbe.stats(),
							longTaskSupported: window.__zoomProbe.longTaskSupported,
						};
					}, production);
				const initial = await snapshot();
				assert.equal(
					initial.graphNodes,
					10000,
					"The authoritative graph contains exactly10,000 nodes",
				);
				const bounds = initial.bounds ?? {
					x: 80,
					y: 100,
					width: 22000,
					height: 12000,
				};
				const surface = await page.locator(".react-flow").boundingBox();
				assert.ok(surface);
				const x = surface.width / 2 - (bounds.x + bounds.width / 2);
				const y = surface.height / 2 - (bounds.y + bounds.height / 2);
				await page.evaluate(
					({ production, x, y }) => {
						const api = production
							? window.__boardParity
							: window.__boardCanvasHarness;
						return production
							? api.viewport(x, y, 1)
							: api.setViewport(x, y, 1);
					},
					{ production, x, y },
				);
				await page.mouse.move(
					surface.x + surface.width / 2,
					surface.y + surface.height / 2,
				);
				await page.waitForTimeout(600);
				if (hideBackground)
					await page.addStyleTag({
						content: ".react-flow__background {visibility:hidden!important}",
					});
				if (hideRichContent)
					await page.addStyleTag({
						content: "[data-board-rich-node] {visibility:hidden!important}",
					});
				if (promoteWorldPlanes)
					await page.addStyleTag({
						content: "[data-board-world-plane] {will-change:transform}",
					});
				if (promoteRichNodes)
					await page.addStyleTag({
						content: "[data-board-rich-node] {will-change:transform}",
					});
				await page.mouse.wheel(0, 120);
				await page.waitForTimeout(150);
				const wheelCalibration = await page.evaluate(
					() => window.__zoomProbe.lastWheel,
				);
				assert.ok(
					wheelCalibration?.deltaY > 0 && wheelCalibration.deltaMode === 0,
				);
				const inputScale = 120 / wheelCalibration.deltaY;
				await page.evaluate(
					({ production, x, y }) => {
						const api = production
							? window.__boardParity
							: window.__boardCanvasHarness;
						return production
							? api.viewport(x, y, 1)
							: api.setViewport(x, y, 1);
					},
					{ production, x, y },
				);
				await page.waitForTimeout(350);
				if (minimap === "css-hidden")
					await page.addStyleTag({
						content: ".react-flow__minimap {display:none!important}",
					});
				if (minimap === "raf-paused") {
					await page.waitForFunction(
						() => window.__zoomProbe.learnedMinimapCallbacks > 0,
					);
					await page.evaluate(() => {
						window.__zoomProbe.minimapPaused = true;
					});
				}
				const before = await snapshot();
				assert.equal(before.viewport.zoom, 1);
				const cdp =
					engine === "chromium" ? await context.newCDPSession(page) : undefined;
				if (cdp) await cdp.send("Performance.enable");
				const metricsBefore = cdp
					? await cdp.send("Performance.getMetrics")
					: undefined;
				if (cpu) {
					await cdp.send("Profiler.enable");
					await cdp.send("Profiler.setSamplingInterval", { interval: 1000 });
					await cdp.send("Profiler.start");
				}
				await page.evaluate((paused) => {
					window.__zoomProbe.pauseNative = paused;
					window.__zoomProbe.begin();
				}, pauseNative);
				const started = performance.now();
				const input = [];
				const idealDeltaY = Math.log2(1 / 0.02) / (0.002 * steps);
				const receivedDeltaY =
					engine === "webkit" ? Math.ceil(idealDeltaY) : idealDeltaY;
				const deltaY = receivedDeltaY * inputScale;
				for (let index = 0; index < steps; index++) {
					await page.mouse.wheel(0, deltaY);
					input.push({ at: performance.now() - started, deltaY });
					const pause =
						((index + 1) * duration) / steps - (performance.now() - started);
					if (pause > 0)
						await new Promise((resolve) => setTimeout(resolve, pause));
				}
				await page.waitForTimeout(300);
				const trace = await page.evaluate(() => window.__zoomProbe.finish());
				const profile = cpu
					? (await cdp.send("Profiler.stop")).profile
					: undefined;
				const metricsAfter = cdp
					? await cdp.send("Performance.getMetrics")
					: undefined;
				const after = await snapshot();
				assert.equal(after.graphNodes, before.graphNodes);
				assert.equal(after.graphEdges, before.graphEdges);
				assert.equal(
					await page
						.locator("[data-board-renderer]")
						.getAttribute("data-board-renderer"),
					renderer,
				);
				assert.deepEqual(errors, []);
				const sorted = trace.samples
					.map((sample) => sample.ms)
					.sort((a, b) => a - b);
				const metrics = metricsAfter
					? Object.fromEntries(
							metricsAfter.metrics.map(({ name, value }) => [
								name,
								value -
									(metricsBefore.metrics.find((metric) => metric.name === name)
										?.value ?? 0),
							]),
						)
					: undefined;
				const calls = {};
				for (const call of trace.nativeCalls) {
					calls[call.method] ??= {
						count: 0,
						totalMs: 0,
						maxMs: 0,
						characters: 0,
					};
					const row = calls[call.method];
					row.count++;
					row.totalMs += call.ms;
					row.maxMs = Math.max(row.maxMs, call.ms);
					row.characters += call.characters ?? 0;
				}
				const result = {
					name,
					renderer,
					minimap,
					before,
					after,
					input,
					wheelCalibration: {
						requested: 120,
						received: wheelCalibration.deltaY,
						inputScale,
						receivedDeltaY,
					},
					trace,
					calls,
					metrics,
					cpu: profile ? summarizeCpu(profile) : undefined,
					errors,
					assets: [...assets],
					frameSummary: {
						frames: sorted.length,
						medianMs: quantile(sorted, 0.5),
						p95Ms: quantile(sorted, 0.95),
						maxMs: sorted.at(-1),
						over50ms: sorted.filter((value) => value > 50).length,
					},
				};
				report.results.push(result);
				if (profile)
					await writeFile(
						`${output}/${name}.cpuprofile`,
						JSON.stringify(profile),
					);
				await page.screenshot({ path: `${output}/${name}-overview.png` });
				await save();
				console.log(
					JSON.stringify({
						name,
						...result.frameSummary,
						longTasks: trace.longTasks.length,
						nativeCalls: calls,
						dom: after.dom,
					}),
				);
				assert.ok(
					Math.abs(after.viewport.zoom - 0.02) < 0.000001,
					`Wheel endpoint must reach0.02; actual ${after.viewport.zoom}, received ${trace.wheelEvents.length}/${steps} wheel events`,
				);
			} finally {
				await context.close();
			}
		}
	report.finished = new Date().toISOString();
} catch (error) {
	report.failure = String(error);
	throw error;
} finally {
	await browser.close();
	await save();
	console.log("All zoom-profile browsers closed.");
}
