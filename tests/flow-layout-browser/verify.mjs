import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import puppeteer from "puppeteer";

const origin = process.env.FLOW_LAYOUT_QA_URL || "http://127.0.0.1:4334";
const output =
	process.env.FLOW_LAYOUT_QA_OUTPUT ||
	join(tmpdir(), "flow-like-layout-verification");
await mkdir(output, { recursive: true });
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		process.env.CHROME_PATH ||
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
	userDataDir: `${output}/browser-data-${process.pid}`,
});
const page = await browser.newPage();
const errors = [];
const results = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.setViewport({ width: 1600, height: 900, deviceScaleFactor: 1 });
await page.emulateMediaFeatures([
	{ name: "prefers-reduced-motion", value: "reduce" },
]);

async function click(label) {
	const handle = await page.evaluateHandle(
		(label) =>
			[...document.querySelectorAll("button")].find(
				(button) =>
					button.textContent.trim() === label ||
					button.querySelector("span")?.textContent === label,
			),
		label,
	);
	assert.ok(handle.asElement(), `Missing button: ${label}`);
	await handle.asElement().click();
	await handle.dispose();
}

async function waitForGraph() {
	await page.waitForFunction(() => {
		if (!window.layoutQa) return false;
		const { board, edges } = window.layoutQa.snapshot();
		return (
			document.querySelectorAll(".react-flow__node").length === board.length &&
			document.querySelectorAll(".react-flow__edge").length === edges.length
		);
	});
	await page.evaluate(() => window.layoutQa.fit());
	await page.evaluate(
		() =>
			new Promise((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(resolve)),
			),
	);
}

async function measure() {
	return page.evaluate(() => {
		const snapshot = window.layoutQa.snapshot();
		const metadata = new Map(snapshot.edges.map((edge) => [edge.id, edge]));
		const boxes = [...document.querySelectorAll(".react-flow__node")].map(
			(node) => {
				const rect = node.getBoundingClientRect();
				return {
					id: node.dataset.id,
					x: rect.left,
					y: rect.top,
					width: rect.width,
					height: rect.height,
				};
			},
		);
		const edges = [];
		const collisions = [];
		for (const wrapper of document.querySelectorAll(".react-flow__edge")) {
			const edge = metadata.get(wrapper.dataset.id);
			if (edge?.type !== "data") continue;
			const path = wrapper.querySelector('path[id$="-base"]');
			if (!path) throw new Error(`Missing real FlowDataEdge path: ${edge.id}`);
			const length = path.getTotalLength();
			const matrix = path.getScreenCTM();
			const count = Math.max(2, Math.ceil(length));
			const points = [];
			const collided = new Set();
			for (let index = 0; index <= count; index++) {
				const local = path.getPointAtLength((length * index) / count);
				const point = new DOMPoint(local.x, local.y).matrixTransform(matrix);
				points.push({ x: point.x, y: point.y });
				for (const box of boxes) {
					if (box.id === edge.source || box.id === edge.target) continue;
					if (
						point.x > box.x + 0.5 &&
						point.x < box.x + box.width - 0.5 &&
						point.y > box.y + 0.5 &&
						point.y < box.y + box.height - 0.5
					)
						collided.add(box.id);
				}
			}
			for (const nodeId of collided)
				collisions.push({ edge: edge.id, node: nodeId });
			edges.push({ ...edge, points });
		}
		const intersects = (a, b, c, d) => {
			const dx = b.x - a.x;
			const dy = b.y - a.y;
			const otherDx = d.x - c.x;
			const otherDy = d.y - c.y;
			const determinant = dx * otherDy - dy * otherDx;
			if (Math.abs(determinant) < 1e-8) return false;
			const t = ((c.x - a.x) * otherDy - (c.y - a.y) * otherDx) / determinant;
			const u = ((c.x - a.x) * dy - (c.y - a.y) * dx) / determinant;
			return t >= -1e-8 && t <= 1 + 1e-8 && u >= -1e-8 && u <= 1 + 1e-8;
		};
		let crossings = 0;
		for (let i = 0; i < edges.length; i++) {
			for (let j = i + 1; j < edges.length; j++) {
				const left = edges[i];
				const right = edges[j];
				if (
					left.sourceHandle === right.sourceHandle ||
					left.targetHandle === right.targetHandle
				)
					continue;
				let found = false;
				for (let a = 1; a < left.points.length && !found; a++) {
					for (let b = 1; b < right.points.length; b++) {
						if (
							intersects(
								left.points[a - 1],
								left.points[a],
								right.points[b - 1],
								right.points[b],
							)
						) {
							found = true;
							break;
						}
					}
				}
				if (found) crossings++;
			}
		}
		return {
			collisions,
			crossings,
			dataSegments: edges.length,
			reroutes: snapshot.board.filter((node) => node.auto_reroute).length,
		};
	});
}

try {
	for (const pathType of ["default", "straight", "step", "smoothstep"]) {
		for (const scenario of ["screenshot", "dense", "crossing"]) {
			await page.goto(`${origin}/?scenario=${scenario}&path=${pathType}`, {
				waitUntil: "domcontentloaded",
				timeout: 60000,
			});
			await waitForGraph();
			const initial = await page.evaluate(() => window.layoutQa.snapshot());
			const before = await measure();
			await page.screenshot({
				path: `${output}/${scenario}-${pathType}-compact.png`,
			});
			await click("Auto Layout");
			await page.waitForSelector('[role="dialog"]');
			const menuText = await page.$eval(
				'[role="dialog"]',
				(element) => element.textContent,
			);
			assert.ok(
				menuText.includes("Routed") && !menuText.includes("Balanced"),
				"Routed replaces Balanced in the menu",
			);
			if (scenario === "screenshot")
				await page.screenshot({ path: `${output}/layout-menu.png` });
			await click("Routed");
			await page.waitForFunction(
				() => window.layoutQa.snapshot().revision === 1,
			);
			await waitForGraph();
			const routed = await page.evaluate(() => window.layoutQa.snapshot());
			const after = await measure();
			await page.screenshot({
				path: `${output}/${scenario}-${pathType}-routed.png`,
			});
			if (scenario !== "crossing")
				assert.ok(after.reroutes > 0, `${scenario}: routing inserts reroutes`);
			assert.deepEqual(
				after.collisions,
				[],
				`${scenario}: rendered data paths clear unrelated nodes`,
			);
			assert.ok(
				after.crossings <= before.crossings,
				`${scenario}: rendered wire crossings do not increase`,
			);
			if (scenario === "crossing") {
				assert.ok(
					before.crossings > 0,
					"The crossing fixture starts with intersecting wires",
				);
				assert.equal(
					after.crossings,
					0,
					"Routed layout clears the crossing fixture",
				);
			}
			await click("Repeat layout");
			await page.waitForFunction(
				() => window.layoutQa.snapshot().revision === 2,
			);
			await waitForGraph();
			const repeated = await page.evaluate(() => window.layoutQa.snapshot());
			assert.deepEqual(
				repeated.board,
				routed.board,
				`${scenario}: repeated layout preserves positions, node ids, and connections`,
			);
			await click("Undo");
			await click("Undo");
			await page.waitForFunction(
				() => window.layoutQa.snapshot().revision === 4,
			);
			await waitForGraph();
			const undone = await page.evaluate(() => window.layoutQa.snapshot());
			assert.deepEqual(
				undone.board,
				initial.board,
				`${scenario}: undo restores the pre-routing fixture`,
			);
			results.push({ scenario, pathType, before, after });
			console.log(
				`PASS ${scenario} / ${pathType}: ${before.collisions.length} → ${after.collisions.length} node intersections, ${before.crossings} → ${after.crossings} data crossings, ${after.reroutes} reroutes; repeat and undo stable.`,
			);
		}
	}
	assert.deepEqual(errors, [], "No browser page errors");
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ results, errors }, null, 2),
	);
	console.log(`Artifacts: ${output}`);
} catch (error) {
	console.error(error.message);
	await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{
				errors,
				message: String(error),
				results,
				snapshot: await page
					.evaluate(() => window.layoutQa?.snapshot())
					.catch(() => undefined),
			},
			null,
			2,
		),
	);
	throw error;
} finally {
	await browser.close();
}
