import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import { setFixtureRenderer } from "./renderer-settings-driver.mjs";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const output = `tmp/board-canvas-browser/renderer-panel/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				executablePath: await puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const fixtures = [
	{
		name: "canvas",
		url: "http://127.0.0.1:4338/",
		api: "__boardCanvasHarness",
		viewport: "setViewport",
	},
	{
		name: "actual-host",
		url: "http://127.0.0.1:4339/host",
		api: "__boardHost",
		viewport: "viewport",
	},
];
const results = [];
const errors = [];
const waitRenderer = (page, renderer) =>
	page.waitForFunction(
		(expected) =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === expected,
		renderer,
	);
const snapshot = (page, api) =>
	page.evaluate((key) => {
		const value = window[key].snapshot();
		return {
			board: value.board ?? value.nodes,
			viewport: value.viewport,
			selectedNodes:
				value.selectedNodes ??
				value.nodes.filter((node) => node.selected).map((node) => node.id),
			selectedEdges:
				value.selectedEdges ??
				value.edges.filter((edge) => edge.selected).map((edge) => edge.id),
			history: value.history,
			commits: value.commits ?? value.transactions.length,
		};
	}, api);

try {
	for (const fixture of fixtures) {
		const context = await browser.newContext({
			viewport: { width: 1600, height: 1000 },
		});
		try {
			await context.route("**/*", async (route) => {
				const url = new URL(route.request().url());
				if (
					!["127.0.0.1", "localhost"].includes(url.hostname) &&
					!["data:", "blob:"].includes(url.protocol)
				)
					return route.abort();
				await route.continue();
			});
			const page = await context.newPage();
			page.setDefaultTimeout(15_000);
			page.on("pageerror", (error) =>
				errors.push({ fixture: fixture.name, message: error.message }),
			);
			await page.goto(fixture.url, { waitUntil: "domcontentloaded" });
			await waitRenderer(page, "egui");
			await page.waitForFunction((key) => !!window[key]?.snapshot, fixture.api);
			await page.waitForLoadState("networkidle");
			await page.evaluate(
				({ api, viewport }) => window[api][viewport](55, -20, 0.8),
				fixture,
			);
			const before = await snapshot(page, fixture.api);
			const status = page.locator("[data-board-renderer-status]");
			const trigger = status.locator("button").first();
			const panel = page.locator("[data-board-renderer-panel]");
			await trigger.hover();
			await panel.waitFor({ state: "visible" });
			assert.equal(
				await trigger.evaluate((element) => document.activeElement === element),
				false,
				"Hover does not steal keyboard focus",
			);
			assert.match(await panel.innerText(), /WASM/);
			assert.match(
				await panel.innerText(),
				/WebAssembly canvas draws nodes and connections/,
			);
			assert.match(
				await panel.innerText(),
				/edge glow is a visual effect; execution is shown on the nodes/,
			);
			await panel.hover();
			await page.waitForTimeout(300);
			assert.equal(
				await panel.isVisible(),
				true,
				"Moving from badge into the hover panel keeps actions reachable",
			);
			await page.mouse.move(5, 5);
			await panel.waitFor({ state: "hidden" });
			await trigger.click();
			await panel.waitFor({ state: "visible" });
			await page.mouse.move(5, 5);
			await page.waitForTimeout(350);
			assert.equal(
				await panel.isVisible(),
				true,
				"Click keeps the panel open after pointer exit",
			);
			await page.keyboard.press("Escape");
			await panel.waitFor({ state: "hidden" });
			assert.equal(
				await trigger.evaluate((element) => document.activeElement === element),
				true,
				"Escape restores focus to the status trigger",
			);
			await trigger.focus();
			await page.keyboard.press("Enter");
			await panel.waitFor({ state: "visible" });
			await page.keyboard.press("Tab");
			assert.equal(
				await panel.evaluate((element) =>
					element.contains(document.activeElement),
				),
				true,
				"Keyboard navigation reaches panel actions",
			);
			await page.keyboard.press("Shift+Tab");
			assert.equal(
				await trigger.evaluate((element) => document.activeElement === element),
				true,
				"Reverse keyboard navigation returns to the status trigger",
			);
			await page.keyboard.press("Tab");
			await page.keyboard.press("Tab");
			await panel.waitFor({ state: "hidden" });
			assert.equal(
				await page.evaluate(() =>
					Boolean(
						document.activeElement?.closest("[data-board-renderer-panel]"),
					),
				),
				false,
				"The non-modal panel lets Tab continue through the page",
			);
			await trigger.focus();
			await page.keyboard.press("Enter");
			await panel.waitFor({ state: "visible" });
			await page.keyboard.press("Escape");
			await panel.waitFor({ state: "hidden" });
			await trigger.click();
			await panel
				.getByRole("button", { name: "Switch to React Flow", exact: true })
				.click();
			await waitRenderer(page, "react-flow");
			assert.equal(
				await status.getAttribute("data-board-renderer-status"),
				"react-flow",
			);
			assert.equal(
				await page.evaluate(() => localStorage.getItem("flow-board-renderer")),
				"react-flow",
			);
			assert.deepEqual(
				await snapshot(page, fixture.api),
				before,
				"Direct opt-out preserves graph, selection, history and viewport",
			);
			await trigger.click();
			await panel.waitFor({ state: "visible" });
			assert.equal(
				await panel
					.getByRole("link", { name: "Renderer settings", exact: true })
					.getAttribute("href"),
				"/settings#board-renderer-card",
			);
			await page.keyboard.press("Escape");
			await setFixtureRenderer(page, "egui");
			assert.deepEqual(
				await snapshot(page, fixture.api),
				before,
				"Settings can re-enable native rendering without changing board state",
			);
			await trigger.hover();
			await panel.waitFor({ state: "visible" });
			await page.waitForTimeout(250);
			await page.screenshot({ path: `${output}/${fixture.name}.png` });
			results.push({
				fixture: fixture.name,
				checks: [
					"hover opening without focus theft",
					"hover actions reachable",
					"click pins panel open",
					"Escape and keyboard focus",
					"Tab enters and exits the non-modal panel",
					"direct opt-out updates preference and actual renderer",
					"board/selection/history/viewport preserved",
					"settings link",
					"settings re-enable",
				],
			});
			console.log(
				`PASS ${fixture.name}: status panel interactions and renderer escape action`,
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
		`Browser closed; ${results.length}/${fixtures.length} panel cases completed, ${errors.length} page errors.`,
	);
}
