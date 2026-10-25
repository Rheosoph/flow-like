import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import { setFixtureRenderer } from "../board-canvas-browser/renderer-settings-driver.mjs";

const origin = process.env.BOARD_PARITY_URL ?? "http://127.0.0.1:4339";
const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
assert.ok(["chromium", "webkit"].includes(kind), "Use chromium or webkit");
const renderers = process.env.BOARD_LARGE_RENDERER
	? [process.env.BOARD_LARGE_RENDERER]
	: ["react-flow", "egui"];
assert.ok(
	renderers.every((renderer) => ["react-flow", "egui"].includes(renderer)),
	"Use react-flow or egui",
);
const testToggles = process.env.BOARD_LARGE_SKIP_TOGGLE !== "1";
const output = resolve(
	process.env.BOARD_LARGE_OUTPUT ??
		`tmp/board-parity-browser/large-smoke/${kind}`,
);
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch({ headless: true })
		: await chromium.launch({
				headless: true,
				executablePath: process.env.CHROME_PATH ?? puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const results = [];
const DOM_NODE_BUDGET = 400;

async function snapshot(page) {
	return page.evaluate(() => {
		const state = window.__boardParity.snapshot();
		const root = document.querySelector(".react-flow").getBoundingClientRect();
		return {
			domainNodes: Object.keys(state.board.nodes).length,
			presentation: state.presentation,
			renderer: document.querySelector("[data-board-renderer]")?.dataset
				.boardRenderer,
			bounds: state.bounds,
			visibleNodes: state.visibleNodes,
			viewport: state.viewport,
			size: { width: root.width, height: root.height },
			mountedWrappers: document.querySelectorAll(".react-flow__node").length,
			emptyProxyWrappers: [
				...document.querySelectorAll(".react-flow__node"),
			].filter((node) => !node.firstElementChild).length,
			canvases: [
				...document.querySelectorAll('[data-testid^="egui-board"]'),
			].map((canvas) => ({
				id: canvas.dataset.testid,
				width: canvas.width,
				height: canvas.height,
				css: {
					width: canvas.getBoundingClientRect().width,
					height: canvas.getBoundingClientRect().height,
				},
			})),
		};
	});
}

/** egui fits the whole board. React Flow stops where a screen holds at most its rich-node budget. */
function assertFits(state) {
	const { bounds, viewport, size } = state;
	const screen = {
		left: viewport.x + bounds.x * viewport.zoom,
		top: viewport.y + bounds.y * viewport.zoom,
		right: viewport.x + (bounds.x + bounds.width) * viewport.zoom,
		bottom: viewport.y + (bounds.y + bounds.height) * viewport.zoom,
	};
	if (state.renderer === "react-flow") {
		assert.ok(
			viewport.zoom > 0.1,
			`React Flow's density floor stops a dense fit above 0.1: ${viewport.zoom}`,
		);
		assert.ok(
			state.mountedWrappers <= DOM_NODE_BUDGET,
			`React Flow mounts at most ${DOM_NODE_BUDGET} rich nodes: ${state.mountedWrappers}`,
		);
		return screen;
	}
	assert.ok(
		viewport.zoom < 0.1,
		"Large fit must zoom below the previous limit",
	);
	assert.ok(
		screen.left >= -2 &&
			screen.top >= -2 &&
			screen.right <= size.width + 2 &&
			screen.bottom <= size.height + 2,
		`Fit view must contain every nonhidden item: ${JSON.stringify({ screen, size, viewport })}`,
	);
	assert.equal(
		state.emptyProxyWrappers,
		0,
		"Native graph items must not mount empty DOM wrappers",
	);
	assert.ok(
		state.mountedWrappers <= state.presentation.domNodes,
		"Only DOM-owned nodes mount wrappers",
	);
	assert.ok(
		state.visibleNodes > 9900,
		"The full graph remains available for fit and selection",
	);
	return screen;
}

/**
 * A renderer switch keeps the camera. Below React Flow's density floor it keeps the board
 * point at the pane center and raises the zoom to that floor.
 */
function assertHandover(from, to, size, renderer) {
	if (Math.abs(to.zoom - from.zoom) < 0.0001) {
		assert.ok(
			Math.abs(to.x - from.x) < 0.1 && Math.abs(to.y - from.y) < 0.1,
			"Renderer toggle preserves the viewport",
		);
		return;
	}
	assert.ok(
		renderer === "react-flow" && to.zoom > from.zoom,
		`Only React Flow's density floor changes the zoom on a switch: ${JSON.stringify({ from, to, renderer })}`,
	);
	// Pane sizes are whole pixels, so each half-pixel axis drifts by up to (ratio - 1) / 4 px.
	const tolerance = (Math.SQRT2 * (to.zoom / from.zoom - 1)) / 4 + 0.5;
	const point = [
		(size.width / 2 - from.x) / from.zoom,
		(size.height / 2 - from.y) / from.zoom,
	];
	const shift = Math.hypot(
		point[0] * to.zoom + to.x - size.width / 2,
		point[1] * to.zoom + to.y - size.height / 2,
	);
	assert.ok(
		shift <= tolerance,
		`Renderer toggle keeps the board point at the pane center: ${shift}px ${JSON.stringify({ from, to })}`,
	);
}

async function waitRenderer(page, renderer) {
	await page.waitForFunction(
		(expected) =>
			window.__boardParity &&
			document.querySelector("[data-board-renderer]")?.dataset.boardRenderer ===
				expected,
		renderer,
		{ timeout: 120000 },
	);
}

try {
	for (const renderer of renderers) {
		const page = await browser.newPage({
			viewport: { width: 1600, height: 1100 },
			deviceScaleFactor: 2,
		});
		page.setDefaultTimeout(120000);
		const errors = [];
		const assets = new Set();
		page.on("pageerror", (error) => errors.push(error.message));
		page.on("response", (response) => {
			if (/\.(?:js|wasm)(?:\?|$)/.test(response.url()))
				assets.add(response.url());
		});
		try {
			const url = new URL(origin);
			url.searchParams.set("boardRenderer", renderer);
			url.searchParams.set("nodes", "10000");
			await page.goto(url.href, { waitUntil: "domcontentloaded" });
			await waitRenderer(page, renderer);
			await page.evaluate(() => document.fonts.ready);
			await page.waitForTimeout(1000);
			const fit = page.locator(".react-flow__controls-fitview");
			assert.ok(await fit.isVisible(), "Fit view is accessible");
			assert.ok(
				await page.locator(".react-flow__minimap").isVisible(),
				"Minimap is visible",
			);
			await fit.click();
			await page.waitForTimeout(700);
			const overview = await snapshot(page);
			assert.equal(overview.domainNodes, 10000);
			const screenBounds = assertFits(overview);
			if (renderer === "egui") {
				assert.equal(overview.canvases.length, 2);
				assert.ok(overview.presentation.nativeNodes > 9000);
			}
			await page.screenshot({ path: `${output}/${renderer}-overview.png` });
			const zoomOut = page.locator(".react-flow__controls-zoomout");
			for (let step = 0; step < 20 && (await zoomOut.isEnabled()); step++) {
				await zoomOut.click();
				await page.waitForTimeout(250);
			}
			const lowerLimit = await page.evaluate(
				() => window.__boardParity.snapshot().viewport,
			);
			assert.equal(
				lowerLimit.zoom,
				renderer === "egui" ? 0.02 : overview.viewport.zoom,
				"User zoom clamps to the renderer's lower zoom limit",
			);
			assert.ok(
				await zoomOut.isDisabled(),
				"Zoom out is disabled at the lower limit",
			);
			await fit.click();
			await page.waitForTimeout(700);
			assertFits(await snapshot(page));
			await page.mouse.move(810, 600);
			await page.mouse.down({ button: "middle" });
			await page.mouse.move(930, 660, { steps: 16 });
			await page.mouse.up({ button: "middle" });
			await page.mouse.move(20, 20);
			await page.waitForTimeout(300);
			const after = await page.evaluate(
				() => window.__boardParity.snapshot().viewport,
			);
			assert.ok(
				Math.abs(after.x - overview.viewport.x - 120) < 1,
				"Middle pan moves 120 CSS pixels horizontally",
			);
			assert.ok(
				Math.abs(after.y - overview.viewport.y - 60) < 1,
				"Middle pan moves 60 CSS pixels vertically",
			);
			await page.screenshot({ path: `${output}/${renderer}-panned.png` });
			await page.evaluate(() => window.__boardParity.runtime(true));
			await page.waitForTimeout(300);
			const runtime = await page.evaluate(() => {
				const state = window.__boardParity.snapshot();
				return { runs: state.runs, presentation: state.presentation };
			});
			assert.ok(runtime.runs.includes("fixture-run"));
			await page.evaluate(() => window.__boardParity.runtime(false));
			assert.equal(
				await page
					.locator("[data-board-renderer]")
					.getAttribute("data-board-renderer"),
				renderer,
				"Renderer remains active after zoom, pan and runtime updates",
			);
			let previous = after;
			for (const next of testToggles
				? [renderer === "egui" ? "react-flow" : "egui", renderer]
				: []) {
				await setFixtureRenderer(page, next);
				await waitRenderer(page, next);
				const toggled = await page.evaluate(
					() => window.__boardParity.snapshot().viewport,
				);
				assertHandover(previous, toggled, overview.size, next);
				previous = toggled;
			}
			assert.deepEqual(errors, []);
			results.push({
				browser: kind,
				fixture:
					"Production components with simulated backend, Next development server",
				renderer,
				...overview,
				screenBounds,
				lowerLimit,
				after,
				runtime,
				fitView: true,
				minimap: true,
				togglePreservesViewport: testToggles ? true : null,
				errors,
				assets: [...assets],
			});
			console.log(
				JSON.stringify({
					renderer,
					domainNodes: overview.domainNodes,
					presentation: overview.presentation,
					visibleNodes: overview.visibleNodes,
					viewport: overview.viewport,
					screenBounds,
					lowerLimit,
					errors,
				}),
			);
		} catch (error) {
			await page
				.screenshot({ path: `${output}/${renderer}-failure.png` })
				.catch(() => {});
			throw error;
		} finally {
			await page.close();
		}
	}
	await writeFile(`${output}/results.json`, JSON.stringify(results, null, 2));
} finally {
	await browser.close();
}
