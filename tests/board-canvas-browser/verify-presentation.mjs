import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const origin = process.env.BOARD_CANVAS_URL ?? "http://127.0.0.1:4338";
const output =
	process.env.BOARD_CANVAS_PRESENTATION_OUTPUT ??
	`tmp/board-canvas-browser/presentation/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch({ headless: true })
		: await chromium.launch({
				headless: true,
				executablePath:
					process.env.CHROME_PATH ?? (await puppeteer.executablePath()),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const page = await browser.newPage({
	viewport: { width: 1600, height: 1100 },
	deviceScaleFactor: 2,
});
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
let release;
const delayed = new Promise((resolve) => {
	release = resolve;
});
await page.route("**/board-canvas/board_canvas.js", async (route) => {
	await delayed;
	await route.continue();
});
const results = {};
try {
	await page.goto(`${origin}/?boardRenderer=egui&nodes=10000`, {
		waitUntil: "domcontentloaded",
	});
	await page.locator("[data-board-canvas-loading]").waitFor();
	await page.waitForTimeout(500);
	results.loading = await page.evaluate(() => ({
		wrappers: document.querySelectorAll(".react-flow__node").length,
		empty: [...document.querySelectorAll(".react-flow__node")].filter(
			(n) => !n.firstElementChild,
		).length,
		renderer: document.querySelector("[data-board-renderer]")?.dataset
			.boardRenderer,
	}));
	assert.equal(results.loading.renderer, "loading");
	assert.equal(results.loading.empty, 0);
	assert.ok(
		results.loading.wrappers < 500,
		"Startup must not mount thousands of rich editors",
	);
	release();
	await page.waitForFunction(() => window.__boardCanvasHarness?.ready(), null, {
		timeout: 60000,
	});
	results.ready = await page.evaluate(() => ({
		presentation: window.__boardCanvasHarness.snapshot().presentation,
		wrappers: document.querySelectorAll(".react-flow__node").length,
		empty: [...document.querySelectorAll(".react-flow__node")].filter(
			(n) => !n.firstElementChild,
		).length,
		svgEdges: document.querySelectorAll(".react-flow__edge").length,
	}));
	assert.equal(
		await page.locator(".react-flow__renderer").count(),
		0,
		"10k WASM board must not mount a React Flow renderer",
	);
	assert.equal(await page.locator("[data-standalone-board]").count(), 1);
	assert.equal(results.ready.empty, 0);
	assert.ok(results.ready.presentation.nativeNodes > 9000);
	assert.ok(results.ready.wrappers < 500);
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_scene;
		window.__sceneReplacements = 0;
		module.Handle.prototype.set_scene = function (...args) {
			window.__sceneReplacements++;
			return original.apply(this, args);
		};
	});
	const minimap = page.locator(".react-flow__minimap-svg");
	const rect = await minimap.boundingBox();
	const before = await page.evaluate(
		() => window.__boardCanvasHarness.snapshot().viewport,
	);
	await page.mouse.move(rect.x + 100, rect.y + 75);
	await page.mouse.down();
	await page.mouse.move(rect.x + 115, rect.y + 85, { steps: 8 });
	await page.mouse.up();
	await page.waitForTimeout(150);
	const after = await page.evaluate(
		() => window.__boardCanvasHarness.snapshot().viewport,
	);
	assert.ok(
		Math.abs(before.x - after.x) > 1 || Math.abs(before.y - after.y) > 1,
		"Cached minimap pans the real viewport",
	);
	await page.mouse.wheel(0, -120);
	await page.waitForTimeout(300);
	const zoomed = await page.evaluate(
		() => window.__boardCanvasHarness.snapshot().viewport,
	);
	assert.notEqual(
		zoomed.zoom,
		after.zoom,
		"Cached minimap zoom remains interactive",
	);
	results.minimap = { before, after, zoomed };
	results.viewportSceneReplacements = await page.evaluate(
		() => window.__sceneReplacements,
	);
	assert.equal(
		results.viewportSceneReplacements,
		0,
		"Viewport changes must not rebuild or serialize the graph scene",
	);
	results.panSceneReplacements = {};
	for (const mode of ["middle", "space"]) {
		await page.mouse.move(0, 0);
		await page.evaluate(() =>
			window.__boardCanvasHarness.setViewport(20, -200, 0.5),
		);
		await page.locator("[data-board-canvas-input]").focus();
		await page.waitForTimeout(350);
		const point = await page.evaluate(() =>
			window.__boardCanvasHarness.node("extra-11"),
		);
		assert.equal(
			await page.locator('.react-flow__node[data-id="extra-11"]').count(),
			0,
			"The pan starts over a native node without a DOM editor",
		);
		await page.evaluate(() => {
			window.__sceneReplacements = 0;
		});
		if (mode === "space") await page.keyboard.down("Space");
		await page.mouse.move(point.x, point.y);
		await page.mouse.down({ button: mode === "middle" ? "middle" : "left" });
		for (let step = 1; step <= 10; step++) {
			await page.mouse.move(point.x + step * 10, point.y + step * 6);
			await page.waitForTimeout(25);
		}
		await page.waitForTimeout(250);
		results.panSceneReplacements[mode] = await page.evaluate(
			() => window.__sceneReplacements,
		);
		assert.equal(
			results.panSceneReplacements[mode],
			0,
			`${mode} pan must not promote, prune or reserialize the graph`,
		);
		await page.mouse.up({ button: mode === "middle" ? "middle" : "left" });
		if (mode === "space") await page.keyboard.up("Space");
		await page.mouse.move(0, 0);
		await page.waitForTimeout(300);
	}
	const hoverPoint = await page.evaluate(() =>
		window.__boardCanvasHarness.node("extra-11"),
	);
	await page.mouse.move(hoverPoint.x, hoverPoint.y);
	const hoverEditor = page.locator('.react-flow__node[data-id="extra-11"]');
	await hoverEditor.waitFor({ state: "visible" });
	await page.waitForTimeout(250);
	const beforeHeldHover = await page.evaluate(() => window.__sceneReplacements);
	await page.mouse.down({ button: "middle" });
	await page.mouse.move(hoverPoint.x + 100, hoverPoint.y + 60, { steps: 10 });
	await page.waitForTimeout(300);
	assert.equal(
		await hoverEditor.count(),
		1,
		"A pan retains an existing editor",
	);
	assert.equal(
		await page.evaluate(() => window.__sceneReplacements),
		beforeHeldHover,
		"An existing hover editor is not pruned during pan",
	);
	await page.mouse.up({ button: "middle" });
	await page.mouse.move(0, 0);
	await hoverEditor.waitFor({ state: "detached" });
	const restoredHover = await page.evaluate(() =>
		window.__boardCanvasHarness.node("extra-11"),
	);
	await page.mouse.move(restoredHover.x, restoredHover.y);
	await hoverEditor.waitFor({ state: "visible" });
	results.hoverRestoredAfterPan = true;
	await page.mouse.move(0, 0);
	await hoverEditor.waitFor({ state: "detached" });
	await page.evaluate(() =>
		window.__boardCanvasHarness.setViewport(200, -600, 1),
	);
	await page.waitForTimeout(350);
	const wheelPoint = await page.evaluate(() =>
		window.__boardCanvasHarness.node("extra-11"),
	);
	const beforeWheel = await page.evaluate(() => {
		window.__sceneReplacements = 0;
		window.__idleHoverQueries = 0;
		const elementFromPoint = document.elementFromPoint;
		document.elementFromPoint = function (...args) {
			window.__idleHoverQueries++;
			return elementFromPoint.apply(this, args);
		};
		return window.__boardCanvasHarness.snapshot().viewport;
	});
	assert.equal(await hoverEditor.count(), 0);
	await page.mouse.move(wheelPoint.x, wheelPoint.y);
	for (let step = 0; step < 12; step++) {
		await page.mouse.wheel(0, 24);
		await page.waitForTimeout(25);
		assert.equal(
			await page.evaluate(() => window.__sceneReplacements),
			0,
			"Wheel zoom over a native node must not promote or serialize the scene",
		);
		assert.equal(
			await hoverEditor.count(),
			0,
			"Native hover promotion stays paused throughout wheel zoom",
		);
	}
	const duringWheel = await page.evaluate(() => ({
		viewport: window.__boardCanvasHarness.snapshot().viewport,
		sceneReplacements: window.__sceneReplacements,
		hoverQueries: window.__idleHoverQueries,
	}));
	results.wheelHover = { before: beforeWheel, during: duringWheel };
	assert.ok(duringWheel.viewport.zoom < beforeWheel.zoom);
	assert.ok(
		// The initial pointer move queries once in the host and in each WASM runner.
		duringWheel.hoverQueries <= 3,
		"Stationary-pointer zoom must defer DOM hit queries instead of reading after every camera write",
	);
	await hoverEditor.waitFor({ state: "visible" });
	assert.ok(
		await page.evaluate(
			(queries) => window.__idleHoverQueries > queries,
			duringWheel.hoverQueries,
		),
		"Idle hover must query the new camera position after zoom settles",
	);
	assert.ok(
		await page.evaluate(() => window.__sceneReplacements > 0),
		"Normal hover promotion resumes after the wheel gesture ends",
	);
	results.wheelHover = {
		before: beforeWheel,
		during: duringWheel,
		hoverRestored: true,
	};
	await page.mouse.move(0, 0);
	await hoverEditor.waitFor({ state: "detached" });
	await page.evaluate(async () => {
		window.__boardCanvasHarness.setReadOnly(true);
		await window.__boardCanvasHarness.setViewport(200, -600, 1);
	});
	const lockedHoverPoint = await page.evaluate(() =>
		window.__boardCanvasHarness.node("extra-11"),
	);
	await page.evaluate(() =>
		window.__boardCanvasHarness.setViewport(650, -600, 1),
	);
	await page.mouse.move(lockedHoverPoint.x, lockedHoverPoint.y);
	await page.waitForTimeout(400);
	assert.equal(await hoverEditor.count(), 0);
	await page.evaluate(() =>
		window.__boardCanvasHarness.setViewport(200, -600, 1),
	);
	await hoverEditor.waitFor({ state: "visible" });
	assert.equal(
		await page.evaluate(() => window.__boardCanvasHarness.snapshot().readOnly),
		true,
	);
	results.readOnlyHoverRestoredAfterCamera = true;
	await page.mouse.move(0, 0);
	await hoverEditor.waitFor({ state: "detached" });
	await page.evaluate(() => window.__boardCanvasHarness.setReadOnly(false));
	assert.deepEqual(errors, []);
	await page.screenshot({ path: `${output}/ready.png` });
	const fallback = await browser.newPage();
	await fallback.route("**/board-canvas/board_canvas.js", (route) =>
		route.abort(),
	);
	await fallback.goto(`${origin}/?boardRenderer=egui`, {
		waitUntil: "domcontentloaded",
	});
	await fallback.waitForFunction(
		() =>
			document.querySelector("[data-board-renderer]")?.dataset.boardRenderer ===
			"react-flow",
	);
	assert.ok(
		await fallback
			.locator('[data-board-renderer-status="fallback"]')
			.isVisible(),
		"Load failure explains the visible React Flow fallback",
	);
	assert.ok(
		(await fallback.locator(".react-flow__node").count()) > 0,
		"Fallback restores original editor nodes",
	);
	results.loadFailureFallback = true;
	await fallback.close();
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, ...results, errors }, null, 2),
	);
	console.log(JSON.stringify({ kind, ...results, errors }));
} catch (error) {
	results.failureState = await page.evaluate(() => ({
		focus: document.activeElement?.outerHTML.slice(0, 1000),
		portals: [
			...document.querySelectorAll(
				'[role="dialog"][data-state="open"], [role="menu"][data-state="open"], [data-radix-popper-content-wrapper]',
			),
		].map((element) => element.outerHTML.slice(0, 1000)),
		hoveredNode: document
			.querySelector(".react-flow__node:hover")
			?.getAttribute("data-id"),
		viewport: window.__boardCanvasHarness?.snapshot().viewport,
	}));
	console.log(JSON.stringify(results));
	await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
	throw error;
} finally {
	release();
	await browser.close();
}
