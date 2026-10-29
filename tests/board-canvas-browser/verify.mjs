import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import sharp from "sharp";

const options = {
	origin: process.env.BOARD_CANVAS_URL ?? "http://127.0.0.1:4338",
	browser: process.env.BOARD_CANVAS_BROWSER ?? "chromium",
	output:
		process.env.BOARD_CANVAS_OUTPUT ?? "tmp/board-canvas-browser/verification",
	headed: false,
};
for (let i = 2; i < process.argv.length; i++) {
	const [name, inline] = process.argv[i].replace(/^--/, "").split("=", 2);
	if (name === "headed") options.headed = inline !== "false";
	else if (["origin", "browser", "output"].includes(name)) {
		const value = inline ?? process.argv[++i];
		assert.ok(value, `Missing --${name} value`);
		options[name] = value;
	} else throw new Error(`Unknown option --${name}`);
}
assert.ok(
	["chromium", "webkit"].includes(options.browser),
	"Use chromium or webkit",
);
const output = resolve(options.output, options.browser);
await mkdir(output, { recursive: true });
const browser =
	options.browser === "webkit"
		? await webkit.launch({ headless: !options.headed })
		: await chromium.launch({
				headless: !options.headed,
				executablePath: process.env.CHROME_PATH ?? puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const page = await browser.newPage({
	viewport: { width: 1280, height: 960 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(10000);
const applePlatform = await page.evaluate(() =>
	/Mac|iPod|iPhone|iPad/.test(navigator.platform),
);
const additiveModifier = applePlatform ? "Meta" : "Control";
const errors = [];
const results = [];
const assets = new Set();
const startedAt = new Date().toISOString();
page.on("pageerror", (error) => errors.push(error.message));
page.on("response", (response) => {
	if (/\.(?:js|wasm)(?:\?|$)/.test(response.url())) assets.add(response.url());
});
const harness = (method, ...args) =>
	page.evaluate(
		({ method, args }) => {
			const api = window.__boardCanvasHarness;
			if (!api || typeof api[method] !== "function")
				throw new Error(`Missing harness method ${method}`);
			return api[method](...args);
		},
		{ method, args },
	);
const snapshot = () => harness("snapshot");
const settle = () =>
	page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
const pin = (node, direction) =>
	harness(
		"pin",
		node,
		direction.startsWith("ref_") ? direction : `${node}.${direction}`,
	);
const edgeId = (source, target) => `${source}.out->${target}.in`;
const graph = (nodes) => {
	const result = structuredClone(nodes);
	for (const node of Object.values(result)) {
		for (const pin of Object.values(node.pins)) {
			pin.connected_to.sort();
			pin.depends_on.sort();
		}
		node.fn_refs?.fn_refs.sort();
	}
	return result;
};
const waitCommits = (count) =>
	page.waitForFunction(
		(expected) => window.__boardCanvasHarness.snapshot().commits === expected,
		count,
	);
async function waitForNativePixels() {
	const started = Date.now();
	let attempts = 0;
	let visible = [];
	while (Date.now() - started < 5000) {
		const points = await Promise.all([
			pin("string-a", "in"),
			pin("integer-a", "in"),
			pin("exec-a", "in"),
		]);
		const screenshot = await page.screenshot();
		if (!attempts) {
			await writeFile(`${output}/startup-first.png`, screenshot);
			await writeFile(`${output}/initial.png`, screenshot);
		}
		const { data, info } = await sharp(screenshot)
			.ensureAlpha()
			.raw()
			.toBuffer({ resolveWithObject: true });
		const count = (left, top, width, height, predicate) => {
			let result = 0;
			for (
				let y = Math.max(0, Math.floor(top));
				y < Math.min(info.height, top + height);
				y++
			) {
				for (
					let x = Math.max(0, Math.floor(left));
					x < Math.min(info.width, left + width);
					x++
				) {
					const offset = (y * info.width + x) * 4;
					if (predicate(data[offset], data[offset + 1], data[offset + 2]))
						result++;
				}
			}
			return result;
		};
		visible = points.map((point, index) => ({
			pinPixels: count(point.x - 7, point.y - 7, 14, 14, (r, g, b) =>
				index < 2
					? Math.max(r, g, b) > 110 &&
						Math.max(r, g, b) - Math.min(r, g, b) > 60
					: Math.min(r, g, b) > 150,
			),
			labelPixels: count(
				point.x + 8,
				point.y - 25,
				100,
				16,
				(r, g, b) => Math.min(r, g, b) > 140,
			),
		}));
		attempts++;
		if (
			visible.every(
				(sample) => sample.pinPixels >= 4 && sample.labelPixels >= 8,
			)
		) {
			await writeFile(`${output}/initial.png`, screenshot);
			results.push({
				name: "idle startup paints native pins and labels before interaction",
				passed: true,
				attempts,
				elapsedMs: Date.now() - started,
				samples: visible,
			});
			console.log(
				`PASS idle native pixels (${attempts} screenshots, ${Date.now() - started} ms)`,
			);
			return;
		}
		await page.waitForTimeout(50);
	}
	results.push({
		name: "idle startup paints native pins and labels before interaction",
		passed: false,
		attempts,
		samples: visible,
	});
	throw new Error(
		`Native graph stayed blank without input: ${JSON.stringify(visible)}`,
	);
}
async function drag(from, to, { steps = 8 } = {}) {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps });
	await page.mouse.up();
	await settle();
}
async function selectEdge(id) {
	const point = await harness("edge", id);
	await page.mouse.click(point.x, point.y);
	await page.waitForFunction(
		(expected) =>
			window.__boardCanvasHarness.snapshot().selectedEdges.includes(expected),
		id,
	);
	await settle();
}
async function reset() {
	await harness("reset");
	await harness("setViewport", 40, 20, 1);
	await page.waitForFunction(
		() => window.__boardCanvasHarness?.ready(),
		undefined,
		{ timeout: 60000 },
	);
	await settle();
	assert.deepEqual((await snapshot()).history, { undo: 0, redo: 0 });
}
function assertWire(state, source, target, connected = true) {
	const output = state.nodes[source].pins[`${source}.out`];
	const input = state.nodes[target].pins[`${target}.in`];
	assert.equal(
		output.connected_to.includes(input.id),
		connected,
		`${source} output adjacency`,
	);
	assert.equal(
		input.depends_on.includes(output.id),
		connected,
		`${target} input adjacency`,
	);
}
async function undoTo(before) {
	assert.equal(
		(await snapshot()).history.undo,
		1,
		"A pointer gesture creates one undo entry",
	);
	await harness("undo");
	await settle();
	const after = await snapshot();
	assert.deepEqual(
		graph(after.nodes),
		graph(before.nodes),
		"Undo restores authoritative graph",
	);
	assert.deepEqual(after.history, { undo: 0, redo: 1 });
}
async function scenario(name, run) {
	await reset();
	const before = await snapshot();
	try {
		await run(before);
		assert.equal(
			await harness("ready"),
			true,
			"The native renderer remains active after the scenario",
		);
		assert.deepEqual(errors, [], "The page reported an uncaught error");
		results.push({ name, passed: true });
		console.log(`PASS ${name}`);
	} catch (error) {
		results.push({ name, passed: false, error: error.message });
		await page.screenshot({ path: `${output}/failure.png`, fullPage: true });
		await writeFile(
			`${output}/failure-state.json`,
			JSON.stringify(await snapshot(), null, 2),
		);
		throw error;
	}
}

try {
	await page.goto(`${options.origin}/?boardRenderer=egui`, {
		waitUntil: "networkidle",
	});
	await page.waitForFunction(
		() => window.__boardCanvasHarness?.ready(),
		undefined,
		{ timeout: 60000 },
	);
	await waitForNativePixels();
	assert.equal(
		await page.locator(".react-flow__renderer").count(),
		0,
		"WASM mode must not mount the React Flow renderer",
	);
	assert.equal(
		await page.locator("[data-standalone-board]").count(),
		1,
		"WASM mode owns its viewport and interactions",
	);
	await scenario(
		"box selection preserves complete node bounds and incident edges",
		async (before) => {
			const bounds = await page
				.locator('.react-flow__node[data-id="string-a"]')
				.boundingBox();
			assert.ok(bounds);
			await page.mouse.move(bounds.x - 18, bounds.y - 18);
			await page.keyboard.down("Shift");
			await page.mouse.down();
			await page.mouse.move(
				bounds.x + bounds.width + 18,
				bounds.y + bounds.height + 18,
				{ steps: 5 },
			);
			await page.mouse.up();
			await page.keyboard.up("Shift");
			await settle();
			const after = await snapshot();
			assert.deepEqual(after.selectedNodes, ["string-a"]);
			assert.ok(after.selectedEdges.includes(edgeId("string-a", "string-b")));
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
		},
	);
	await scenario(
		"additive wire clicks toggle selection without changing topology",
		async (before) => {
			await selectEdge(edgeId("string-a", "string-b"));
			const point = await harness("edge", edgeId("string-a", "string-b"));
			await page.keyboard.down(additiveModifier);
			await page.mouse.click(point.x, point.y);
			await page.keyboard.up(additiveModifier);
			await settle();
			assert.deepEqual((await snapshot()).selectedEdges, []);
			assert.deepEqual(graph((await snapshot()).nodes), graph(before.nodes));
		},
	);
	if (options.browser === "webkit" && applePlatform) {
		await scenario(
			"macOS Control-click reaches the existing context menu without a native gesture",
			async (before) => {
				await page.evaluate(() => {
					window.__contextMenuCheck = { pointerdowns: 0, menus: 0 };
					window.addEventListener(
						"pointerdown",
						() => {
							window.__contextMenuCheck.pointerdowns++;
						},
						{ once: true },
					);
					window.addEventListener(
						"contextmenu",
						(event) => {
							event.preventDefault();
							window.__contextMenuCheck.menus++;
						},
						{ once: true },
					);
				});
				const point = await harness("node", "integer-a");
				await page.keyboard.down("Control");
				await page.mouse.click(point.x, point.y);
				await page.keyboard.up("Control");
				await settle();
				assert.deepEqual(await page.evaluate(() => window.__contextMenuCheck), {
					pointerdowns: 1,
					menus: 1,
				});
				const after = await snapshot();
				assert.deepEqual(graph(after.nodes), graph(before.nodes));
				assert.deepEqual(after.history, before.history);
				assert.equal(after.nativeConnections, before.nativeConnections);
			},
		);
	}
	await scenario(
		"multi-node selection drags preserve group offsets in one undo transaction",
		async (before) => {
			const first = await harness("node", "integer-a");
			const second = await harness("node", "integer-b");
			await page.mouse.click(first.x, first.y);
			await settle();
			assert.deepEqual(
				(await snapshot()).selectedNodes,
				["integer-a"],
				"The first node click selects its body",
			);
			await page.keyboard.down(additiveModifier);
			await page.mouse.click(second.x, second.y);
			await page.keyboard.up(additiveModifier);
			await settle();
			assert.deepEqual((await snapshot()).selectedNodes.sort(), [
				"integer-a",
				"integer-b",
			]);
			await drag(first, { x: first.x + 47, y: first.y + 21 });
			await waitCommits(1);
			const after = await snapshot();
			for (const id of ["integer-a", "integer-b"])
				assert.deepEqual(after.nodes[id].coordinates, [
					before.nodes[id].coordinates[0] + 47,
					before.nodes[id].coordinates[1] + 21,
					0,
				]);
			await undoTo(before);
		},
	);
	await scenario(
		"a foreground DOM item prevents snapping to a covered pin",
		async (before) => {
			const from = await pin("integer-a", "out");
			const to = await pin("integer-b", "in");
			await page.evaluate(({ x, y }) => {
				const blocker = document.createElement("div");
				blocker.id = "pin-occlusion-fixture";
				blocker.className = "react-flow__node";
				blocker.dataset.id = "foreground-dom-item";
				Object.assign(blocker.style, {
					position: "fixed",
					left: `${x - 30}px`,
					top: `${y - 30}px`,
					width: "60px",
					height: "60px",
					zIndex: "9999",
					background: "var(--card)",
					pointerEvents: "auto",
				});
				document.body.append(blocker);
			}, to);
			try {
				await drag(from, to);
				const after = await snapshot();
				assert.deepEqual(graph(after.nodes), graph(before.nodes));
				assert.deepEqual(after.history, before.history);
				assert.equal(after.paletteDrops, 0);
			} finally {
				await page.evaluate(() =>
					document.getElementById("pin-occlusion-fixture")?.remove(),
				);
			}
		},
	);
	await scenario(
		"additive drag adds a new node and moves the selected group",
		async (before) => {
			const first = await harness("node", "integer-a");
			const second = await harness("node", "integer-b");
			await page.mouse.click(first.x, first.y);
			await page.keyboard.down(additiveModifier);
			await drag(second, { x: second.x + 47, y: second.y + 21 });
			await page.keyboard.up(additiveModifier);
			await waitCommits(1);
			const after = await snapshot();
			for (const id of ["integer-a", "integer-b"])
				assert.deepEqual(after.nodes[id].coordinates, [
					before.nodes[id].coordinates[0] + 47,
					before.nodes[id].coordinates[1] + 21,
					0,
				]);
			await undoTo(before);
		},
	);
	for (const cancel of [false, true]) {
		await scenario(
			`box-selected group drag ${cancel ? "cancels on Escape" : "commits one undo batch"}`,
			async (before) => {
				const first = await page
					.locator('.react-flow__node[data-id="integer-a"]')
					.boundingBox();
				const second = await page
					.locator('.react-flow__node[data-id="integer-b"]')
					.boundingBox();
				assert.ok(first && second);
				await page.mouse.move(first.x - 18, first.y - 18);
				await page.keyboard.down("Shift");
				await page.mouse.down();
				await page.mouse.move(
					second.x + second.width + 18,
					second.y + second.height + 18,
					{ steps: 5 },
				);
				await page.mouse.up();
				await page.keyboard.up("Shift");
				await settle();
				assert.deepEqual((await snapshot()).selectedNodes.sort(), [
					"integer-a",
					"integer-b",
				]);
				const start = await harness("node", "integer-a");
				await page.mouse.move(start.x, start.y);
				await page.mouse.down();
				await page.mouse.move(start.x + 47, start.y + 21, { steps: 5 });
				await settle();
				if (cancel) await page.keyboard.press("Escape");
				await page.mouse.up();
				await settle();
				if (cancel) {
					const after = await snapshot();
					assert.deepEqual(graph(after.nodes), graph(before.nodes));
					assert.deepEqual(after.history, before.history);
					assert.equal(after.cancelledDrags, 1);
				} else {
					await waitCommits(1);
					const after = await snapshot();
					for (const id of ["integer-a", "integer-b"])
						assert.deepEqual(after.nodes[id].coordinates, [
							before.nodes[id].coordinates[0] + 47,
							before.nodes[id].coordinates[1] + 21,
							0,
						]);
					await undoTo(before);
				}
			},
		);
	}
	await scenario("data input replacement and undo/redo", async (before) => {
		await drag(await pin("string-c", "out"), await pin("string-b", "in"));
		await waitCommits(1);
		const saved = await snapshot();
		assertWire(saved, "string-c", "string-b");
		assertWire(saved, "string-a", "string-b", false);
		await undoTo(before);
		await harness("redo");
		await settle();
		assert.deepEqual(graph((await snapshot()).nodes), graph(saved.nodes));
	});
	await scenario("backward input drag normalizes endpoints", async (before) => {
		await drag(await pin("integer-b", "in"), await pin("integer-a", "out"));
		await waitCommits(1);
		assertWire(await snapshot(), "integer-a", "integer-b");
		await undoTo(before);
	});
	await scenario(
		"invalid type drops preserve graph and history",
		async (before) => {
			await drag(await pin("string-a", "out"), await pin("integer-b", "in"));
			const after = await snapshot();
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
			assert.equal(after.paletteDrops, 0);
		},
	);
	await scenario(
		"target reconnect changes the authoritative wire",
		async (before) => {
			await selectEdge(edgeId("string-a", "string-b"));
			await drag(await pin("string-b", "in"), await pin("string-c", "in"));
			await waitCommits(1);
			assertWire(await snapshot(), "string-a", "string-c");
			assertWire(await snapshot(), "string-a", "string-b", false);
			await undoTo(before);
		},
	);
	await scenario(
		"source reconnect changes the authoritative wire",
		async (before) => {
			await selectEdge(edgeId("string-a", "string-b"));
			await drag(await pin("string-a", "out"), await pin("string-c", "out"));
			await waitCommits(1);
			assertWire(await snapshot(), "string-c", "string-b");
			assertWire(await snapshot(), "string-a", "string-b", false);
			await undoTo(before);
		},
	);
	await scenario(
		"execution output replacement restores displaced wire on undo",
		async (before) => {
			await drag(await pin("exec-a", "out"), await pin("exec-c", "in"));
			await waitCommits(1);
			assertWire(await snapshot(), "exec-a", "exec-c");
			assertWire(await snapshot(), "exec-a", "exec-b", false);
			await undoTo(before);
		},
	);
	await scenario("unchanged reconnect is not a command", async (before) => {
		await selectEdge(edgeId("exec-a", "exec-b"));
		const point = await pin("exec-b", "in");
		await drag(point, point);
		const after = await snapshot();
		assert.deepEqual(graph(after.nodes), graph(before.nodes));
		assert.deepEqual(after.history, before.history);
		assert.equal(after.commits, 0);
	});
	await scenario(
		"invalid reconnect drop does not disconnect",
		async (before) => {
			await selectEdge(edgeId("string-a", "string-b"));
			await drag(await pin("string-b", "in"), await pin("integer-b", "in"));
			const after = await snapshot();
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
			assert.equal(after.paletteDrops, 0);
		},
	);
	await scenario(
		"empty reconnect drop disconnects and undoes",
		async (before) => {
			await selectEdge(edgeId("string-a", "string-b"));
			await drag(await pin("string-b", "in"), { x: 1100, y: 800 });
			await waitCommits(1);
			assertWire(await snapshot(), "string-a", "string-b", false);
			assert.equal((await snapshot()).paletteDrops, 0);
			await undoTo(before);
		},
	);
	await scenario(
		"empty new-wire drop opens the palette without a command",
		async (before) => {
			await drag(await pin("integer-a", "out"), { x: 1100, y: 800 });
			const after = await snapshot();
			assert.equal(after.paletteDrops, 1);
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
		},
	);
	await scenario("clicking two pins connects nothing", async (before) => {
		const from = await pin("integer-a", "out");
		const to = await pin("integer-b", "in");
		await page.mouse.click(from.x, from.y);
		await settle();
		await page.mouse.click(to.x, to.y);
		await settle();
		const after = await snapshot();
		assert.deepEqual(graph(after.nodes), graph(before.nodes));
		assert.deepEqual(after.history, before.history);
		assert.equal(after.commits, 0);
	});
	await scenario(
		"near-pin snapping remains usable when zoomed out",
		async (before) => {
			await harness("setViewport", 40, 20, 0.5);
			await settle();
			const to = await pin("integer-b", "in");
			await drag(await pin("integer-a", "out"), { x: to.x, y: to.y + 18 });
			await waitCommits(1);
			const saved = await snapshot();
			assertWire(saved, "integer-a", "integer-b");
			assert.equal(saved.nativeConnections, 1);
			assert.equal(saved.legacyConnections, 0);
			await undoTo(before);
		},
	);
	await scenario(
		"selected DOM handle uses the native transaction path",
		async (before) => {
			const point = await harness("node", "integer-a");
			await page.mouse.click(point.x, point.y);
			await page.waitForFunction(() =>
				window.__boardCanvasHarness
					.snapshot()
					.selectedNodes.includes("integer-a"),
			);
			await settle();
			const handle = page.locator(
				'.react-flow__node[data-id="integer-a"] .react-flow__handle[data-handleid="integer-a.out"]',
			);
			const bounds = await handle.boundingBox();
			assert.ok(bounds, "Selected DOM output handle is mounted");
			await drag(
				{ x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
				await pin("integer-b", "in"),
			);
			await waitCommits(1);
			const saved = await snapshot();
			assertWire(saved, "integer-a", "integer-b");
			assert.equal(saved.nativeConnections, 1);
			assert.equal(saved.legacyConnections, 0);
			await undoTo(before);
		},
	);
	await scenario(
		"function-reference handles update refs and undo",
		async (before) => {
			await drag(
				await pin("ref-a", "ref_out_ref-a"),
				await pin("ref-b", "ref_in_ref-b"),
			);
			await waitCommits(1);
			assert.deepEqual((await snapshot()).nodes["ref-a"].fn_refs.fn_refs, [
				"ref-b",
			]);
			await undoTo(before);
		},
	);
	await scenario(
		"selected DOM handle clicks start no legacy gesture",
		async (before) => {
			const node = await harness("node", "integer-a");
			await page.mouse.click(node.x, node.y);
			await settle();
			const handle = page.locator(
				'.react-flow__node[data-id="integer-a"] .react-flow__handle[data-handleid="integer-a.out"]',
			);
			const bounds = await handle.boundingBox();
			assert.ok(bounds);
			await page.mouse.click(
				bounds.x + bounds.width / 2,
				bounds.y + bounds.height / 2,
			);
			const target = await pin("integer-b", "in");
			await page.mouse.click(target.x, target.y);
			await settle();
			const after = await snapshot();
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.equal(after.nativeConnections, 0);
			assert.equal(after.legacyConnections, 0);
		},
	);
	await scenario(
		"rapid release commits its final node position",
		async (before) => {
			const from = await harness("node", "integer-a");
			await drag(from, { x: from.x + 70, y: from.y + 35 }, { steps: 1 });
			const after = await snapshot();
			assert.deepEqual(after.nodes["integer-a"].coordinates, [
				before.nodes["integer-a"].coordinates[0] + 70,
				before.nodes["integer-a"].coordinates[1] + 35,
				0,
			]);
			await undoTo(before);
		},
	);
	await scenario("Escape cancels an active connection", async (before) => {
		const from = await pin("integer-a", "out");
		const to = await pin("integer-b", "in");
		await page.mouse.move(from.x, from.y);
		await page.mouse.down();
		await page.mouse.move(to.x, to.y, { steps: 4 });
		await page.keyboard.press("Escape");
		await page.mouse.up();
		await settle();
		const after = await snapshot();
		assert.deepEqual(graph(after.nodes), graph(before.nodes));
		assert.deepEqual(after.history, before.history);
	});
	await scenario("selected DOM controls remain clickable", async (before) => {
		const point = await harness("node", "integer-a");
		await page.mouse.click(point.x, point.y);
		const button = page.locator(
			'.react-flow__node[data-id="integer-a"] button',
		);
		const previousClicks = Number(
			(await button.textContent())?.split(" ").at(-1),
		);
		await button.click();
		assert.equal(
			await button.textContent(),
			`DOM control ${previousClicks + 1}`,
		);
		const after = await snapshot();
		assert.deepEqual(graph(after.nodes), graph(before.nodes));
		assert.deepEqual(after.history, before.history);
	});
	await scenario(
		"a pin editor nested inside a Handle receives its own click",
		async (before) => {
			const point = await harness("node", "integer-a");
			await page.mouse.click(point.x, point.y);
			const editor = page
				.locator('.react-flow__node[data-id="integer-a"]')
				.getByTestId("harness-pin-editor");
			await editor.click();
			assert.equal(await editor.textContent(), "Pin editor 1");
			const after = await snapshot();
			assert.equal(after.nativeConnections, 0);
			assert.equal(after.legacyConnections, 0);
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
		},
	);
	for (const additive of [false, true]) {
		await scenario(
			`dragging stays unsnapped ${additive ? "with the additive modifier" : "without a modifier"}`,
			async (before) => {
				const from = await harness("node", "integer-a");
				await page.mouse.click(from.x, from.y);
				if (additive) await page.keyboard.down(additiveModifier);
				await drag(from, { x: from.x + 37, y: from.y + 27 });
				if (additive) await page.keyboard.up(additiveModifier);
				await waitCommits(1);
				const expected = [
					before.nodes["integer-a"].coordinates[0] + 37,
					before.nodes["integer-a"].coordinates[1] + 27,
				];
				assert.deepEqual((await snapshot()).nodes["integer-a"].coordinates, [
					...expected,
					0,
				]);
				await undoTo(before);
			},
		);
	}
	await scenario(
		"Shift during an unsnapped drag locks its original axis",
		async (before) => {
			const from = await harness("node", "integer-a");
			await page.mouse.move(from.x, from.y);
			await page.mouse.down();
			await page.mouse.move(from.x + 10, from.y + 2);
			await page.keyboard.down("Shift");
			await page.mouse.move(from.x + 37, from.y + 9, { steps: 3 });
			await page.mouse.up();
			await page.keyboard.up("Shift");
			await waitCommits(1);
			const original = before.nodes["integer-a"].coordinates;
			assert.deepEqual((await snapshot()).nodes["integer-a"].coordinates, [
				original[0] + 37,
				original[1],
				0,
			]);
			await undoTo(before);
		},
	);
	await scenario(
		"Escape restores node position and ends the drag without a command",
		async (before) => {
			const from = await harness("node", "integer-a");
			await page.mouse.move(from.x, from.y);
			await page.mouse.down();
			await page.mouse.move(from.x + 70, from.y + 35, { steps: 4 });
			await settle();
			await page.keyboard.press("Escape");
			await page.mouse.up();
			await settle();
			const after = await snapshot();
			assert.equal(after.cancelledDrags, 1);
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
			const restored = await harness("node", "integer-a");
			assert.ok(
				Math.abs(restored.x - from.x) < 2 && Math.abs(restored.y - from.y) < 2,
				"Transient node position is restored",
			);
			const button = page.locator(
				'.react-flow__node[data-id="integer-a"] button',
			);
			const previousClicks = Number(
				(await button.textContent())?.split(" ").at(-1),
			);
			await button.click();
			assert.equal(
				await button.textContent(),
				`DOM control ${previousClicks + 1}`,
			);
		},
	);
	await scenario(
		"a pending commit blocks selected-handle fallback gestures",
		async (before) => {
			const selected = await harness("node", "integer-a");
			await page.mouse.click(selected.x, selected.y);
			await settle();
			await harness("setCommitDelay", 500);
			await drag(await pin("string-c", "out"), await pin("string-b", "in"));
			assert.equal((await snapshot()).pending, true);
			const handle = page.locator(
				'.react-flow__node[data-id="integer-a"] .react-flow__handle[data-handleid="integer-a.out"]',
			);
			const bounds = await handle.boundingBox();
			assert.ok(bounds, "Selected output is still mounted during save");
			await drag(
				{ x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2 },
				await pin("integer-b", "in"),
			);
			await waitCommits(1);
			await page.waitForFunction(
				() => !window.__boardCanvasHarness.snapshot().pending,
			);
			await settle();
			const after = await snapshot();
			assertWire(after, "string-c", "string-b");
			assertWire(after, "integer-a", "integer-b", false);
			assert.equal(after.nativeConnections, 1);
			assert.equal(after.legacyConnections, 0);
			await undoTo(before);
		},
	);
	await scenario(
		"read-only transition cancels an in-flight connection",
		async (before) => {
			const from = await pin("integer-a", "out");
			const to = await pin("integer-b", "in");
			await page.mouse.move(from.x, from.y);
			await page.mouse.down();
			await page.mouse.move(to.x, to.y, { steps: 4 });
			await harness("setReadOnly", true);
			await settle();
			await page.mouse.up();
			await settle();
			const after = await snapshot();
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
			assert.equal(after.paletteDrops, 0);
		},
	);
	await scenario(
		"scope change cancels an in-flight connection",
		async (before) => {
			const from = await pin("integer-a", "out");
			const to = await pin("integer-b", "in");
			await page.mouse.move(from.x, from.y);
			await page.mouse.down();
			await page.mouse.move(to.x, to.y, { steps: 4 });
			await harness("changeScope");
			await settle();
			await page.mouse.up();
			await page.waitForFunction(
				() => window.__boardCanvasHarness?.ready(),
				undefined,
				{ timeout: 60000 },
			);
			const after = await snapshot();
			assert.deepEqual(graph(after.nodes), graph(before.nodes));
			assert.deepEqual(after.history, before.history);
			assert.equal(after.paletteDrops, 0);
		},
	);
	await scenario(
		"the lock control blocks native edits and unlocking restores them",
		async (before) => {
			const control = page.locator(".react-flow__controls-interactive");
			await control.click();
			await settle();
			await drag(await pin("integer-a", "out"), await pin("integer-b", "in"));
			const from = await harness("node", "integer-a");
			await drag(from, { x: from.x + 70, y: from.y + 35 });
			const locked = await snapshot();
			assert.deepEqual(graph(locked.nodes), graph(before.nodes));
			assert.deepEqual(locked.history, before.history);
			assert.equal(locked.paletteDrops, 0);
			await control.click();
			await harness("setViewport", 40, 20, 1);
			await settle();
			await drag(await pin("integer-a", "out"), await pin("integer-b", "in"));
			await waitCommits(1);
			assertWire(await snapshot(), "integer-a", "integer-b");
			await undoTo(before);
		},
	);
	await scenario(
		"screenshot capture waits for DOM rendering and restores egui",
		async (before) => {
			await page.getByRole("button", { name: "Capture screenshot" }).click();
			await page.waitForFunction(
				() =>
					window.__boardCanvasHarness.snapshot().screenshot?.asyncComplete ===
					true,
			);
			await page.waitForFunction(() => window.__boardCanvasHarness.ready());
			const saved = await snapshot();
			assert.equal(saved.screenshot.nodes, 10);
			assert.ok(saved.screenshot.edges > 0);
			assert.equal(saved.screenshot.nativeCanvases, 0);
			assert.deepEqual(graph(saved.nodes), graph(before.nodes));
			assert.deepEqual(saved.history, before.history);
		},
	);
	await settle();
	await page.screenshot({ path: `${output}/complete.png`, fullPage: true });
} finally {
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{
				browser: options.browser,
				origin: options.origin,
				startedAt,
				assets: [...assets],
				results,
				errors,
			},
			null,
			2,
		),
	);
	await browser.close();
	console.log(
		`Browser closed; ${results.filter((result) => result.passed).length}/${results.length} scenarios passed. Artifacts: ${output}`,
	);
}
