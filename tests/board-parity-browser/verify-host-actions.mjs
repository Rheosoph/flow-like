import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output =
	process.env.BOARD_ACTION_OUTPUT ??
	`tmp/board-parity-browser/host-actions/${kind}`;
await mkdir(output, { recursive: true });
const browser = await (kind === "webkit"
	? webkit.launch()
	: chromium.launch({
			headless: true,
			executablePath: puppeteer.executablePath(),
			args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
		}));
const page = await browser.newPage({
	viewport: { width: 1600, height: 1000 },
	hasTouch: true,
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
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardHost[method](...args), {
		method,
		args,
	});
const snapshot = async () => JSON.parse(JSON.stringify(await api("snapshot")));
const palette = () =>
	page.getByPlaceholder("Search nodes and actions", { exact: true });
const node = (id) => page.locator(`.react-flow__node[data-id="${id}"]`);
const settle = () => page.waitForTimeout(150);
const wait = (predicate) => page.waitForFunction(predicate);
const selected = () =>
	page.evaluate(() =>
		window.__boardHost
			.snapshot()
			.nodes.filter((n) => n.selected)
			.map((n) => n.id),
	);
async function reload(extra = "") {
	await page.goto(`http://127.0.0.1:4339/host?boardRenderer=egui${extra}`, {
		timeout: 180000,
	});
	await wait(
		() =>
			!!window.__boardHost &&
			document.querySelector('[data-board-renderer="egui"]'),
	);
	await page.waitForTimeout(750);
	await api("viewport", 20, 20, 1);
	await settle();
}
async function header(id) {
	const state = await snapshot();
	const n = state.nodes.find((n) => n.id === id);
	assert.ok(n);
	const box = await page.locator(".react-flow").boundingBox();
	return {
		x: box.x + state.viewport.x + (n.position.x + 60) * state.viewport.zoom,
		y: box.y + state.viewport.y + (n.position.y + 12) * state.viewport.zoom,
	};
}
async function clickNode(id, modifier) {
	const p = await header(id);
	if (modifier) await page.keyboard.down("ControlOrMeta");
	await page.mouse.click(p.x, p.y);
	if (modifier) await page.keyboard.up("ControlOrMeta");
	await settle();
}
async function drag(from, to) {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps: 15 });
	await page.mouse.up();
	await settle();
}
async function pin(id, direction) {
	// A native node mounts its production handles while the pointer rests on it.
	if (!(await node(id).count())) {
		const p = await header(id);
		await page.mouse.move(p.x, p.y);
		await node(id).waitFor();
		await settle();
	}
	const locator = node(id).locator(`[data-handleid="${id}.${direction}"]`);
	const box = await locator.boundingBox();
	assert.ok(box);
	return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}
async function historyUndo() {
	const n = (await snapshot()).transactions.length;
	await page.keyboard.press("ControlOrMeta+z");
	await page.waitForFunction((n) => {
		const t = window.__boardHost.snapshot().transactions;
		return t.length > n && t.at(-1).finished;
	}, n);
	await settle();
}
async function test(name, run, extra = "") {
	if (
		process.env.BOARD_ACTION_FILTER &&
		!new RegExp(process.env.BOARD_ACTION_FILTER).test(name)
	)
		return;
	await reload(extra);
	try {
		await run();
		assert.ok(
			await page.locator('[data-board-renderer="egui"]').count(),
			"Native renderer stays active",
		);
		assert.deepEqual(errors, []);
		results.push({ name, passed: true });
		console.log("PASS", name);
	} catch (error) {
		results.push({ name, passed: false, error: error.message });
		await page.screenshot({ path: `${output}/failure.png` });
		await writeFile(
			`${output}/failure-state.json`,
			JSON.stringify(await snapshot().catch(() => ({ errors })), null, 2),
		);
		throw error;
	}
}
try {
	await test("FlowScript and presence highlights promote native nodes and clear independently without selection or navigation", async () => {
		const before = await snapshot();
		const nativeBefore = before.nodes.find((value) => value.id === "native");
		assert.equal(nativeBefore.nativeDom, false);
		assert.equal(await node("native").count(), 0);
		await api("highlightNode", "native");
		await page
			.locator('.react-flow__node.flowscript-nav-highlight[data-id="native"]')
			.waitFor();
		assert.equal(
			(await snapshot()).nodes.find((value) => value.id === "native").nativeDom,
			true,
		);
		await api("highlightNodes", ["native", "integer-a"]);
		await page
			.locator(
				'.react-flow__node.flowscript-nav-highlight[data-id="integer-a"]',
			)
			.waitFor();
		await api("highlightNode");
		await settle();
		assert.equal(
			await page
				.locator('.react-flow__node.flowscript-nav-highlight[data-id="native"]')
				.count(),
			1,
		);
		await api("highlightNode", "native");
		await api("highlightNodes");
		await page
			.locator(
				'.react-flow__node.flowscript-nav-highlight[data-id="integer-a"]',
			)
			.waitFor({ state: "detached" });
		assert.equal(
			await page
				.locator('.react-flow__node.flowscript-nav-highlight[data-id="native"]')
				.count(),
			1,
		);
		await api("highlightNode");
		await node("native").waitFor({ state: "detached" });
		const after = await snapshot();
		assert.equal(
			after.nodes.find((value) => value.id === "native").nativeDom,
			false,
		);
		assert.deepEqual(after.viewport, before.viewport);
		assert.deepEqual(
			after.nodes.filter((value) => value.selected).map((value) => value.id),
			before.nodes.filter((value) => value.selected).map((value) => value.id),
		);
		assert.deepEqual(after.board, before.board);
		assert.deepEqual(after.history, before.history);
		assert.equal(after.transactions.length, 0);
	});
	await test("switching boards clears both highlight sources before rendering the same node IDs", async () => {
		await api("highlightNode", "native");
		await api("highlightNodes", ["native", "integer-a"]);
		await page
			.locator('.react-flow__node.flowscript-nav-highlight[data-id="native"]')
			.waitFor();
		await api("switchBoard", "host-parity-next");
		await wait(
			() => window.__boardHost.snapshot().surfaceBoardId === "host-parity-next",
		);
		await wait(
			() =>
				document.querySelectorAll(".react-flow__node.flowscript-nav-highlight")
					.length === 0,
		);
		await wait(
			() =>
				window.__boardHost
					.snapshot()
					.nodes.find((value) => value.id === "native")?.nativeDom === false,
		);
		assert.equal((await snapshot()).board.id, "host-parity-next");
		assert.equal((await snapshot()).transactions.length, 0);
	});
	await test("Find shortcut opens search without placing a node", async () => {
		await clickNode("native");
		const before = await snapshot();
		await page.keyboard.press("ControlOrMeta+f");
		await page.getByRole("dialog").waitFor();
		assert.equal(
			(await snapshot()).transactions.length,
			before.transactions.length,
		);
		await page.keyboard.press("Escape");
	});
	await test("native node context menu opens and dismissal does not edit the graph", async () => {
		const p = await header("native");
		await page.mouse.click(p.x, p.y, { button: "right" });
		await palette().waitFor();
		assert.ok((await selected()).includes("native"));
		await page.keyboard.press("Escape");
		assert.equal((await snapshot()).transactions.length, 0);
	});
	await test("wire context selection deletes one wire and undoes", async () => {
		const a = await pin("string-a", "out");
		const b = await pin("string-b", "in");
		await page.mouse.click((a.x + b.x) / 2, (a.y + b.y) / 2, {
			button: "right",
		});
		await palette().waitFor();

		await page.keyboard.press("Escape");
		await page.locator("[data-board-canvas-input]").focus();
		await page.keyboard.press("Delete");
		await wait(
			() =>
				window.__boardHost.snapshot().board.nodes["string-a"].pins[
					"string-a.out"
				].connected_to.length === 0,
		);
		assert.deepEqual(
			(await snapshot()).transactions
				.at(-1)
				.commands.map((c) => c.command_type),
			["DisconnectPin"],
		);
		await historyUndo();
		assert.deepEqual(
			(await snapshot()).board.nodes["string-a"].pins["string-a.out"]
				.connected_to,
			["string-b.in"],
		);
	});
	await test("mixed node and comment deletion commits one batch and restores connections", async () => {
		await clickNode("native");
		await clickNode("foreground", true);
		assert.deepEqual((await selected()).sort(), ["foreground", "native"]);
		const before = await snapshot();
		await page.keyboard.press("Delete");
		await wait(() => !window.__boardHost.snapshot().board.nodes.native);
		const after = await snapshot();
		assert.ok(!after.board.comments.foreground);
		assert.deepEqual(
			after.transactions
				.at(-1)
				.commands.map((c) => c.command_type)
				.sort(),
			["RemoveComment", "RemoveNode"],
		);
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
		assert.deepEqual((await snapshot()).board.comments, before.board.comments);
	});
	await test("duplicate uses one CopyPaste batch without touching the clipboard", async () => {
		await clickNode("native");
		const before = await snapshot();
		await page.keyboard.press("ControlOrMeta+d");
		await page.waitForFunction(
			(count) =>
				Object.keys(window.__boardHost.snapshot().board.nodes).length ===
				count + 1,
			Object.keys(before.board.nodes).length,
		);
		const after = await snapshot();
		const added = Object.values(after.board.nodes).find(
			(n) => !before.board.nodes[n.id],
		);
		assert.deepEqual(added.coordinates, [1084, 404, 0]);
		assert.ok(
			Object.values(added.pins).every(
				(p) => !p.connected_to.length && !p.depends_on.length,
			),
		);
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["CopyPaste"],
		);
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
	});
	await test("copy then cut records clipboard data before deleting and undo restores the node", async () => {
		await clickNode("native");
		await page.evaluate(() => {
			window.__copiedText = "";
			for (const type of ["copy", "cut"])
				document.addEventListener(type, (event) => {
					window.__copiedText =
						event.clipboardData?.getData("text/plain") ?? "";
				});
		});
		const before = await snapshot();
		await page.keyboard.press("ControlOrMeta+c");
		await settle();
		assert.equal(
			JSON.parse(await page.evaluate(() => window.__copiedText)).nodes[0].id,
			"native",
		);
		assert.equal(
			(await snapshot()).transactions.length,
			before.transactions.length,
		);
		await page.keyboard.press("ControlOrMeta+x");
		await wait(() => !window.__boardHost.snapshot().board.nodes.native);
		assert.equal(
			JSON.parse(await page.evaluate(() => window.__copiedText)).nodes[0].id,
			"native",
		);
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
	});
	await test("paste consumes the delivered clipboard graph and preserves internal wires", async () => {
		await clickNode("string-a");
		await clickNode("string-b", true);
		await page.keyboard.press("ControlOrMeta+c");
		await settle();
		const before = await snapshot();
		await page.mouse.move(1250, 750);
		await page.keyboard.press("ControlOrMeta+v");
		await page.waitForFunction(
			(count) =>
				Object.keys(window.__boardHost.snapshot().board.nodes).length ===
				count + 2,
			Object.keys(before.board.nodes).length,
		);
		const after = await snapshot();
		const added = Object.values(after.board.nodes).filter(
			(n) => !before.board.nodes[n.id],
		);
		const pins = new Set(added.flatMap((n) => Object.keys(n.pins)));
		assert.equal(
			added.flatMap((n) => Object.values(n.pins).flatMap((p) => p.connected_to))
				.length,
			1,
		);
		assert.ok(
			added.every((n) =>
				Object.values(n.pins).every((p) =>
					[...p.connected_to, ...p.depends_on].every((id) => pins.has(id)),
				),
			),
		);
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
	});
	await test("edge double-click inserts a reroute in one undo transaction", async () => {
		const before = await snapshot();
		const a = await pin("string-a", "out");
		const b = await pin("string-b", "in");
		await page.mouse.dblclick((a.x + b.x) / 2, (a.y + b.y) / 2);
		await wait(() =>
			Object.values(window.__boardHost.snapshot().board.nodes).some(
				(n) => n.name === "reroute",
			),
		);
		const after = await snapshot();
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["AddNode", "DisconnectPin", "ConnectPin", "ConnectPin"],
		);
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
	});
	await test("empty native wire drop opens compatible palette and one choice creates one undo batch", async () => {
		const before = await snapshot();
		const from = await pin("integer-a", "out");
		await drag(from, { x: 1280, y: 740 });
		await palette().waitFor();
		await palette().fill("integer-b");
		await page.keyboard.press("Enter");
		await page.waitForFunction(
			(count) =>
				Object.keys(window.__boardHost.snapshot().board.nodes).length ===
				count + 1,
			Object.keys(before.board.nodes).length,
		);
		await settle();
		const after = await snapshot();
		assert.deepEqual(
			after.transactions.at(-1).commands.map((c) => c.command_type),
			["AddNode", "ConnectPin"],
		);
		assert.equal(after.transactions.length, before.transactions.length + 1);
		await page.locator("[data-board-canvas-input]").focus();
		await historyUndo();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
	});
	await test("comment drag carries enclosed nodes and Alt-drag moves only the comment", async () => {
		const before = await snapshot();
		const box = await node("background").boundingBox();
		assert.ok(box);
		const from = { x: box.x + 6, y: box.y + 8 };
		await drag(from, { x: from.x + 35, y: from.y + 25 });
		await page.waitForFunction(
			() => window.__boardHost.snapshot().transactions.length > 0,
		);
		const moved = await snapshot();
		const dx =
			moved.board.comments.background.coordinates[0] -
			before.board.comments.background.coordinates[0];
		const dy =
			moved.board.comments.background.coordinates[1] -
			before.board.comments.background.coordinates[1];
		assert.ok(Math.abs(dx) > 10);
		for (const id of ["string-a", "string-b"])
			assert.deepEqual(moved.board.nodes[id].coordinates, [
				before.board.nodes[id].coordinates[0] + dx,
				before.board.nodes[id].coordinates[1] + dy,
				0,
			]);
		assert.equal(moved.transactions.length, 1);
		await historyUndo();
		await page.keyboard.down("Alt");
		await drag(from, { x: from.x + 27, y: from.y + 19 });
		await page.keyboard.up("Alt");
		const alone = await snapshot();
		assert.deepEqual(alone.board.nodes, before.board.nodes);
		assert.notDeepEqual(
			alone.board.comments.background.coordinates,
			before.board.comments.background.coordinates,
		);
	});
	await test("mixed grouping and ungrouping use actual toolbars and reversible host commands", async () => {
		await clickNode("integer-a");
		await clickNode("middle", true);
		const p = await header("integer-a");
		await page.mouse.move(p.x, p.y);
		await settle();
		await node("integer-a")
			.getByRole("button", { name: "Collapse", exact: true })
			.click();
		await wait(
			() =>
				Object.keys(window.__boardHost.snapshot().board.layers).length === 2,
		);
		const grouped = await snapshot();
		const layer = Object.values(grouped.board.layers).find(
			(l) => l.id !== "group",
		);
		assert.equal(grouped.board.nodes["integer-a"].layer, layer.id);
		assert.equal(grouped.board.comments.middle.layer, layer.id);
		await clickNode(layer.id);
		await node(layer.id)
			.getByRole("button", { name: "Extend (Ungroup)", exact: true })
			.click();
		await wait(
			() =>
				Object.keys(window.__boardHost.snapshot().board.layers).length === 1,
		);
		assert.equal((await snapshot()).board.nodes["integer-a"].layer, undefined);
		assert.equal((await snapshot()).board.comments.middle.layer, undefined);
		await page.locator("[data-board-canvas-input]").focus();
		await historyUndo();
		assert.equal((await snapshot()).board.nodes["integer-a"].layer, layer.id);
	});
	await test(
		"historical boards block delete, cut, duplicate and palette placement",
		async () => {
			await clickNode("native");
			const before = await snapshot();
			for (const chord of ["Delete", "ControlOrMeta+x", "ControlOrMeta+d"])
				await page.keyboard.press(chord);
			const from = await pin("integer-a", "out");
			await drag(from, { x: 1280, y: 740 });
			await settle();
			assert.equal(await palette().isVisible(), false);
			assert.equal((await snapshot()).transactions.length, 0);
			assert.deepEqual((await snapshot()).board, before.board);
		},
		"&historical=1",
	);
	if (kind === "chromium")
		await test("stationary touch opens the board menu and moving touch cancels long press", async () => {
			const cdp = await page.context().newCDPSession(page);
			const p = await header("native");
			await cdp.send("Input.dispatchTouchEvent", {
				type: "touchStart",
				touchPoints: [p],
			});
			await page.waitForTimeout(780);
			await cdp.send("Input.dispatchTouchEvent", {
				type: "touchEnd",
				touchPoints: [],
			});
			await palette().waitFor();
			await page.keyboard.press("Escape");
			await cdp.send("Input.dispatchTouchEvent", {
				type: "touchStart",
				touchPoints: [{ x: 1450, y: 800 }],
			});
			await cdp.send("Input.dispatchTouchEvent", {
				type: "touchMove",
				touchPoints: [{ x: 1400, y: 800 }],
			});
			await page.waitForTimeout(780);
			await cdp.send("Input.dispatchTouchEvent", {
				type: "touchEnd",
				touchPoints: [],
			});
			assert.equal(await palette().isVisible(), false);
			await cdp.detach();
		});
	await test("a rejected palette batch keeps the graph and history unchanged and permits retry", async () => {
		const before = await snapshot();
		await drag(await pin("integer-a", "out"), { x: 1280, y: 740 });
		await palette().fill("integer-b");
		await api("rejectNext");
		await page.keyboard.press("Enter");
		await page
			.getByText("Fixture rejected the command batch", { exact: true })
			.first()
			.waitFor();
		assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
		assert.equal((await snapshot()).history.canUndo, false);
		assert.equal(await palette().isVisible(), true);
		await page.keyboard.press("Enter");
		await page.waitForFunction(
			(count) =>
				Object.keys(window.__boardHost.snapshot().board.nodes).length ===
				count + 1,
			Object.keys(before.board.nodes).length,
		);
	});
	await test(
		"actual recommendation ghost accepts via its DOM control and undoes as one batch",
		async () => {
			await clickNode("integer-a");
			await page
				.getByRole("button", {
					name: "Add suggested node integer-b",
					exact: true,
				})
				.waitFor({ timeout: 30000 });
			const before = await snapshot();
			await page
				.getByRole("button", {
					name: "Add suggested node integer-b",
					exact: true,
				})
				.click();
			await page.waitForFunction(
				(count) =>
					Object.keys(window.__boardHost.snapshot().board.nodes).length ===
					count + 1,
				Object.keys(before.board.nodes).length,
			);
			assert.equal((await snapshot()).transactions.length, 1);
			assert.equal(
				await page.locator("[data-node-suggestion-ghost]").count(),
				0,
			);
			await page.locator("[data-board-canvas-input]").focus();
			await historyUndo();
			assert.deepEqual((await snapshot()).board.nodes, before.board.nodes);
		},
		"&ghosts=1",
	);
	await test(
		"recommendation dismissal removes only transient state",
		async () => {
			await clickNode("integer-a");
			await page
				.getByRole("button", {
					name: "Add suggested node integer-b",
					exact: true,
				})
				.waitFor({ timeout: 30000 });
			const before = await snapshot();
			await page
				.getByRole("button", { name: "Dismiss suggestion", exact: true })
				.click();
			await page
				.locator("[data-node-suggestion-ghost]")
				.waitFor({ state: "hidden" });
			assert.deepEqual((await snapshot()).board, before.board);
			assert.equal((await snapshot()).transactions.length, 0);
		},
		"&ghosts=1",
	);

	await page.screenshot({ path: `${output}/complete.png` });
} finally {
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, results, errors }, null, 2),
	);
	await browser.close();
	console.log(
		`Browser closed; ${results.filter((r) => r.passed).length}/${results.length} passed`,
	);
}
