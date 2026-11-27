import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import { setFixtureRenderer } from "./renderer-settings-driver.mjs";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
assert.ok(["chromium", "webkit"].includes(kind));
const output = `tmp/board-canvas-browser/renderer-settings/${kind}`;
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
const fixtures = [
	{
		name: "canvas",
		url: "http://127.0.0.1:4338/",
		api: "__boardCanvasHarness",
	},
	{
		name: "production-widgets",
		url: "http://127.0.0.1:4339/",
		api: "__boardParity",
	},
	{
		name: "actual-host",
		url: "http://127.0.0.1:4339/host",
		api: "__boardHost",
	},
].filter(
	(fixture) =>
		!process.env.BOARD_SETTINGS_FIXTURE ||
		fixture.name === process.env.BOARD_SETTINGS_FIXTURE,
);
const renderer = (page) => page.locator("[data-board-renderer]");
const waitRenderer = async (page, expected) => {
	await page.waitForFunction(
		(value) =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === value,
		expected,
	);
	assert.equal(
		await page.getByRole("combobox", { name: "Canvas renderer" }).count(),
		0,
	);
	assert.equal(
		await renderer(page)
			.getByRole("switch", { name: "Use WASM board (preview)" })
			.count(),
		0,
	);
};
const snapshot = (page, api) =>
	page.evaluate((key) => {
		const value = window[key].snapshot();
		return {
			graph: value.board ?? value.nodes,
			viewport: value.viewport,
		};
	}, api);

try {
	for (const fixture of fixtures) {
		const context = await browser.newContext({
			viewport: { width: 1600, height: 1000 },
		});
		try {
			let phase = "default";
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
			page.setDefaultTimeout(20_000);
			page.on("pageerror", (error) =>
				errors.push({
					fixture: fixture.name,
					phase,
					message: error.message,
					stack: error.stack,
				}),
			);
			await page.goto(fixture.url, { waitUntil: "domcontentloaded" });
			await waitRenderer(page, "egui");
			await page.locator('[data-board-renderer-status="active"]').waitFor();
			await page.waitForFunction((key) => !!window[key]?.snapshot, fixture.api);
			await page.waitForTimeout(500);
			assert.equal(
				await page.evaluate(() => localStorage.getItem("flow-board-renderer")),
				null,
			);
			const before = await snapshot(page, fixture.api);
			phase = "disable-default";
			await setFixtureRenderer(page, "react-flow");
			await waitRenderer(page, "react-flow");
			await page.locator('[data-board-renderer-status="react-flow"]').waitFor();
			phase = "enable";
			await setFixtureRenderer(page, "egui");
			await waitRenderer(page, "egui");
			assert.deepEqual(
				await snapshot(page, fixture.api),
				before,
				"Settings toggle preserves graph and viewport",
			);
			assert.equal(
				await page.evaluate(() => localStorage.getItem("flow-board-renderer")),
				"egui",
			);
			phase = "reload-enabled";
			await page.reload({ waitUntil: "domcontentloaded" });
			await waitRenderer(page, "egui");
			phase = "second-tab";
			const second = await context.newPage();
			second.setDefaultTimeout(20_000);
			second.on("pageerror", (error) =>
				errors.push({
					fixture: fixture.name,
					phase,
					message: error.message,
					stack: error.stack,
				}),
			);
			await second.goto(fixture.url, { waitUntil: "domcontentloaded" });
			await waitRenderer(second, "egui");
			phase = "disable";
			await setFixtureRenderer(second, "react-flow");
			await waitRenderer(second, "react-flow");
			await waitRenderer(page, "react-flow");
			await second.close();
			phase = "reload-disabled";
			await page.reload({ waitUntil: "domcontentloaded" });
			await waitRenderer(page, "react-flow");
			await page.waitForFunction((key) => !!window[key]?.snapshot, fixture.api);
			// Let the host finish loading and saving its initial board before the next navigation.
			await page.waitForLoadState("networkidle");
			phase = "query-initializer";
			await page.goto(`${fixture.url}?boardRenderer=egui`, {
				waitUntil: "domcontentloaded",
			});
			await waitRenderer(page, "egui");
			await page
				.getByTestId("fixture-renderer-settings")
				.locator("summary")
				.click();
			assert.equal(
				await page
					.getByRole("switch", { name: "Use WASM board (preview)" })
					.getAttribute("aria-checked"),
				"true",
			);
			await page.screenshot({ path: `${output}/${fixture.name}.png` });
			results.push({
				fixture: fixture.name,
				checks: [
					"WASM default with explicit React Flow opt-out",
					"settings-only switch",
					"live toggle preserves graph and viewport",
					"persist enabled on reload",
					"shared preference across tabs",
					"persist disabled on reload",
					"test-only query initializer",
				],
			});
			console.log(
				`PASS ${fixture.name}: default, settings toggle, persistence, tab sync, fixture query`,
			);
		} finally {
			await context.close();
		}
	}
	const failureContext = await browser.newContext({
		viewport: { width: 1600, height: 1000 },
	});
	try {
		await failureContext.addInitScript(() => {
			window.__fixtureFailCanvas = true;
			localStorage.setItem("flow-board-renderer", "egui");
		});
		await failureContext.route("**/board-canvas/board_canvas.js", (route) =>
			route.fulfill({
				contentType: "text/javascript",
				body: `export { default } from "/board-canvas/board_canvas.js?settingsRetry=true";
import { start as startCanvas } from "/board-canvas/board_canvas.js?settingsRetry=true";
export async function start(...args) {
  if (window.__fixtureFailCanvas) throw new Error("Fixture canvas startup rejection");
  return startCanvas(...args);
}`,
			}),
		);
		const page = await failureContext.newPage();
		page.setDefaultTimeout(20_000);
		page.on("pageerror", (error) =>
			errors.push({ fixture: "failure-retry", message: error.message }),
		);
		await page.goto("http://127.0.0.1:4338/", {
			waitUntil: "domcontentloaded",
		});
		await waitRenderer(page, "react-flow");
		const settings = page.getByTestId("fixture-renderer-settings");
		await settings.locator("summary").click();
		await page.locator('[data-board-renderer-status="fallback"]').waitFor();
		await waitRenderer(page, "react-flow");
		assert.equal(
			await page.evaluate(() => localStorage.getItem("flow-board-renderer")),
			"egui",
		);
		assert.equal(
			await settings
				.getByRole("switch", { name: "Use WASM board (preview)" })
				.getAttribute("aria-checked"),
			"true",
		);
		await page.evaluate(() => {
			window.__fixtureFailCanvas = false;
		});
		await settings.locator("summary").click();
		await page
			.locator('[data-board-renderer-status="fallback"] button')
			.click();
		await page.getByRole("button", { name: "Retry WASM" }).click();
		await waitRenderer(page, "egui");
		await page.locator('[data-board-renderer-status="active"]').waitFor();
		results.push({
			fixture: "failure-retry",
			checks: [
				"fallback leaves saved opt-in unchanged",
				"fallback badge retries native startup directly",
			],
		});
		console.log(
			"PASS failure-retry: saved opt-in survives fallback and Retry WASM restores the canvas",
		);
	} finally {
		await failureContext.close();
	}
	assert.deepEqual(errors, []);
} finally {
	await browser.close();
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ browser: kind, results, errors }, null, 2),
	);
	console.log(
		`Browser closed; ${results.length}/${fixtures.length + 1} functional settings groups completed; ${errors.length} page errors.`,
	);
}
