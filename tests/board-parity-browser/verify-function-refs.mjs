import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { Position, getBezierPath } from "@xyflow/react";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output = `tmp/board-parity-browser/function-refs/${kind}`;
await mkdir(output, { recursive: true });
const browser = await (kind === "webkit"
	? webkit.launch()
	: chromium.launch({
			headless: true,
			executablePath: puppeteer.executablePath(),
			args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
		}));
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
page.setDefaultTimeout(10000);
const errors = [];
const results = [];
page.on("pageerror", (error) => errors.push(error.message));
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = () => api("snapshot");
async function center(id) {
	const b = await page.locator(`[data-handleid="${id}"]`).boundingBox();
	assert.ok(b, id);
	return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
}
async function drag(from, to) {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps: 12 });
	await page.mouse.up();
	await page.waitForTimeout(250);
}
function pathPoint(path, at) {
	return page.evaluate(
		({ path, at }) => {
			const element = document.createElementNS(
				"http://www.w3.org/2000/svg",
				"path",
			);
			element.setAttribute("d", path);
			const total = element.getTotalLength();
			const point = element.getPointAtLength(
				at.fraction === undefined ? total - at.fromEnd : total * at.fraction,
			);
			return { x: point.x, y: point.y };
		},
		{ path, at },
	);
}
async function refs(expected) {
	await page.waitForFunction(
		(expected) =>
			JSON.stringify(
				window.__boardHost.snapshot().board.nodes["ref-a"].fn_refs.fn_refs,
			) === JSON.stringify(expected),
		expected,
	);
	const s = await snapshot();
	assert.deepEqual(s.board.nodes["ref-a"].fn_refs.fn_refs, expected);
	assert.deepEqual(s.board.nodes["ref-a"].pins["ref-a.in"].depends_on, []);
	assert.deepEqual(s.board.nodes["ref-a"].pins["ref-a.out"].connected_to, []);
}
async function undo() {
	await page.keyboard.press("ControlOrMeta+z");
	await page.waitForTimeout(300);
}
try {
	await page.goto(
		"http://127.0.0.1:4339/host?boardRenderer=egui&function-refs",
		{ timeout: 180000 },
	);
	await page.waitForFunction(
		() =>
			window.__boardHost &&
			document.querySelector('[data-board-renderer="egui"]'),
	);
	if (!process.env.BOARD_REF_PALETTE_ONLY) {
		for (const zoom of [1, 0.25, 2.5]) {
			await api("viewport", 20, 400 - 380 * zoom, zoom);
			await page.waitForTimeout(300);
			await drag(await center("ref_out_ref-a"), await center("ref_in_ref-b"));
			await refs(["ref-b"]);
			assert.equal(
				(await snapshot()).transactions.at(-1).commands[0].command_type,
				"UpdateNode",
			);
			results.push({
				name: `production output-to-event reference at zoom ${zoom}`,
				passed: true,
			});
			await undo();
			await refs([]);
			await drag(await center("ref_in_ref-b"), await center("ref_out_ref-a"));
			await refs(["ref-b"]);
			results.push({
				name: `production event-to-output reference at zoom ${zoom}`,
				passed: true,
			});
			await undo();
			await refs([]);
		}
	}
	await api("viewport", 20, 400 - 380 * 2.5, 2.5);
	await page.waitForTimeout(300);
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_scene;
		window.__refScenes = new Map();
		module.Handle.prototype.set_scene = function (json) {
			const result = original.call(this, json);
			window.__refScenes.set(this, JSON.parse(json));
			return result;
		};
	});
	const sourceBox = await page
		.locator('[data-handleid="ref_out_ref-a"]')
		.boundingBox();
	assert.ok(sourceBox);
	const corner = { x: sourceBox.x + 2, y: sourceBox.y + sourceBox.height - 2 };
	const sourceCenter = await center("ref_out_ref-a");
	assert.ok(
		Math.hypot(corner.x - sourceCenter.x, corner.y - sourceCenter.y) > 12,
		"A visibly draggable high-zoom corner exceeds the old center hit radius",
	);
	assert.equal(
		await page.evaluate(
			(p) =>
				document
					.elementFromPoint(p.x, p.y)
					?.closest("[data-handleid]")
					?.getAttribute("data-handleid"),
			corner,
		),
		"ref_out_ref-a",
	);
	const sourceCoordinates = (await snapshot()).board.nodes["ref-a"].coordinates;
	await drag(corner, { x: 1000, y: 850 });
	await page
		.getByPlaceholder("Search nodes and actions", { exact: true })
		.waitFor();
	assert.deepEqual(
		(await snapshot()).board.nodes["ref-a"].coordinates,
		sourceCoordinates,
	);
	results.push({
		name: "high-zoom reference handle corner starts wire and opens compatible palette",
		passed: true,
	});
	await page.getByRole("button", { name: "Event", exact: true }).click();
	await page.waitForFunction(() =>
		window.__boardHost
			.snapshot()
			.transactions.at(-1)
			?.commands.some((c) => c.command_type === "AddNode"),
	);
	const transaction = (await snapshot()).transactions.at(-1);
	assert.deepEqual(
		transaction.commands.map((c) => c.command_type),
		["AddNode", "UpdateNode"],
	);
	const created = transaction.commands[0].node;
	await refs([created.id]);
	await page.waitForFunction(
		(id) =>
			window.__boardHost
				.snapshot()
				.edges.some(
					(e) =>
						e.sourceHandle === "ref_out_ref-a" &&
						e.targetHandle === `ref_in_${id}`,
				),
		created.id,
	);
	await page.waitForTimeout(500);
	await page.screenshot({ path: `${output}/created-event.png` });
	const geometry = await page.evaluate((id) => {
		const [handle, scene] = [...window.__refScenes].at(-1);
		const edge = scene.edges.find(
			(e) =>
				e.source.pinId === "ref_out_ref-a" && e.target.pinId === `ref_in_${id}`,
		);
		const root = document.querySelector(".react-flow").getBoundingClientRect();
		const v = window.__boardHost.snapshot().viewport;
		const endpoints = ["source", "target"].map((end) => {
			const endpoint = edge[end];
			const node = scene.nodes.find((n) => n.id === endpoint.nodeId);
			const pin = node.pins.find((p) => p.id === endpoint.pinId);
			const b = document
				.querySelector(`[data-handleid="${endpoint.pinId}"]`)
				.getBoundingClientRect();
			return {
				scene: {
					x: root.x + v.x + (node.x + pin.x) * v.zoom,
					y: root.y + v.y + (node.y + pin.y) * v.zoom,
				},
				dom: { x: b.x + b.width / 2, y: b.y + b.height / 2 },
			};
		});
		return {
			edge,
			endpoints,
			stats: [...window.__refScenes.keys()].map((h) => JSON.parse(h.stats())),
		};
	}, created.id);
	assert.ok(geometry.edge);
	for (const endpoint of geometry.endpoints)
		assert.ok(
			Math.hypot(
				endpoint.scene.x - endpoint.dom.x,
				endpoint.scene.y - endpoint.dom.y,
			) < 1.1,
			"Newly placed reference endpoints match visible production handles",
		);
	for (const stats of geometry.stats) assert.equal(stats.unresolvedEdges, 0);
	results.push({
		name: "Event action atomically creates reference and paints measured endpoints",
		passed: true,
	});
	await api("viewport", 20, 20, 1);
	await page.waitForTimeout(400);
	const a = await center("ref_out_ref-a");
	const b = await center(`ref_in_${created.id}`);
	const [path] = getBezierPath({
		sourceX: a.x,
		sourceY: a.y,
		targetX: b.x,
		targetY: b.y,
		sourcePosition: Position.Bottom,
		targetPosition: Position.Top,
	});
	const point = await pathPoint(path, { fraction: 0.85 });
	await page.mouse.click(point.x, point.y);
	await page.waitForFunction(
		(id) =>
			window.__boardHost
				.snapshot()
				.edges.some((e) => e.target === id && e.selected),
		created.id,
	);
	// A pin press starts a new wire; the wire's end corridor moves the existing one.
	await drag(
		await pathPoint(path, { fromEnd: 18 }),
		await center("ref_in_ref-c"),
	);
	await refs(["ref-c"]);
	results.push({
		name: "newly created Event reference reconnects to another real Event",
		passed: true,
	});
	await undo();
	await refs([created.id]);
	await undo();
	await refs([]);
	assert.equal((await snapshot()).board.nodes[created.id], undefined);
	results.push({
		name: "undo restores reconnected Event then removes placement and reference together",
		passed: true,
	});

	assert.deepEqual(errors, []);
	console.log(JSON.stringify({ kind, results, errors }, null, 2));
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, results, errors }, null, 2),
	);
} catch (error) {
	await page.screenshot({ path: `${output}/failure.png` });
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{ error: String(error), results, errors, state: await snapshot() },
			null,
			2,
		),
	);
	throw error;
} finally {
	await browser.close();
}
