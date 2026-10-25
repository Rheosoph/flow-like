import assert from "node:assert/strict";
import { chromium } from "playwright";
import puppeteer from "puppeteer";

const origin = process.env.BOARD_PARITY_URL ?? "http://127.0.0.1:4339";
const browser = await chromium.launch({
	headless: true,
	executablePath: process.env.CHROME_PATH ?? puppeteer.executablePath(),
	args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
});

try {
	for (const renderer of ["egui", "react-flow"]) {
		const page = await browser.newPage({
			viewport: { width: 1600, height: 1000 },
		});
		page.setDefaultTimeout(30_000);
		const errors = [];
		page.on("pageerror", (error) => errors.push(error.message));
		await page.route("**/*", (route) => {
			const url = new URL(route.request().url());
			return ["127.0.0.1", "localhost"].includes(url.hostname) ||
				["data:", "blob:"].includes(url.protocol)
				? route.continue()
				: route.abort();
		});
		try {
			await page.goto(`${origin}/?boardRenderer=${renderer}&nodes=1634`, {
				waitUntil: "domcontentloaded",
				timeout: 180_000,
			});
			await page.waitForFunction(
				(expected) =>
					window.__boardParity &&
					document.querySelector("[data-board-renderer]")?.dataset
						.boardRenderer === expected,
				renderer,
				{ timeout: 120_000 },
			);
			const navigation = page.locator("[data-large-graph-navigation]");
			await navigation.waitFor();
			// The fixture restores its detailed camera shortly after onInit.
			await page.waitForTimeout(500);
			await page.locator(".react-flow__controls-fitview").click();
			await page.waitForFunction(
				() => window.__boardParity.snapshot().viewport.zoom < 1,
			);
			const initial = await page.evaluate(() =>
				window.__boardParity.snapshot(),
			);
			assert.ok(initial.visibleNodes >= 1600);
			assert.match(await navigation.innerText(), /1,6\d\d nodes/);
			assert.equal(
				await page.locator("[data-board-canvas-loading]").count(),
				0,
			);
			const preference = await page.evaluate(() =>
				localStorage.getItem("flow-board-renderer"),
			);
			await page.screenshot({
				path: `/tmp/large-graph-overview-${renderer}.png`,
			});

			for (const [query, id] of [
				["Native node", "native"],
				["string-a", "string-a"],
			]) {
				await navigation.getByRole("button", { name: "Browse nodes" }).click();
				const search = page.getByRole("textbox", { name: "Search nodes" });
				await search.waitFor();
				assert.ok((await page.locator('[role="dialog"] li').count()) <= 25);
				await search.fill(query);
				if (id === "native") {
					await page.waitForFunction(() => {
						const dialog = document.querySelector('[role="dialog"]');
						return dialog && getComputedStyle(dialog).opacity === "1";
					});
					await page.screenshot({
						path: `/tmp/large-graph-search-${renderer}.png`,
					});
				}
				const result = page.locator('[role="dialog"] li button').filter({
					has: page.locator("span", { hasText: new RegExp(`^${id}$`) }),
				});
				await result.click();
				await page.waitForFunction(
					() => window.__boardParity.snapshot().viewport.zoom === 1,
				);
				await search.waitFor({ state: "hidden" });
				const focused = await page.evaluate((nodeId) => {
					const state = window.__boardParity.snapshot();
					const bounds = window.__boardParity.nodeBounds(nodeId);
					const pane = document
						.querySelector(".react-flow")
						.getBoundingClientRect();
					return {
						viewport: state.viewport,
						bounds,
						width: pane.width,
						height: pane.height,
					};
				}, id);
				assert.ok(
					Math.abs(
						focused.viewport.x +
							focused.bounds.x +
							focused.bounds.width / 2 -
							focused.width / 2,
					) < 1,
					"The chosen node is centered horizontally",
				);
				assert.ok(
					Math.abs(
						focused.viewport.y +
							focused.bounds.y +
							focused.bounds.height / 2 -
							focused.height / 2,
					) < 1,
					"The chosen node is centered vertically",
				);
				if (id === "native")
					await page.screenshot({
						path: `/tmp/large-graph-focused-${renderer}.png`,
					});
				await navigation.getByRole("button", { name: "Fit graph" }).click();
				await page.waitForFunction(
					(zoom) =>
						Math.abs(window.__boardParity.snapshot().viewport.zoom - zoom) <
						0.00001,
					initial.viewport.zoom,
				);
			}
			const after = await page.evaluate(() => window.__boardParity.snapshot());
			assert.deepEqual(after.board, initial.board);
			assert.deepEqual(after.commands, initial.commands);
			assert.equal(
				await page.evaluate(() => localStorage.getItem("flow-board-renderer")),
				preference,
			);
			assert.equal(
				await page
					.locator("[data-board-renderer]")
					.getAttribute("data-board-renderer"),
				renderer,
			);
			assert.deepEqual(errors, []);
			await page.screenshot({
				path: `/tmp/large-graph-navigation-${renderer}.png`,
			});
			console.log(
				`PASS ${renderer}: 1634-node name/ID search, focus at 100%, fit graph, unchanged graph and renderer preference`,
			);
		} finally {
			await page.close();
		}
	}
} finally {
	await browser.close();
}
