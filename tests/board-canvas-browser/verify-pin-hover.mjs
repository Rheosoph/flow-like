import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";

const kind = process.env.BOARD_CANVAS_BROWSER ?? "chromium";
const origin = process.env.BOARD_CANVAS_URL ?? "http://127.0.0.1:4338";
const output = `tmp/board-canvas-browser/pin-hover/${kind}`;
await mkdir(output, { recursive: true });
const browser =
	kind === "webkit"
		? await webkit.launch({ headless: true })
		: await chromium.launch({
				headless: true,
				executablePath: process.env.CHROME_PATH ?? puppeteer.executablePath(),
				args: ["--enable-webgl", "--enable-unsafe-swiftshader"],
			});
const page = await browser.newPage({
	viewport: { width: 1280, height: 960 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(10000);
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
const results = [];
const api = (method, ...args) =>
	page.evaluate(
		({ method, args }) => window.__boardCanvasHarness[method](...args),
		{ method, args },
	);
const hover = page.locator("[data-board-pin-hover]:visible");
const verifyHover = async (point, nodeId, pinId) => {
	await page.mouse.move(point.x, point.y);
	await hover.waitFor();
	assert.equal(await hover.getAttribute("data-node-id"), nodeId);
	assert.equal(await hover.getAttribute("data-pin-id"), pinId);
	assert.equal(
		await page.evaluate(
			({ x, y }) => getComputedStyle(document.elementFromPoint(x, y)).cursor,
			point,
		),
		"crosshair",
	);
	assert.deepEqual(await page.evaluate(() => window.__hoverOverlay), {
		nodeId,
		pinId,
		valid: true,
	});
	return {
		nodeId,
		pinId,
		label: await hover.locator("[data-board-pin-label]").innerText(),
	};
};
const drag = async (from, to) => {
	await page.mouse.move(from.x, from.y);
	await page.mouse.down();
	await page.mouse.move(to.x, to.y, { steps: 12 });
	await page.mouse.up();
	await page.waitForFunction(
		() => window.__boardCanvasHarness.snapshot().commits === 1,
	);
};
try {
	await page.goto(`${origin}/?boardRenderer=egui`, {
		waitUntil: "domcontentloaded",
	});
	await page.waitForFunction(() => window.__boardCanvasHarness?.ready());
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_overlay;
		module.Handle.prototype.set_overlay = function (json) {
			window.__overlay = JSON.parse(json);
			window.__hoverOverlay = window.__overlay.hoveredPin;
			return original.call(this, json);
		};
	});
	await api("reset");
	const ordinary = await api("pin", "integer-a", "integer-a.out");
	const snapped = { x: ordinary.x + 11, y: ordinary.y };
	const ordinaryCandidate = await verifyHover(
		snapped,
		"integer-a",
		"integer-a.out",
	);
	await page.screenshot({ path: `${output}/ordinary.png` });
	await drag(snapped, await api("pin", "integer-b", "integer-b.in"));
	const ordinarySaved = await api("snapshot");
	assert.ok(
		ordinarySaved.edges.some(
			(edge) =>
				edge.sourceHandle === ordinaryCandidate.pinId &&
				edge.targetHandle === "integer-b.in",
		),
	);
	assert.equal(ordinarySaved.nativeConnections, 1);
	results.push({
		name: "snapped ordinary hover matches committed source",
		candidate: ordinaryCandidate,
	});

	await api("reset");
	// The referenced square sits at the right end of ref-b's header; keep it on screen.
	await api("setViewport", -120, -1200, 2.5);
	const ref = page.locator(
		'[data-nodeid="ref-a"][data-handleid="ref_out_ref-a"]',
	);
	await ref.waitFor();
	const bounds = await ref.boundingBox();
	const corner = {
		x: bounds.x + bounds.width - 2,
		y: bounds.y + bounds.height / 2,
	};
	const referenceCandidate = await verifyHover(
		corner,
		"ref-a",
		"ref_out_ref-a",
	);
	await page.screenshot({ path: `${output}/reference.png` });
	await drag(corner, await api("pin", "ref-b", "ref_in_ref-b"));
	const referenceSaved = await api("snapshot");
	assert.deepEqual(referenceSaved.nodes["ref-a"].fn_refs.fn_refs, ["ref-b"]);
	assert.ok(
		referenceSaved.edges.some(
			(edge) =>
				edge.sourceHandle === referenceCandidate.pinId &&
				edge.targetHandle === "ref_in_ref-b",
		),
	);
	results.push({
		name: "large DOM reference handle matches committed source",
		candidate: referenceCandidate,
	});

	await api("reset");
	const body = await api("node", "integer-a");
	await page.mouse.click(body.x, body.y);
	const editor = page.locator(
		'[data-id="integer-a"] [data-testid="harness-pin-editor"]',
	);
	await editor.hover();
	await page.waitForTimeout(100);
	assert.equal(
		await hover.count(),
		0,
		"Nested pin controls do not advertise a connection drag",
	);
	await editor.click();
	assert.match(await editor.innerText(), /Pin editor 1/);
	assert.equal((await api("snapshot")).nativeConnections, 0);
	results.push({ name: "nested pin editor keeps its own hover and click" });

	await api("setReadOnly", true);
	const locked = await api("pin", "integer-a", "integer-a.out");
	await page.mouse.move(locked.x, locked.y);
	await page.waitForTimeout(100);
	assert.equal(await hover.count(), 0);
	assert.notEqual(
		await page.evaluate(
			({ x, y }) => getComputedStyle(document.elementFromPoint(x, y)).cursor,
			locked,
		),
		"crosshair",
	);
	assert.equal((await api("snapshot")).nativeConnections, 0);
	results.push({ name: "read-only pin has no connect cursor or highlight" });

	await api("setReadOnly", false);
	await api("reset");
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_trace;
		window.__traceHandles = new Set();
		module.Handle.prototype.set_trace = function (json) {
			window.__traceHandles.add(this);
			window.__trace = JSON.parse(json);
			return original.call(this, json);
		};
	});
	const endsOf = (trace) =>
		trace.ends.map((end) => `${end.nodeId}:${end.pinId}`).sort();
	const traced = await api("pin", "string-a", "string-a.out");
	await page.mouse.move(traced.x, traced.y);
	await page.waitForFunction(
		() => window.__trace?.hover?.origin.pinId === "string-a.out",
	);
	assert.deepEqual(endsOf(await page.evaluate(() => window.__trace.hover)), [
		"string-b:string-b.in",
	]);
	await page.waitForFunction(() =>
		[...window.__traceHandles].some((handle) => {
			const stats = JSON.parse(handle.stats());
			return stats.paintLayer === "edges" && stats.traceWiresDrawn === 1;
		}),
	);
	await page.mouse.move(traced.x, traced.y + 120);
	await page.waitForFunction(() => !window.__trace?.hover);
	results.push({ name: "hovering a pin lights its wire" });

	const wireEnd = await api("pin", "string-b", "string-b.in");
	const rewire = { x: wireEnd.x - 18, y: wireEnd.y };
	await page.mouse.move(rewire.x, rewire.y + 40);
	await page.mouse.move(rewire.x, rewire.y, { steps: 4 });
	await page.waitForFunction(() =>
		document
			.querySelector("[data-board-pin-label]")
			?.textContent?.startsWith("Reconnect"),
	);
	const rewireCursor = await page.evaluate(
		({ x, y }) => getComputedStyle(document.elementFromPoint(x, y)).cursor,
		rewire,
	);
	assert.match(rewireCursor, /^url\("data:image\/svg\+xml,.*\) 9 9, grab$/);
	await page.mouse.move(wireEnd.x, wireEnd.y, { steps: 2 });
	await page.waitForFunction(() =>
		document
			.querySelector("[data-board-pin-label]")
			?.textContent?.startsWith("Drag"),
	);
	assert.equal(
		await page.evaluate(
			({ x, y }) => getComputedStyle(document.elementFromPoint(x, y)).cursor,
			wireEnd,
		),
		"crosshair",
	);
	await page.mouse.move(rewire.x, rewire.y + 120);
	results.push({ name: "the end of a wire shows the rewire cursor" });

	await api("connect", {
		source: "string-a",
		sourceHandle: "string-a.out",
		target: "string-c",
		targetHandle: "string-c.in",
	});
	await api("setViewport", 560, 330, 1.4);
	const focused = await api("pin", "string-a", "string-a.out");
	/** Waits until a plane's trace stats include `expected`. */
	const traceStats = (paintLayer, expected) =>
		page.waitForFunction(
			({ paintLayer, expected }) =>
				[...window.__traceHandles].some((handle) => {
					const stats = JSON.parse(handle.stats());
					return (
						stats.paintLayer === paintLayer &&
						Object.entries(expected).every(
							([key, value]) => stats[key] === value,
						)
					);
				}),
			{ paintLayer, expected },
		);
	await page.mouse.move(focused.x, focused.y);
	await page.waitForFunction(
		() => window.__trace?.hover?.origin.pinId === "string-a.out",
	);
	await traceStats("nodes", { tracePreviewsDrawn: 1, traceFocused: false });
	// The card fades in; capture it settled.
	await page.waitForTimeout(250);
	await page.screenshot({ path: `${output}/hover-preview.png` });
	results.push({ name: "hovering a pin previews its off-screen end" });

	await page.mouse.click(focused.x, focused.y);
	await page.waitForFunction(
		() => window.__trace?.focus?.origin.pinId === "string-a.out",
	);
	await page.mouse.move(focused.x + 60, focused.y + 90, { steps: 4 });
	await page.waitForTimeout(100);
	assert.equal(
		await page.evaluate(() => window.__overlay?.connection),
		undefined,
		"A pin click starts no wire",
	);
	assert.deepEqual(endsOf(await page.evaluate(() => window.__trace.focus)), [
		"string-b:string-b.in",
		"string-c:string-c.in",
	]);
	await page.waitForFunction(() =>
		[...window.__traceHandles].some(
			(handle) => JSON.parse(handle.stats()).tracePreviewsDrawn === 1,
		),
	);
	const card = await page.evaluate(() => {
		const canvas = document.querySelector('[data-testid="egui-board-canvas"]');
		const bounds = canvas.getBoundingClientRect();
		const handle = [...window.__traceHandles].find(
			(handle) => JSON.parse(handle.stats()).paintLayer === "nodes",
		);
		for (let x = 4; x < bounds.width; x += 8)
			for (let y = 4; y < bounds.height; y += 8) {
				const hit = handle.preview_at(x, y);
				if (!hit) continue;
				const { rect, ...endpoint } = JSON.parse(hit);
				const [left, top, width, height] = rect;
				return {
					endpoint,
					left: bounds.left + left,
					top: bounds.top + top,
					width,
					height,
					x: bounds.left + left + width / 2,
					y: bounds.top + top + height / 2,
				};
			}
		return undefined;
	});
	assert.deepEqual(card?.endpoint, {
		nodeId: "string-c",
		pinId: "string-c.in",
	});
	const cursorAt = (point) =>
		page.evaluate(
			({ x, y }) => getComputedStyle(document.elementFromPoint(x, y)).cursor,
			point,
		);
	// The card covers both the pane and node wrappers; all of it reads as one link.
	for (const [dx, dy] of [
		[0, 0],
		[-0.4, 0],
		[0.4, 0],
		[0, -0.4],
		[0, 0.4],
	]) {
		const point = {
			x: card.x + dx * card.width,
			y: card.y + dy * card.height,
		};
		await page.mouse.move(point.x, point.y);
		await page.waitForFunction(() =>
			document
				.querySelector(".react-flow")
				?.hasAttribute("data-board-preview-hover"),
		);
		assert.equal(await cursorAt(point), "pointer", `cursor at ${dx}, ${dy}`);
	}
	await page.mouse.move(card.x, card.y);
	const previewHint = page.locator("[data-board-preview-hint]:visible");
	await previewHint.waitFor();
	assert.equal(await previewHint.innerText(), "Go to string-c · String");
	const label = await previewHint.boundingBox();
	assert.ok(
		label.y + label.height <= card.top || label.y >= card.top + card.height,
		"The label sits beside the card, not over its preview",
	);
	await page.screenshot({ path: `${output}/trace-preview.png` });
	await page.mouse.click(card.x, card.y);
	const before = (await api("snapshot")).viewport;
	await page.waitForFunction(({ x, y }) => {
		const viewport = window.__boardCanvasHarness.snapshot().viewport;
		return viewport.x !== x || viewport.y !== y;
	}, before);
	await page.waitForTimeout(600);
	const landed = await api("pin", "string-c", "string-c.in");
	const canvas = await page
		.locator('[data-testid="egui-board-canvas"]')
		.boundingBox();
	assert.ok(
		Math.abs(landed.x - (canvas.x + canvas.width / 2)) < 2 &&
			Math.abs(landed.y - (canvas.y + canvas.height / 2)) < 2,
		"A preview click centres its pin",
	);
	assert.equal(
		(await api("snapshot")).nativeConnections,
		0,
		"A preview click connects nothing",
	);
	assert.equal(
		await page.evaluate(() => window.__trace.focus?.origin.pinId),
		"string-a.out",
	);
	await page.keyboard.press("Escape");
	await page.waitForFunction(() => !window.__trace?.focus);
	assert.equal(
		await page.evaluate(() =>
			document
				.querySelector(".react-flow")
				?.hasAttribute("data-board-preview-hover"),
		),
		false,
	);
	results.push({
		name: "a clicked pin previews its off-screen end and flies there",
	});

	await api("reset");
	const wireId = "string-a.out->string-b.in";
	const expectPath = async () => {
		await page.waitForFunction(
			(id) => window.__trace?.focus?.segment === id,
			wireId,
		);
		const path = await page.evaluate(() => window.__trace.focus);
		assert.equal(path.origin, undefined);
		assert.deepEqual(path.edges, [wireId]);
		assert.deepEqual(endsOf(path), [
			"string-a:string-a.out",
			"string-b:string-b.in",
		]);
		assert.deepEqual((await api("snapshot")).selectedEdges, [wireId]);
		assert.equal((await api("snapshot")).nativeConnections, 0);
	};
	const middle = await api("edge", wireId);
	await page.mouse.click(middle.x, middle.y);
	await expectPath();
	await traceStats("edges", { traceWiresDrawn: 1, traceFocused: true });
	await page.screenshot({ path: `${output}/wire-path.png` });
	await page.mouse.click(middle.x, middle.y + 140);
	await page.waitForFunction(() => !window.__trace?.focus);
	const end = await api("pin", "string-b", "string-b.in");
	await page.mouse.click(end.x - 18, end.y);
	await expectPath();
	await page.keyboard.press("Escape");
	await page.waitForFunction(() => !window.__trace?.focus);
	results.push({
		name: "a clicked wire, or its end, lights the path it lies on",
	});
	assert.deepEqual(errors, []);
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, results, errors }, null, 2),
	);
	console.log(JSON.stringify({ kind, results, errors }));
} finally {
	await browser.close();
}
