import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { Position, getBezierPath } from "@xyflow/react";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const origin = process.env.BOARD_PARITY_URL ?? "http://127.0.0.1:4339";
const output = `tmp/board-parity-browser/connection-targets/${kind}`;
await mkdir(output, { recursive: true });
const browser = await (kind === "webkit"
	? webkit.launch()
	: chromium.launch({
			headless: true,
			executablePath: await puppeteer.executablePath(),
			args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
		}));
const errors = [];
const results = [];
let gesture;
let page;
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = () => api("snapshot");
async function load(fixture) {
	// Each URL describes a different authoritative graph with the same host board id.
	await page?.close();
	page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
	page.setDefaultTimeout(10000);
	page.on("pageerror", (error) => errors.push(error.message));
	await page.goto(
		`${origin}/host?boardRenderer=egui&connection-targets=${fixture}`,
		{ timeout: 180000 },
	);
	await page.waitForFunction(
		() =>
			window.__boardHost &&
			document.querySelector('[data-board-renderer="egui"]'),
	);
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_overlay;
		module.Handle.prototype.set_overlay = function (json) {
			window.__connectionOverlay = JSON.parse(json);
			return original.call(this, json);
		};
	});
}
async function center(id) {
	const locator = page.locator(`[data-handleid="${id}"]`);
	await locator.waitFor();
	const box = await locator.boundingBox();
	assert.ok(box, id);
	return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}
async function highlight(pinId) {
	await page.waitForFunction(
		(pinId) =>
			window.__connectionOverlay?.hoveredPin?.pinId === pinId &&
			window.__connectionOverlay.hoveredPin.valid &&
			window.__connectionOverlay.connection?.valid,
		pinId,
	);
}
async function drag(from, to) {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps: 12 });
	await page.mouse.up();
	await page.waitForTimeout(250);
}
async function expectEdges(expected) {
	await page.waitForFunction((expected) => {
		const edges = window.__boardHost
			.snapshot()
			.edges.map((edge) => `${edge.sourceHandle}->${edge.targetHandle}`)
			.sort();
		return JSON.stringify(edges) === JSON.stringify(expected);
	}, [...expected].sort());
}
async function undo(expected) {
	await page.keyboard.press("ControlOrMeta+z");
	await expectEdges(expected);
	await page.mouse.click(1300, 700);
	await page.waitForTimeout(180);
}
async function pointOnWire(reference, end) {
	const source = await center(reference ? "ref_out_source" : "source.out");
	const target = await center(reference ? "ref_in_target" : "target.first");
	const state = await snapshot();
	const edgeId = state.edges.find(
		(edge) => edge.source === "source" && edge.target === "target",
	)?.id;
	assert.ok(edgeId);
	const svgPoint = await page.evaluate(
		({ edgeId, end }) => {
			const edge = [...document.querySelectorAll(".react-flow__edge")].find(
				(element) => element.getAttribute("data-id") === edgeId,
			);
			const path = edge?.querySelector(".react-flow__edge-path");
			if (!(path instanceof SVGPathElement)) return null;
			const matrix = path.getScreenCTM();
			if (!matrix) return null;
			const scale = Math.hypot(matrix.a, matrix.b);
			const length = path.getTotalLength();
			const point = path
				.getPointAtLength(
					end === "source"
						? 18 / scale
						: end === "target"
							? length - 18 / scale
							: length / 2,
				)
				.matrixTransform(matrix);
			return {
				x: point.x,
				y: point.y,
				length: length * scale,
				geometry: "actual-svg",
			};
		},
		{ edgeId, end },
	);
	if (svgPoint) return svgPoint;
	const zoom = state.viewport.zoom;
	const [d] = getBezierPath({
		sourceX: 0,
		sourceY: 0,
		targetX: (target.x - source.x) / zoom,
		targetY: (target.y - source.y) / zoom,
		sourcePosition: reference ? Position.Bottom : Position.Right,
		targetPosition: reference ? Position.Top : Position.Left,
	});
	return page.evaluate(
		({ d, end, source, zoom }) => {
			const path = document.createElementNS(
				"http://www.w3.org/2000/svg",
				"path",
			);
			path.setAttribute("d", d);
			const length = path.getTotalLength();
			const distance =
				end === "source"
					? 18 / zoom
					: end === "target"
						? length - 18 / zoom
						: length / 2;
			const point = path.getPointAtLength(distance);
			return {
				x: source.x + point.x * zoom,
				y: source.y + point.y * zoom,
				length: length * zoom,
				geometry: "world-bezier",
			};
		},
		{ d, end, source, zoom },
	);
}
try {
	await load("adjacent");
	for (const zoom of process.env.BOARD_CONNECTION_CASES === "direct"
		? []
		: [1, 0.5, 2]) {
		await api("viewport", 20, 80, zoom);
		await page.waitForTimeout(250);
		for (const [first, second] of [
			["first", "second"],
			["second", "first"],
		]) {
			const from = await center("source.out");
			const a = await center(`target.${first}`);
			const b = await center(`target.${second}`);
			assert.ok(Math.abs(a.y - b.y) < 70, "Inputs are neighboring pin rows");
			await page.mouse.move(from.x, from.y);
			await page.mouse.down();
			await page.mouse.move(a.x, a.y, { steps: 12 });
			await highlight(`target.${first}`);
			await page.mouse.move(b.x, b.y, { steps: 5 });
			await highlight(`target.${second}`);
			await page.mouse.up();
			await expectEdges([`source.out->target.${second}`]);
			const saved = await snapshot();
			assert.deepEqual(
				saved.board.nodes.source.pins["source.out"].connected_to,
				[`target.${second}`],
			);
			assert.deepEqual(
				saved.board.nodes.target.pins[`target.${second}`].depends_on,
				["source.out"],
			);
			assert.deepEqual(
				saved.board.nodes.target.pins[`target.${first}`].depends_on,
				[],
			);
			results.push({
				name: `Adjacent ${first} to ${second} at zoom ${zoom}`,
				passed: true,
			});
			await undo([]);
		}
	}
	for (const family of ["string", "execution", "reference"]) {
		await load(family);
		const reference = family === "reference";
		const sourceHandle = reference ? "ref_out_source" : "source.out";
		const targetHandle = reference ? "ref_in_target" : "target.first";
		const initial = `${sourceHandle}->${targetHandle}`;
		await expectEdges([initial]);
		for (const zoom of [1, 0.5]) {
			await api("viewport", 20, 80, zoom);
			await page.waitForTimeout(300);
			for (const end of ["source", "target"]) {
				const before = await snapshot();
				assert.ok(
					before.edges.every((edge) => !edge.selected),
					"Wire is not selected before grabbing",
				);
				const grab = await pointOnWire(reference, end);
				const nextHandle = reference
					? end === "source"
						? "ref_out_source-alt"
						: "ref_in_target-alt"
					: end === "source"
						? "source-alt.out"
						: "target-alt.first";
				await page.mouse.move(grab.x, grab.y);
				await page.waitForTimeout(100);
				gesture = {
					family,
					zoom,
					end,
					grab,
					to: await center(nextHandle),
					hover: await page.evaluate(
						({ x, y }) => ({
							elements: document
								.elementsFromPoint(x, y)
								.slice(0, 6)
								.map((element) => ({
									tag: element.tagName,
									classes: element.getAttribute("class"),
									id: element.getAttribute("data-id"),
									role: element.getAttribute("role"),
								})),
							feedback: document.querySelector("[data-board-pin-hover]")
								?.textContent,
							overlay: window.__connectionOverlay,
						}),
						grab,
					),
				};
				await drag(grab, gesture.to);
				await expectEdges([
					end === "source"
						? `${nextHandle}->${targetHandle}`
						: `${sourceHandle}->${nextHandle}`,
				]);
				const saved = await snapshot();
				assert.deepEqual(
					Object.values(saved.board.nodes).map((node) => node.coordinates),
					Object.values(before.board.nodes).map((node) => node.coordinates),
					"Near-wire drag reconnects without moving a node",
				);
				assert.equal(
					saved.transactions.filter((tx) => tx.kind === "execute").length,
					before.transactions.filter((tx) => tx.kind === "execute").length + 1,
				);
				if (reference) {
					assert.deepEqual(
						saved.board.nodes.source.fn_refs.fn_refs,
						end === "source" ? [] : ["target-alt"],
					);
					assert.deepEqual(
						saved.board.nodes["source-alt"].fn_refs.fn_refs,
						end === "source" ? ["target"] : [],
					);
				} else {
					const source = end === "source" ? "source-alt" : "source";
					const target = end === "target" ? "target-alt" : "target";
					assert.deepEqual(
						saved.board.nodes[source].pins[`${source}.out`].connected_to,
						[`${target}.first`],
					);
					assert.deepEqual(
						saved.board.nodes[target].pins[`${target}.first`].depends_on,
						[`${source}.out`],
					);
				}
				results.push({
					name: `Unselected ${family} ${end} wire grab at zoom ${zoom}`,
					passed: true,
				});
				await undo([initial]);
			}
		}
		await api("viewport", 20, 80, 1);
		await page.waitForTimeout(250);
		const middle = await pointOnWire(reference, "middle");
		const beforeClick = await snapshot();
		await page.mouse.click(middle.x, middle.y);
		await page.waitForFunction(() =>
			window.__boardHost.snapshot().edges.some((edge) => edge.selected),
		);
		assert.equal(
			(await snapshot()).transactions.length,
			beforeClick.transactions.length,
		);
		results.push({
			name: `${family} wire midpoint still selects`,
			passed: true,
		});
		await page.mouse.click(1300, 700);
		await page.waitForFunction(() =>
			window.__boardHost.snapshot().edges.every((edge) => !edge.selected),
		);
		if (family !== "execution") {
			const alternate = reference ? "ref_in_target-alt" : "target-alt.first";
			await drag(await center(sourceHandle), await center(alternate));
			await expectEdges([initial, `${sourceHandle}->${alternate}`]);
			results.push({
				name: `${family} exact output pin creates another connection`,
				passed: true,
			});
		}
	}
	assert.deepEqual(errors, []);
	await page.screenshot({ path: `${output}/complete.png` });
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, results, errors }, null, 2),
	);
	console.log(JSON.stringify({ kind, results, errors }));
} catch (error) {
	await page.screenshot({ path: `${output}/failure.png` });
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{
				error: String(error),
				gesture,
				results,
				errors,
				state: await snapshot().catch(() => null),
				overlay: await page.evaluate(() => window.__connectionOverlay),
				dialogs: await page.locator('[role="dialog"]').allTextContents(),
			},
			null,
			2,
		),
	);
	throw error;
} finally {
	await browser.close();
}
