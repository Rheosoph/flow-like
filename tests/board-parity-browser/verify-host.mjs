import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import { setFixtureRenderer } from "../board-canvas-browser/renderer-settings-driver.mjs";

const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output = `tmp/board-parity-browser/host/${kind}`;
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
const externalRequests = [];
const results = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.route("**/*", async (route) => {
	const url = new URL(route.request().url());
	if (
		!["127.0.0.1", "localhost"].includes(url.hostname) &&
		!["data:", "blob:"].includes(url.protocol)
	) {
		externalRequests.push(url.href);
		return route.abort();
	}
	await route.continue();
});
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = () => api("snapshot");
const node = (id) => page.locator(`.react-flow__node[data-id="${id}"]`);
const edge = (id) => page.locator(`.react-flow__edge[data-id="${id}"]`);
const test = async (name, run) => {
	await run();
	assert.equal(
		await page
			.getByPlaceholder("Search nodes and actions", { exact: true })
			.isVisible(),
		false,
		`The node palette must stay closed after ${name}`,
	);
	results.push(name);
	console.log("PASS", name);
};
const waitFor = (predicate, arg) => page.waitForFunction(predicate, arg);
const center = async (locator) => {
	const box = await locator.boundingBox();
	assert.ok(box, "The actual interaction target must be visible");
	return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
};
const graphNodePoint = async (id) => {
	const state = await snapshot();
	const target = state.nodes.find((entry) => entry.id === id);
	const root = await page.locator(".react-flow").boundingBox();
	assert.ok(target && root);
	return {
		x:
			root.x +
			state.viewport.x +
			(target.position.x + 60) * state.viewport.zoom,
		y:
			root.y +
			state.viewport.y +
			(target.position.y + 12) * state.viewport.zoom,
	};
};
const assertViewport = (actual, expected) => {
	for (const key of ["x", "y", "zoom"])
		assert.ok(
			Math.abs(actual[key] - expected[key]) < 0.001,
			`Renderer switch preserves camera ${key}: ${actual[key]} versus ${expected[key]}`,
		);
};
const drag = async (from, to) => {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps: 15 });
	await page.mouse.up();
};
const reconnectTarget = async (edgeId, targetNode, targetPin) => {
	const from = await center(
		edge(edgeId).locator(".react-flow__edgeupdater-target"),
	);
	const to = await center(
		node(targetNode).locator(`[data-handleid="${targetPin}"]`),
	);
	await drag(from, to);
};
const undo = async () => {
	assert.equal(
		await page
			.getByPlaceholder("Search nodes and actions", { exact: true })
			.isVisible(),
		false,
		"Reconnection must not open a palette that steals history shortcuts",
	);
	await node("native").focus();
	const count = (await snapshot()).transactions.length;
	await page.keyboard.press("ControlOrMeta+z");
	await waitFor((count) => {
		const transactions = window.__boardHost.snapshot().transactions;
		return transactions.length === count + 1 && transactions.at(-1).finished;
	}, count);
	assert.equal((await snapshot()).transactions.at(-1).kind, "undo");
	await page.waitForTimeout(150);
};

try {
	await page.goto("http://127.0.0.1:4339/host?boardRenderer=egui", {
		timeout: 180000,
	});
	await waitFor(() => !!window.__boardHost);
	await waitFor(
		() =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === "egui",
	);
	await api("viewport", 20, 20, 1);
	await page.waitForTimeout(350);
	await test("actual FlowBoard mounts with local backend and both native paint planes", async () => {
		assert.equal(await page.getByTestId("actual-flow-board-host").count(), 1);
		const state = await snapshot();
		assert.ok(
			state.reads.getCatalog && state.reads.getBoard && state.reads.getApp,
		);
		assert.equal(state.nodes.length, 16);
		assert.equal(state.edges.length, 5);
		assert.equal(
			await page.locator('[data-testid="egui-board-canvas"]').count(),
			1,
		);
		assert.equal(
			await page.locator('[data-testid="egui-board-edges"]').count(),
			1,
		);
		await page.screenshot({ path: `${output}/initial.png` });
	});
	await test("native keyboard movement commits one host MoveNode transaction", async () => {
		const point = await graphNodePoint("native");
		await page.mouse.click(point.x, point.y);
		await waitFor(
			() =>
				window.__boardHost.snapshot().nodes.find((n) => n.id === "native")
					?.selected,
		);
		await node("native").focus();
		const before = await snapshot();
		await page.keyboard.press("ArrowRight");
		await waitFor(
			(x) =>
				window.__boardHost.snapshot().board.nodes.native.coordinates[0] ===
				x + 5,
			before.board.nodes.native.coordinates[0],
		);
		await waitFor(() => window.__boardHost.snapshot().history.canUndo);
		const after = await snapshot();
		assert.equal(after.transactions.length, before.transactions.length + 1);
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["MoveNode"],
		);
		assert.equal(
			after.nodes.find((n) => n.id === "native").position.x,
			before.board.nodes.native.coordinates[0] + 5,
		);
	});
	await test("host history undo and redo restore keyboard movement", async () => {
		const x = (await snapshot()).board.nodes.native.coordinates[0];
		await undo();
		assert.equal((await snapshot()).board.nodes.native.coordinates[0], x - 5);
		await page.keyboard.press("ControlOrMeta+Shift+z");
		await waitFor(
			(x) =>
				window.__boardHost.snapshot().board.nodes.native.coordinates[0] === x,
			x,
		);
		assert.equal((await snapshot()).transactions.at(-1).kind, "redo");
		await undo();
	});
	await test("a drag from DOM board chrome does not select page text", async () => {
		const point = await graphNodePoint("exec-b");
		await page.mouse.move(point.x, point.y);
		const separator = node("exec-b")
			.locator("[data-board-dom-control] .w-px")
			.first();
		await separator.waitFor();
		await page.evaluate(() => getSelection()?.removeAllRanges());
		await drag(
			await center(separator),
			await center(page.locator(".parity-bar")),
		);
		assert.notEqual(
			await page.evaluate(() => getSelection()?.type),
			"Range",
			`The toolbar drag selected "${await page.evaluate(() => getSelection()?.toString())}"`,
		);
		await page.mouse.move(point.x, point.y + 200);
	});
	await test("native and legacy switching preserves the existing viewport", async () => {
		const expected = { x: 137, y: -82, zoom: 0.73 };
		await api("viewport", expected.x, expected.y, expected.zoom);
		await setFixtureRenderer(page, "react-flow");
		await page.waitForTimeout(300);
		assertViewport((await snapshot()).viewport, expected);
		await setFixtureRenderer(page, "egui");
		await page.waitForTimeout(300);
		assertViewport((await snapshot()).viewport, expected);
		await setFixtureRenderer(page, "react-flow");
	});
	await waitFor(
		() =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === "react-flow",
	);
	await api("viewport", 20, 20, 1);
	await page.waitForTimeout(250);
	await test("React Flow keyboard movement uses the same authoritative transaction", async () => {
		await node("native").focus();
		const before = await snapshot();
		await page.keyboard.press("ArrowDown");
		await waitFor(
			(y) =>
				window.__boardHost.snapshot().board.nodes.native.coordinates[1] ===
				y + 5,
			before.board.nodes.native.coordinates[1],
		);
		assert.equal(
			(await snapshot()).transactions.length,
			before.transactions.length + 1,
		);
		await undo();
	});
	await api("setDelay", 600);
	const oldEdge = "string-a.out-string-b.in";
	await test("delayed SVG reconnect commits one batch without a reconnect-end delete race", async () => {
		const before = await snapshot();
		await reconnectTarget(oldEdge, "string-c", "string-c.in");
		await waitFor(
			(n) => window.__boardHost.snapshot().transactions.length > n,
			before.transactions.length,
		);
		const pending = await snapshot();
		assert.equal(pending.transactions.length, before.transactions.length + 1);
		assert.equal(
			pending.transactions.at(-1).finished,
			undefined,
			"The callback must still be pending after pointerup",
		);
		assert.deepEqual(
			pending.board.nodes["string-a"].pins["string-a.out"].connected_to,
			["string-b.in"],
		);
		await page.waitForTimeout(1100);
		const after = await snapshot();
		assert.equal(
			after.transactions.length,
			before.transactions.length + 1,
			"Reconnect-end must not queue a second disconnect",
		);
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["DisconnectPin", "ConnectPin"],
		);
		assert.deepEqual(
			after.board.nodes["string-a"].pins["string-a.out"].connected_to,
			["string-c.in"],
		);
		assert.ok(after.edges.some((e) => e.id === "string-a.out-string-c.in"));
		await undo();
		assert.deepEqual(
			(await snapshot()).board.nodes["string-a"].pins["string-a.out"]
				.connected_to,
			["string-b.in"],
		);
	});
	await test("unchanged SVG reconnect does not write or remove the wire", async () => {
		const before = await snapshot();
		await reconnectTarget(oldEdge, "string-b", "string-b.in");
		await page.waitForTimeout(850);
		const after = await snapshot();
		assert.equal(after.transactions.length, before.transactions.length);
		assert.deepEqual(
			after.board.nodes["string-a"].pins["string-a.out"].connected_to,
			["string-b.in"],
		);
	});
	await test("invalid SVG reconnect preserves the original wire", async () => {
		const before = await snapshot();
		await reconnectTarget(oldEdge, "integer-b", "integer-b.in");
		await page.waitForTimeout(850);
		const after = await snapshot();
		assert.equal(after.transactions.length, before.transactions.length);
		assert.deepEqual(
			after.board.nodes["string-a"].pins["string-a.out"].connected_to,
			["string-b.in"],
		);
	});
	await test("SVG reconnect released on the pane disconnects once and remains undoable", async () => {
		const before = await snapshot();
		const from = await center(
			edge(oldEdge).locator(".react-flow__edgeupdater-target"),
		);
		const pane = await page.locator(".react-flow__pane").boundingBox();
		assert.ok(pane);
		await drag(from, { x: pane.x + 900, y: pane.y + 720 });
		await waitFor(
			() =>
				window.__boardHost.snapshot().board.nodes["string-a"].pins[
					"string-a.out"
				].connected_to.length === 0,
		);
		const after = await snapshot();
		assert.equal(after.transactions.length, before.transactions.length + 1);
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["DisconnectPin"],
		);
		await undo();
	});
	await setFixtureRenderer(page, "egui");
	await api("setDelay", 0);
	await test("actual auto-layout commits a batch and keeps native wires attached to measured ports", async () => {
		await page.evaluate(async () => {
			const module = await import("/board-canvas/board_canvas.js");
			const original = module.Handle.prototype.set_scene;
			const scenes = new Map();
			module.Handle.prototype.set_scene = function (json) {
				const result = original.call(this, json);
				scenes.set(this, JSON.parse(json));
				return result;
			};
			window.__hostGeometryProbe = () =>
				[...scenes].map(([handle, scene]) => ({
					scene,
					stats: JSON.parse(handle.stats()),
				}));
			window.__restoreHostGeometryProbe = () => {
				module.Handle.prototype.set_scene = original;
			};
		});
		try {
			const before = await snapshot();
			await page
				.getByRole("button", { name: "Auto Layout", exact: true })
				.click();
			await page
				.getByRole("dialog")
				.getByRole("button", { name: /^Compact/ })
				.click();
			await waitFor((count) => {
				const list = window.__boardHost.snapshot().transactions;
				return list.length > count && list.at(-1).finished;
			}, before.transactions.length);
			await page.waitForTimeout(900);
			const after = await snapshot();
			assert.equal(after.transactions.length, before.transactions.length + 1);
			const commands = after.transactions.at(-1).commands;
			assert.ok(commands.length > 1);
			assert.ok(
				commands.every((command) => command.command_type === "MoveNode"),
			);
			assert.ok(
				after.nodes.some((entry) => {
					const old = before.nodes.find((item) => item.id === entry.id);
					return (
						old &&
						(old.position.x !== entry.position.x ||
							old.position.y !== entry.position.y)
					);
				}),
			);
			assert.deepEqual(
				after.edges.map((entry) => entry.id).sort(),
				before.edges.map((entry) => entry.id).sort(),
			);
			const geometry = await page.evaluate(() =>
				window
					.__hostGeometryProbe()
					.find((entry) => entry.stats.paintLayer === "edges"),
			);
			assert.ok(geometry, "Auto-layout must update the native scene");
			assert.equal(geometry.stats.unresolvedEdges, 0);
			if (geometry.scene.edges.some((wire) => !wire.dom))
				assert.ok(
					geometry.stats.renderedEdges > 0,
					JSON.stringify(geometry.stats),
				);
			const root = await page.locator(".react-flow").boundingBox();
			assert.ok(root);
			let checkedHandles = 0;
			for (const wire of geometry.scene.edges) {
				const svgEndpoints = wire.dom
					? await edge(wire.id)
							.locator(".react-flow__edge-path")
							.first()
							.evaluate((path) => {
								const matrix = path.getScreenCTM();
								if (!matrix) return null;
								const point = (length) => {
									const value = path
										.getPointAtLength(length)
										.matrixTransform(matrix);
									return { x: value.x, y: value.y };
								};
								return [point(0), point(path.getTotalLength())];
							})
					: undefined;
				if (wire.dom)
					assert.ok(svgEndpoints, `${wire.id} retains its actual SVG path`);
				let endpointIndex = 0;
				for (const endpoint of [wire.source, wire.target]) {
					const projected = geometry.scene.nodes.find(
						(entry) => entry.id === endpoint.nodeId,
					);
					const pin = projected?.pins.find(
						(entry) => entry.id === endpoint.pinId,
					);
					assert.ok(
						projected && pin,
						`${wire.id} has a resolved endpoint ${endpoint.pinId}`,
					);
					const authoritative = after.nodes.find(
						(entry) => entry.id === endpoint.nodeId,
					);
					assert.ok(authoritative);
					assert.equal(projected.x, authoritative.position.x);
					assert.equal(projected.y, authoritative.position.y);
					const expectedX =
						root.x +
						after.viewport.x +
						(projected.x + pin.x) * after.viewport.zoom;
					const expectedY =
						root.y +
						after.viewport.y +
						(projected.y + pin.y) * after.viewport.zoom;
					if (svgEndpoints) {
						const actual = svgEndpoints[endpointIndex];
						assert.ok(
							Math.abs(actual.x - expectedX) < 2 &&
								Math.abs(actual.y - expectedY) < 2,
							`${wire.id} SVG endpoint stays on ${endpoint.pinId}: ${JSON.stringify({ actual, expectedX, expectedY })}`,
						);
					}
					endpointIndex++;
					const locator = node(endpoint.nodeId).locator(
						`[data-handleid="${endpoint.pinId}"]`,
					);
					if (!(await locator.count())) continue;
					const handle = await locator.boundingBox();
					assert.ok(handle);
					assert.ok(
						Math.abs(
							root.x +
								after.viewport.x +
								(projected.x + pin.x) * after.viewport.zoom -
								(handle.x + handle.width / 2),
						) < 2,
					);
					assert.ok(
						Math.abs(
							root.y +
								after.viewport.y +
								(projected.y + pin.y) * after.viewport.zoom -
								(handle.y + handle.height / 2),
						) < 2,
					);
					checkedHandles++;
				}
			}
			assert.ok(
				checkedHandles >= 4,
				"Actual rich DOM ports are checked after layout",
			);
		} finally {
			await page.evaluate(() => window.__restoreHostGeometryProbe?.());
		}
	});
	assert.deepEqual(errors, []);
	assert.deepEqual(externalRequests, []);
	await page.screenshot({ path: `${output}/complete.png` });
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{
				browser: kind,
				passed: results,
				errors,
				externalRequests,
				snapshot: await snapshot(),
			},
			null,
			2,
		),
	);
	console.log(`${results.length} actual FlowBoard checks passed in ${kind}`);
} catch (error) {
	await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{
				browser: kind,
				passed: results,
				error: String(error),
				errors,
				externalRequests,
				snapshot: await snapshot().catch(() => null),
			},
			null,
			2,
		),
	);
	throw error;
} finally {
	await browser.close();
}
