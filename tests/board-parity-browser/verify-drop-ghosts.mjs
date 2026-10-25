import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output = `tmp/board-parity-browser/drop-ghosts/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch()
		: await chromium.launch({
				headless: true,
				executablePath: puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const page = await browser.newPage({
	viewport: { width: 1600, height: 1000 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(15000);
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", async (route) => {
	const url = new URL(route.request().url());
	if (
		!["127.0.0.1", "localhost"].includes(url.hostname) &&
		!["data:", "blob:"].includes(url.protocol)
	)
		return route.abort();
	await route.continue();
});

const snapshot = () => page.evaluate(() => window.__boardHost.snapshot());
const waitFor = (predicate, arg) => page.waitForFunction(predicate, arg);
const ghosts = page.locator("[data-drop-ghost]");
const ghostCount = (count) =>
	waitFor(
		(count) => document.querySelectorAll("[data-drop-ghost]").length === count,
		count,
	);
const center = async (locator) => {
	const box = await locator.boundingBox();
	assert.ok(box, "The drag source must be visible");
	return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
};
/** Flow coordinates under a client point, from the camera the host reports. */
const flowPoint = async (point) => {
	const { viewport } = await snapshot();
	const root = await page.locator(".react-flow").boundingBox();
	assert.ok(root);
	return {
		x: (point.x - root.x - viewport.x) / viewport.zoom,
		y: (point.y - root.y - viewport.y) / viewport.zoom,
	};
};
const startDrag = async (source) => {
	const from = await center(source);
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(from.x + 24, from.y, { steps: 4 });
	return from;
};
const assertGhostAt = async (point, label) => {
	const box = await ghosts.first().boundingBox();
	assert.ok(box, `${label}: the ghost must be visible`);
	assert.ok(
		Math.abs(box.x - point.x) <= 1 && Math.abs(box.y - point.y) <= 1,
		`${label}: the ghost's corner sits under the pointer, got ${box.x},${box.y} for ${point.x},${point.y}`,
	);
};
const placedNodes = async (name) =>
	Object.values((await snapshot()).board.nodes).filter(
		(node) => node.name === name,
	);
const assertPlacedAt = async (name, point, label) => {
	await waitFor(
		(name) =>
			Object.values(window.__boardHost.snapshot().board.nodes).some(
				(node) => node.name === name,
			),
		name,
	);
	const [placed] = await placedNodes(name);
	const expected = await flowPoint(point);
	assert.ok(
		Math.abs(placed.coordinates[0] - expected.x) <= 1 &&
			Math.abs(placed.coordinates[1] - expected.y) <= 1,
		`${label}: the node lands where its ghost was, got ${placed.coordinates} for ${expected.x},${expected.y}`,
	);
};
const results = [];
const test = async (name, run) => {
	await run();
	results.push(name);
	console.log("PASS", name);
};

const DROP = { x: 700, y: 540 };

try {
	for (const renderer of ["egui", "react-flow"]) {
		await page.goto(
			`http://127.0.0.1:4339/host?drop-ghosts&boardRenderer=${renderer}`,
			{ timeout: 180000 },
		);
		await waitFor(() => !!window.__boardHost);
		await waitFor(
			(renderer) =>
				document
					.querySelector("[data-board-renderer]")
					?.getAttribute("data-board-renderer") === renderer,
			renderer,
		);
		await page.evaluate(() => window.__boardHost.viewport(20, 20, 1));
		await page
			.getByRole("button", { name: "Variables & Functions" })
			.first()
			.click();

		await test(`${renderer}: a dragged variable previews its node under the pointer`, async () => {
			const from = await startDrag(
				page.getByText("Counter", { exact: true }).first(),
			);
			await page.mouse.move(DROP.x, DROP.y, { steps: 20 });
			await ghostCount(1);
			await assertGhostAt(DROP, "variable");
			assert.match(await ghosts.first().innerText(), /Get Variable/);
			await page.screenshot({ path: `${output}/${renderer}-variable.png` });

			await page.mouse.move(from.x, from.y + 40, { steps: 10 });
			await ghostCount(0);
			await page.mouse.move(DROP.x, DROP.y, { steps: 10 });
			await ghostCount(1);
			await page.mouse.up();

			await page.getByRole("dialog").waitFor();
			assert.equal(
				await ghosts.count(),
				1,
				"The preview stays at the drop point while the user picks Get or Set",
			);
			await assertGhostAt(DROP, "pending variable");
			await page.screenshot({
				path: `${output}/${renderer}-variable-pending.png`,
			});
			await page.getByRole("button", { name: "Set", exact: true }).click();
			await assertPlacedAt("variable_set", DROP, "variable");
			await ghostCount(0);
		});

		await test(`${renderer}: a dragged function previews its call node with the function's pins`, async () => {
			await page.getByRole("tab", { name: /^Functions/ }).click();
			const point = { x: DROP.x + 260, y: DROP.y + 130 };
			await startDrag(page.getByText("Fetch", { exact: true }).first());
			await page.mouse.move(point.x, point.y, { steps: 20 });
			await ghostCount(1);
			await assertGhostAt(point, "function");
			const text = await ghosts.first().innerText();
			for (const label of ["Call Fetch", "url", "body"])
				assert.ok(text.includes(label), `The call ghost shows ${label}`);
			await page.screenshot({ path: `${output}/${renderer}-function.png` });
			await page.mouse.up();
			await assertPlacedAt("control_call_function", point, "function");
			await ghostCount(0);
		});

		await test(`${renderer}: a dragged file previews both nodes that address it, wired`, async () => {
			await page.getByRole("button", { name: "Explorer" }).first().click();
			const point = { x: DROP.x - 120, y: DROP.y + 260 };
			await startDrag(page.getByText("invoice.pdf", { exact: true }).first());
			await page.mouse.move(point.x, point.y, { steps: 20 });
			await ghostCount(2);
			await assertGhostAt(point, "storage");
			const [dir, child] = await Promise.all([
				ghosts.nth(0).boundingBox(),
				ghosts.nth(1).boundingBox(),
			]);
			assert.ok(
				child.x > dir.x + dir.width,
				"The child node sits right of its directory",
			);
			assert.equal(
				await page.locator("[data-board-drop-ghosts] svg path").count(),
				1,
				"The directory's path is wired into the child",
			);
			assert.match(
				await page.locator("[data-board-drop-ghosts]").innerText(),
				/invoice\.pdf/,
			);
			await page.screenshot({ path: `${output}/${renderer}-storage.png` });
			await page.mouse.up();
			await assertPlacedAt("path_from_upload_dir", point, "storage");
			assert.equal((await placedNodes("child")).length, 1);
			await ghostCount(0);
		});
	}
	assert.deepEqual(errors, [], "The host raised no page errors");
	console.log(`${results.length} drop ghost checks passed`);
} finally {
	await browser.close();
}
