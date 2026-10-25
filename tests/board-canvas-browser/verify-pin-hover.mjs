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
			window.__hoverOverlay = JSON.parse(json).hoveredPin;
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
	assert.deepEqual(errors, []);
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ kind, results, errors }, null, 2),
	);
	console.log(JSON.stringify({ kind, results, errors }));
} finally {
	await browser.close();
}
