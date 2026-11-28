import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const origin = process.env.BOARD_PARITY_URL ?? "http://127.0.0.1:4339";
const output = `tmp/board-parity-browser/dynamic-pins/${kind}`;
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
const api = (page, method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = (page) => api(page, "snapshot");
const branches = (state) =>
	Object.values(state.board.nodes.parallel.pins)
		.filter((pin) => pin.name === "exec_out")
		.sort((a, b) => a.index - b.index);
const controls = (page) => page.locator("[data-egui-pin-controls]");
const manager = (page) => page.locator("[data-egui-pin-manager]");
const waitBranches = (page, count) =>
	page.waitForFunction(
		(expected) =>
			Object.values(
				window.__boardHost.snapshot().board.nodes.parallel.pins,
			).filter((pin) => pin.name === "exec_out").length === expected,
		count,
	);

async function position(page, zoom = 1) {
	await api(page, "viewport", 200 - 1500 * zoom, 160 - 100 * zoom, zoom);
	await page.waitForTimeout(200);
}
async function selectParallel(page) {
	const state = await snapshot(page);
	const bounds = await page.locator(".react-flow").boundingBox();
	assert.ok(bounds);
	await page.mouse.click(
		bounds.x + state.viewport.x + 1560 * state.viewport.zoom,
		bounds.y + state.viewport.y + 110 * state.viewport.zoom,
	);
	await page.waitForFunction(() =>
		window.__boardHost
			.snapshot()
			.nodes.some((node) => node.id === "parallel" && node.selected),
	);
}
async function openManager(page) {
	if (await manager(page).isVisible()) return;
	await controls(page).locator("[data-egui-pin-manage]").click();
	await manager(page).waitFor();
}
async function removePin(page, id) {
	await openManager(page);
	const button = manager(page)
		.locator(`[data-pin-id="${id}"]`)
		.getByRole("button", { name: "Delete Pin", exact: true });
	// The marker may be on the button itself or its numbered row.
	const direct = manager(page).locator(
		`button[data-pin-id="${id}"][data-pin-action="remove"]`,
	);
	await ((await direct.count()) ? direct : button).click();
}
async function history(page, kind) {
	await page.keyboard.press("Escape");
	const count = (await snapshot(page)).transactions.length;
	await page.keyboard.press(
		kind === "undo" ? "ControlOrMeta+z" : "ControlOrMeta+Shift+z",
	);
	await page.waitForFunction(
		({ count, kind }) => {
			const transactions = window.__boardHost.snapshot().transactions;
			return (
				transactions.length > count &&
				transactions.at(-1).kind === kind &&
				transactions.at(-1).finished
			);
		},
		{ count, kind },
	);
	await page.waitForTimeout(150);
}
async function pinCenter(page, node, id) {
	const bounds = await page
		.locator(`.react-flow__node[data-id="${node}"] [data-handleid="${id}"]`)
		.boundingBox();
	assert.ok(bounds, `The ${id} handle is mounted`);
	return { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 };
}
async function run(name, query, verify) {
	const context = await browser.newContext({
		viewport: { width: 1600, height: 1000 },
		deviceScaleFactor: 2,
		reducedMotion: "reduce",
	});
	const page = await context.newPage();
	page.setDefaultTimeout(15_000);
	page.on("pageerror", (error) =>
		errors.push({ name, message: error.message }),
	);
	await context.route("**/*", (route) => {
		const url = new URL(route.request().url());
		return ["127.0.0.1", "localhost"].includes(url.hostname) ||
			["data:", "blob:"].includes(url.protocol)
			? route.continue()
			: route.abort();
	});
	try {
		const parameters = new URLSearchParams(query);
		if (!parameters.has("dynamic-pins")) parameters.set("dynamic-pins", "");
		await page.goto(`${origin}/host?${parameters}`, {
			waitUntil: "domcontentloaded",
			timeout: 180_000,
		});
		await page.waitForFunction(() => !!window.__boardHost?.snapshot);
		await page.waitForFunction(
			(renderer) =>
				document
					.querySelector("[data-board-renderer]")
					?.getAttribute("data-board-renderer") === renderer &&
				window.__boardHost
					.snapshot()
					.nodes.some((node) => node.id === "parallel"),
			parameters.get("boardRenderer"),
		);
		await page.waitForLoadState("networkidle");
		await page.waitForTimeout(750);
		await position(page);
		if (parameters.has("historical")) {
			const bounds = await page.locator(".react-flow").boundingBox();
			assert.ok(bounds);
			await page.mouse.move(bounds.x + 260, bounds.y + 170);
			await page.waitForTimeout(250);
		} else {
			await selectParallel(page);
		}
		await verify(page);
		assert.deepEqual(errors, []);
		results.push({ name, passed: true });
		console.log(`PASS ${name}`);
	} catch (error) {
		await page.screenshot({ path: `${output}/${name}-failure.png` });
		await writeFile(
			`${output}/${name}-failure.json`,
			JSON.stringify(await snapshot(page).catch(() => ({})), null, 2),
		);
		results.push({ name, passed: false, message: error.message });
		throw error;
	} finally {
		await context.close();
	}
}

try {
	await run("native-actions", "boardRenderer=egui", async (page) => {
		await page.locator('[data-board-renderer="egui"]').waitFor();
		await controls(page).waitFor();
		const add = controls(page).getByRole("button", {
			name: "Add pin",
			exact: true,
		});
		const initial = await snapshot(page);
		assert.equal(branches(initial).length, 2);
		await openManager(page);
		assert.equal(
			await manager(page).getByRole("button", { name: "Delete Pin" }).count(),
			0,
			"The two catalog branches cannot be removed",
		);
		await page.keyboard.press("Escape");
		await api(page, "rejectNext");
		await add.click();
		await page
			.getByText("Fixture rejected the command batch", { exact: true })
			.waitFor();
		assert.deepEqual(
			(await snapshot(page)).board.nodes.parallel,
			initial.board.nodes.parallel,
		);
		await page.waitForFunction(
			() =>
				!document.querySelector(
					'[data-egui-pin-controls] [data-pin-action="add"]',
				)?.disabled,
		);
		await add.click();
		await waitBranches(page, 3);
		const added = branches(await snapshot(page)).find(
			(pin) => !initial.board.nodes.parallel.pins[pin.id],
		);
		assert.ok(added);
		const pinsAfterAdd = Object.values(
			(await snapshot(page)).board.nodes.parallel.pins,
		);
		for (const direction of ["Input", "Output"]) {
			const ordered = pinsAfterAdd
				.filter((pin) => pin.pin_type === direction)
				.sort((a, b) => a.index - b.index);
			assert.deepEqual(
				ordered.map((pin) => pin.index),
				ordered.map((_, index) => index + 1),
				`${direction} pin indices remain sequential`,
			);
			if (direction === "Output") {
				assert.deepEqual(
					ordered.map((pin) => pin.name),
					["exec_out", "exec_out", "exec_out", "exec_done"],
				);
				assert.equal(
					ordered[2].id,
					added.id,
					"The appended branch precedes Done",
				);
			}
		}
		const typical = await add.boundingBox();
		await openManager(page);
		await page.waitForTimeout(250);
		await page.screenshot({ path: `${output}/native-zoom-1.png` });
		await page.keyboard.press("Escape");
		await position(page, 0.5);
		const low = await add.boundingBox();
		assert.ok(typical && low);
		assert.ok(Math.abs(typical.width - low.width) <= 1);
		assert.ok(Math.abs(typical.height - low.height) <= 1);
		await openManager(page);
		await page.waitForTimeout(250);
		await page.screenshot({ path: `${output}/native-zoom-0.5.png` });
		await page.keyboard.press("Escape");
		await position(page, 2.5);
		const high = await add.boundingBox();
		assert.ok(typical && high);
		assert.ok(Math.abs(typical.width - high.width) <= 1);
		assert.ok(
			Math.abs(typical.height - high.height) <= 1,
			"Dynamic pin actions retain their screen-space size at high zoom",
		);
		await openManager(page);
		await page.waitForTimeout(250);
		await page.screenshot({ path: `${output}/native-zoom-2.5.png` });
		await removePin(page, added.id);
		await waitBranches(page, 2);
		await history(page, "undo");
		await waitBranches(page, 3);
		assert.ok((await snapshot(page)).board.nodes.parallel.pins[added.id]);
		await history(page, "redo");
		await waitBranches(page, 2);
		await position(page);
		await selectParallel(page);
		await add.click();
		await waitBranches(page, 3);
		const connected = branches(await snapshot(page)).find(
			(pin) => !initial.board.nodes.parallel.pins[pin.id],
		);
		assert.ok(connected);
		const from = await pinCenter(page, "parallel", connected.id);
		// Hover mounts the target's real production handle before the native gesture.
		await page.mouse.move(600, 320);
		const targetPosition = (await snapshot(page)).nodes.find(
			(node) => node.id === "parallel-target",
		);
		assert.ok(targetPosition);
		const root = await page.locator(".react-flow").boundingBox();
		const viewport = (await snapshot(page)).viewport;
		await page.mouse.move(
			root.x + viewport.x + 1880 * viewport.zoom,
			root.y + viewport.y + 190 * viewport.zoom,
		);
		await page.waitForTimeout(250);
		const to = await pinCenter(page, "parallel-target", "parallel-target.in");
		await page.mouse.move(from.x, from.y);
		await page.mouse.down();
		await page.mouse.move(to.x, to.y, { steps: 15 });
		await page.mouse.up();
		await page.waitForFunction(
			(id) =>
				window.__boardHost
					.snapshot()
					.board.nodes["parallel-target"].pins[
						"parallel-target.in"
					].depends_on.includes(id),
			connected.id,
		);
		await page.waitForFunction(
			(id) =>
				window.__boardHost
					.snapshot()
					.edges.some((edge) => edge.sourceHandle === id),
			connected.id,
		);
		await selectParallel(page);
		const beforeRemoval = await snapshot(page);
		assert.ok(
			beforeRemoval.edges.some((edge) => edge.sourceHandle === connected.id),
		);
		await openManager(page);
		assert.match(await manager(page).innerText(), /1 connected/);
		await removePin(page, connected.id);
		await waitBranches(page, 2);
		await page.waitForFunction(
			(id) =>
				!window.__boardHost
					.snapshot()
					.edges.some((edge) => edge.sourceHandle === id),
			connected.id,
		);
		const removed = await snapshot(page);
		assert.deepEqual(
			removed.board.nodes["parallel-target"].pins["parallel-target.in"]
				.depends_on,
			[],
		);
		assert.equal(
			removed.edges.some((edge) => edge.sourceHandle === connected.id),
			false,
		);
		await history(page, "undo");
		await waitBranches(page, 3);
		assert.deepEqual(
			(await snapshot(page)).board.nodes,
			beforeRemoval.board.nodes,
			"Undo restores the removed pin and both connection endpoints",
		);
		await history(page, "redo");
		await waitBranches(page, 2);
		assert.deepEqual((await snapshot(page)).board.nodes, removed.board.nodes);
		assert.equal(await page.locator('[data-board-renderer="egui"]').count(), 1);
	});
	await run(
		"native-readonly",
		"boardRenderer=egui&dynamic-pins=expanded&historical",
		async (page) => {
			await page.locator('[data-board-renderer="egui"]').waitFor();
			const before = await snapshot(page);
			assert.equal(branches(before).length, 3);
			assert.equal(await controls(page).count(), 0);
			assert.equal(
				await page
					.getByRole("button", { name: /^(Add pin|Delete Pin)$/ })
					.count(),
				0,
			);
			await position(page, 2.5);
			await page.waitForTimeout(250);
			await page.screenshot({ path: `${output}/native-readonly.png` });
			assert.deepEqual((await snapshot(page)).board, before.board);
			assert.equal((await snapshot(page)).transactions.length, 0);
		},
	);
	await run("legacy-controls", "boardRenderer=react-flow", async (page) => {
		await page.locator('[data-board-renderer="react-flow"]').waitFor();
		assert.equal(await controls(page).count(), 0);
		const legacy = page.locator('.react-flow__node[data-id="parallel"]');
		await legacy.getByRole("button", { name: "Add pin", exact: true }).click();
		await waitBranches(page, 3);
		await legacy.hover();
		await legacy
			.getByRole("button", { name: "Delete Pin", exact: true })
			.click();
		await waitBranches(page, 2);
		await page.screenshot({ path: `${output}/legacy-controls.png` });
	});
} finally {
	await browser.close();
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ browser: kind, results, errors }, null, 2),
	);
	console.log(
		`Browser closed; ${results.length}/3 cases completed, ${errors.length} page errors.`,
	);
}
