import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output =
	process.env.BOARD_REROUTE_OUTPUT ??
	`tmp/board-parity-browser/reroute-drag/${kind}`;
await mkdir(output, { recursive: true });
const browser = await (kind === "webkit"
	? webkit.launch()
	: chromium.launch({
			headless: true,
			executablePath:
				process.env.CHROME_PATH ?? (await puppeteer.executablePath()),
			args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
		}));
const page = await browser.newPage({
	viewport: { width: 1600, height: 1000 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(15000);
const errors = [];
const results = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	return ["127.0.0.1", "localhost"].includes(url.hostname) ||
		["data:", "blob:"].includes(url.protocol)
		? route.continue()
		: route.abort();
});
await page.route("**/board-canvas/board_canvas.js", (route) =>
	route.fulfill({
		contentType: "text/javascript",
		body: `export { default } from "/board-canvas/board_canvas.js?rerouteProbe=1";
import { start as startCanvas } from "/board-canvas/board_canvas.js?rerouteProbe=1";
export async function start(canvas, json, palette) {
  const handle = await startCanvas(canvas, json, palette);
  const entry = {canvas, handle, scene: JSON.parse(json), layer: "all"};
  (window.__rerouteCanvases ??= []).push(entry);
  const scene = handle.set_scene.bind(handle);
  handle.set_scene = (value) => { entry.scene = JSON.parse(value); return scene(value); };
  const layer = handle.set_layer.bind(handle);
  handle.set_layer = (value) => { entry.layer = value; return layer(value); };
  const positions = handle.set_positions.bind(handle);
  handle.set_positions = (value) => {
    const updates = new Map(JSON.parse(value).map((node) => [node.id, node]));
    entry.scene.nodes = entry.scene.nodes.map((node) => ({...node, ...updates.get(node.id)}));
    return positions(value);
  };
  return handle;
}`,
	}),
);
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = () => api("snapshot");
const node = (id) => page.locator(`.react-flow__node[data-id="${id}"]`);
const center = async (locator) => {
	const box = await locator.boundingBox();
	assert.ok(box, "The real DOM interaction target must be visible");
	return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
};
const topology = (board) =>
	Object.fromEntries(
		Object.values(board.nodes).map((entry) => [
			entry.id,
			{
				pins: entry.pins,
				fn_refs: entry.fn_refs,
			},
		]),
	);
async function bodyPoint(id) {
	const state = await snapshot();
	const entry = state.nodes.find((value) => value.id === id);
	assert.ok(entry);
	if (entry.nativeDom) return center(node(id));
	const box = await page.locator(".react-flow").boundingBox();
	assert.ok(box);
	return {
		x: box.x + state.viewport.x + (entry.position.x + 8) * state.viewport.zoom,
		y: box.y + state.viewport.y + (entry.position.y + 6) * state.viewport.zoom,
	};
}
async function prepare(selected, zoom) {
	await page.goto("http://127.0.0.1:4339/host?boardRenderer=egui", {
		timeout: 180000,
	});
	await page.waitForFunction(
		() =>
			!!window.__boardHost &&
			document.querySelector('[data-board-renderer="egui"]'),
	);
	await page.waitForTimeout(500);
	await api("viewport", 20, 20, 1);
	const a = await center(
		node("string-a").locator('[data-handleid="string-a.out"]'),
	);
	const b = await center(
		node("string-b").locator('[data-handleid="string-b.in"]'),
	);
	await page.mouse.dblclick((a.x + b.x) / 2, (a.y + b.y) / 2);
	await page.waitForFunction(() =>
		Object.values(window.__boardHost.snapshot().board.nodes).some(
			(entry) => entry.name === "reroute",
		),
	);
	const route = Object.values((await snapshot()).board.nodes).find(
		(entry) => entry.name === "reroute",
	);
	assert.ok(route);
	await page.mouse.click(1450, 800);
	await page.mouse.move(10, 10);
	await page.waitForTimeout(350);
	if (selected) {
		const point = await bodyPoint(route.id);
		await page.keyboard.down("Shift");
		await page.mouse.move(point.x - 20, point.y - 20);
		await page.mouse.down();
		await page.mouse.move(point.x + 20, point.y + 20, { steps: 8 });
		await page.mouse.up();
		await page.keyboard.up("Shift");
		await page.mouse.move(10, 10);
		await page.waitForFunction(
			(id) =>
				window.__boardHost.snapshot().nodes.find((entry) => entry.id === id)
					?.selected,
			route.id,
		);
	}
	await api(
		"viewport",
		650 - (route.coordinates[0] + 8) * zoom,
		380 - (route.coordinates[1] + 6) * zoom,
		zoom,
	);
	await page.waitForTimeout(350);
	const state = await snapshot();
	const presented = state.nodes.find((entry) => entry.id === route.id);
	assert.equal(!!presented?.selected, selected);
	assert.equal(
		!!presented?.nativeDom,
		selected,
		"The test exercises the requested native or rich reroute",
	);
	return route.id;
}
async function endpointErrors(id) {
	return page.evaluate((id) => {
		const edges = window.__boardHost
			.snapshot()
			.edges.filter((edge) => edge.source === id || edge.target === id);
		return edges.map((edge) => {
			const source = edge.source === id;
			const endpoint = source ? edge.sourceHandle : edge.targetHandle;
			const handle = document.querySelector(
				`.react-flow__node[data-id="${CSS.escape(id)}"] [data-handleid="${CSS.escape(endpoint)}"]`,
			);
			const path = document.querySelector(
				`.react-flow__edge[data-id="${CSS.escape(edge.id)}"] .react-flow__edge-path`,
			);
			if (!handle) throw new Error(`Missing reroute handle for ${edge.id}`);
			let screen;
			let renderer;
			if (path instanceof SVGPathElement) {
				const matrix = path.getScreenCTM();
				if (!matrix) throw new Error(`Missing path transform for ${edge.id}`);
				const point = path.getPointAtLength(source ? 0 : path.getTotalLength());
				screen = new DOMPoint(point.x, point.y).matrixTransform(matrix);
				renderer = "svg";
			} else {
				const painter = window.__rerouteCanvases?.find(
					(entry) => entry.canvas.isConnected && entry.layer === "edges",
				);
				const native = painter?.scene.edges.find(
					(value) => value.id === edge.id,
				);
				const owner = painter?.scene.nodes.find((value) => value.id === id);
				const pin = owner?.pins.find((value) => value.id === endpoint);
				const raw = painter?.handle.stats();
				const stats = typeof raw === "string" ? JSON.parse(raw) : raw;
				if (
					!native ||
					native.dom ||
					!owner ||
					!pin ||
					stats?.unresolvedEdges !== 0 ||
					!(stats?.renderedEdges > 0)
				)
					throw new Error(
						`Missing painted native connection for ${edge.id}: ${JSON.stringify({ native, stats })}`,
					);
				const viewport = window.__boardHost.snapshot().viewport;
				const root = document
					.querySelector(".react-flow")
					.getBoundingClientRect();
				screen = {
					x: root.x + viewport.x + (owner.x + pin.x) * viewport.zoom,
					y: root.y + viewport.y + (owner.y + pin.y) * viewport.zoom,
				};
				renderer = "native";
			}
			const rect = handle.getBoundingClientRect();
			return {
				edge: edge.id,
				renderer,
				error: Math.hypot(
					screen.x - rect.x - rect.width / 2,
					screen.y - rect.y - rect.height / 2,
				),
			};
		});
	}, id);
}
try {
	for (const selected of [false, true]) {
		for (const zoom of [0.25, 0.5]) {
			const name = `${selected ? "selected rich" : "unselected native"} reroute body drags at zoom ${zoom}`;
			if (
				process.env.BOARD_REROUTE_FILTER &&
				!new RegExp(process.env.BOARD_REROUTE_FILTER).test(name)
			)
				continue;
			const id = await prepare(selected, zoom);
			const before = await snapshot();
			const point = await bodyPoint(id);
			const target = await page.evaluate(({ x, y }) => {
				const hit = document.elementFromPoint(x, y);
				const wrapper = hit?.closest(".react-flow__node");
				return {
					tag: hit?.tagName,
					handle: hit
						?.closest(".react-flow__handle")
						?.getAttribute("data-handleid"),
					body: wrapper?.getBoundingClientRect().toJSON(),
					handles: [
						...(wrapper?.querySelectorAll(".react-flow__handle") ?? []),
					].map((element) => ({
						id: element.getAttribute("data-handleid"),
						rect: element.getBoundingClientRect().toJSON(),
					})),
				};
			}, point);
			await page.mouse.move(point.x, point.y);
			await page.mouse.down();
			await page.mouse.move(point.x + 48, point.y + 32, { steps: 10 });
			await page.mouse.up();
			await page.waitForFunction(
				({ id, count }) => {
					const state = window.__boardHost.snapshot();
					return (
						state.transactions.length > count &&
						state.transactions.at(-1).finished &&
						state.nodes.find((entry) => entry.id === id)?.selected
					);
				},
				{ id, count: before.transactions.length },
			);
			await page.waitForTimeout(450);
			const after = await snapshot();
			assert.equal(
				after.transactions.length,
				before.transactions.length + 1,
				"Body movement is one undo transaction",
			);
			assert.deepEqual(
				after.transactions
					.at(-1)
					.commands.map((command) => command.command_type),
				["MoveNode"],
			);
			assert.deepEqual(
				topology(after.board),
				topology(before.board),
				"Body dragging cannot connect or disconnect a pin",
			);
			for (const [axis, delta] of [
				[0, 48],
				[1, 32],
			]) {
				const moved =
					(after.board.nodes[id].coordinates[axis] -
						before.board.nodes[id].coordinates[axis]) *
					zoom;
				assert.ok(
					Math.abs(moved - delta) <= 8,
					`Reroute follows the pointer, allowing existing alignment guides (${moved}px versus ${delta}px)`,
				);
			}
			assert.equal(
				await page
					.getByPlaceholder("Search nodes and actions", { exact: true })
					.isVisible(),
				false,
			);
			const geometry = await endpointErrors(id);
			assert.equal(
				geometry.length,
				2,
				"Both incident connections remain rendered",
			);
			for (const endpoint of geometry)
				assert.ok(
					endpoint.error <= 2,
					`Moved wire ${endpoint.edge} remains attached (${endpoint.error}px)`,
				);
			await page.screenshot({
				path: `${output}/${selected ? "rich" : "native"}-${zoom}.png`,
			});
			await page.locator("[data-board-canvas-input]").focus();
			await page.keyboard.press("ControlOrMeta+z");
			await page.waitForFunction(
				({ id, coordinates }) => {
					const node = window.__boardHost.snapshot().board.nodes[id];
					return (
						node.coordinates[0] === coordinates[0] &&
						node.coordinates[1] === coordinates[1]
					);
				},
				{ id, coordinates: before.board.nodes[id].coordinates },
			);
			assert.deepEqual(
				topology((await snapshot()).board),
				topology(before.board),
			);
			assert.ok(await page.locator('[data-board-renderer="egui"]').count());
			assert.deepEqual(errors, []);
			results.push({ name, passed: true, target, geometry });
			console.log("PASS", name);
		}
	}
} catch (error) {
	results.push({ passed: false, error: error.stack ?? error.message });
	await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
	await writeFile(
		`${output}/failure-state.json`,
		JSON.stringify(await snapshot().catch(() => ({})), null, 2),
	);
	throw error;
} finally {
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ browser: kind, results, errors }, null, 2),
	);
	await browser.close();
	console.log(
		`Browser closed; ${results.filter((result) => result.passed).length}/4 scenarios passed.`,
	);
}
