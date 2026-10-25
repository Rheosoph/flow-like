import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { chromium, webkit } from "playwright";
import puppeteer from "puppeteer";
import { setFixtureRenderer } from "../board-canvas-browser/renderer-settings-driver.mjs";
const kind = process.env.BOARD_PARITY_BROWSER ?? "chromium";
const output = `tmp/board-parity-browser/${kind}`;
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
	viewport: { width: 1440, height: 1000 },
	deviceScaleFactor: 1,
});
page.setDefaultTimeout(15000);
const errors = [];
const results = [];
const resizeDiagnostics = [];
const resizeObserverMessage =
	"ResizeObserver loop completed with undelivered notifications.";
let phase = "startup";
page.on("pageerror", (error) => {
	console.log("BROWSER ERROR", phase, error.message);
	if (
		error.message === resizeObserverMessage &&
		(phase === "media resize persists its real dimensions" ||
			phase ===
				"global rem sizing and a changed UI font preserve native and DOM text geometry")
	)
		resizeDiagnostics.push({ phase, message: error.message });
	else errors.push(error.message);
});
const api = (method, ...args) =>
	page.evaluate(({ method, args }) => window.__boardParity[method](...args), {
		method,
		args,
	});
const node = (id) => page.locator(`.react-flow__node[data-id="${id}"]`);
const projectedBounds = async (id) => {
	const bounds = await api("nodeBounds", id);
	const viewport = (await api("snapshot")).viewport;
	const container = await page.locator(".react-flow").boundingBox();
	assert.ok(bounds && container);
	return {
		x: container.x + viewport.x + bounds.x * viewport.zoom,
		y: container.y + viewport.y + bounds.y * viewport.zoom,
		width: bounds.width * viewport.zoom,
		height: bounds.height * viewport.zoom,
	};
};
/** Native nodes mount their production editor while the pointer rests on them. */
const hover = async (id) => {
	const bounds = await projectedBounds(id);
	await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 8);
	await node(id).waitFor();
	await page.waitForTimeout(250);
};
/** Paint statistics of the node plane, whose handle receives the run state. */
const canvasStatus = async () => {
	const stats = await page.evaluate(() =>
		[...(window.__statusHandles ?? [])].map((handle) =>
			JSON.parse(handle.stats()),
		),
	);
	const nodes = stats.find((entry) => entry.paintLayer === "nodes");
	assert.ok(nodes, "The native node plane received run state");
	return nodes;
};
const test = async (name, fn) => {
	phase = name;
	await fn();
	assert.equal(
		await page
			.locator("[data-board-renderer]")
			.getAttribute("data-board-renderer"),
		"egui",
		`Native renderer must remain active after ${name}: ${await page.locator(".parity-board").innerText()}`,
	);
	results.push(name);
	console.log("PASS", name);
};
const settle = () => page.waitForTimeout(250);
async function verifyTypographyBaseline() {
	const baseline = await browser.newPage({
		viewport: { width: 1440, height: 1000 },
		deviceScaleFactor: 1,
	});
	const baselineErrors = [];
	baseline.on("pageerror", (error) => baselineErrors.push(error.message));
	try {
		await baseline.goto("http://127.0.0.1:4339/?boardRenderer=react-flow");
		await baseline.waitForFunction(() => !!window.__boardParity);
		await baseline.waitForTimeout(500);
		await baseline.evaluate(() => {
			document.documentElement.style.fontSize = "20px";
		});
		await baseline.waitForTimeout(500);
		await baseline.evaluate(() => window.__boardParity.select("native"));
		await baseline.waitForTimeout(250);
		const label = baseline
			.locator('.react-flow__node[data-id="native"]')
			.getByTitle("String", { exact: true })
			.first();
		assert.equal(
			await label.evaluate((element) => getComputedStyle(element).fontSize),
			"12px",
		);
		await baseline.evaluate(() => {
			window.__boardParity.select("none");
			document.documentElement.style.fontSize = "";
			document.querySelector(".react-flow").style.fontFamily = "serif";
		});
		await baseline.waitForTimeout(500);
		await baseline.evaluate(() => {
			document.querySelector(".react-flow").style.fontFamily = "";
		});
		await baseline.waitForTimeout(500);
		assert.ok(
			baselineErrors.includes(resizeObserverMessage),
			"A typography resize diagnostic in egui must also reproduce in the React Flow baseline",
		);
		assert.deepEqual(
			baselineErrors.filter((message) => message !== resizeObserverMessage),
			[],
		);
		return baselineErrors;
	} finally {
		await baseline.close();
	}
}
async function verifyResizeBaseline() {
	const baseline = await browser.newPage({
		viewport: { width: 1440, height: 1000 },
		deviceScaleFactor: 1,
	});
	const baselineErrors = [];
	baseline.on("pageerror", (error) => baselineErrors.push(error.message));
	try {
		await baseline.goto("http://127.0.0.1:4339/?boardRenderer=react-flow");
		await baseline.waitForFunction(() => !!window.__boardParity);
		await baseline.evaluate(() => window.__boardParity.select("media"));
		const handle = baseline.locator(
			'.react-flow__node[data-id="media"] .react-flow__resize-control.bottom.right',
		);
		await handle.waitFor();
		await baseline.waitForTimeout(500);
		const before = await baseline.evaluate(
			() => window.__boardParity.snapshot().board.comments.media,
		);
		const corner = await handle.boundingBox();
		assert.ok(corner);
		await baseline.mouse.move(
			corner.x + corner.width / 2,
			corner.y + corner.height / 2,
		);
		await baseline.mouse.down();
		await baseline.mouse.move(
			corner.x + corner.width / 2 + 40,
			corner.y + corner.height / 2 + 30,
			{ steps: 10 },
		);
		await baseline.mouse.up();
		await baseline.waitForTimeout(500);
		const after = await baseline.evaluate(
			() => window.__boardParity.snapshot().board.comments.media,
		);
		assert.equal(after.width, before.width + 40);
		assert.equal(after.height, before.height + 30);
		assert.ok(
			baselineErrors.includes(resizeObserverMessage),
			"A resize diagnostic in egui must also reproduce in the React Flow baseline",
		);
		assert.deepEqual(
			baselineErrors.filter((message) => message !== resizeObserverMessage),
			[],
		);
		return baselineErrors;
	} finally {
		await baseline.close();
	}
}
try {
	await page.goto("http://127.0.0.1:4339/?boardRenderer=egui", {
		timeout: 180000,
	});
	await page.waitForFunction(() => !!window.__boardParity);
	await page.waitForFunction(
		() =>
			document
				.querySelector("[data-board-renderer]")
				?.getAttribute("data-board-renderer") === "egui",
	);
	await settle();
	await page.evaluate(async () => {
		const module = await import("/board-canvas/board_canvas.js");
		const original = module.Handle.prototype.set_status;
		window.__statusHandles = new Set();
		module.Handle.prototype.set_status = function (json) {
			window.__statusHandles.add(this);
			return original.call(this, json);
		};
	});
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
	await test("real node errors, comments, icons and dropdown inputs survive unselected", async () => {
		assert.equal(
			await node("string-a").getByText("Required schema is missing").count(),
			1,
		);
		assert.equal(
			await node("string-a")
				.getByText("Verify the incoming payload before running.")
				.count(),
			1,
		);
		assert.equal(await node("string-b").locator("img").count(), 1);
		// Dropdown inputs paint natively at rest and mount the production editor on hover.
		assert.equal(await node("string-c").count(), 0);
		await hover("string-c");
		assert.equal(
			await node("string-c").getByText("first", { exact: true }).count(),
			1,
		);
		await page.mouse.move(1380, 800);
		await settle();
		assert.equal(
			await page.locator('[data-testid="egui-board-canvas"]').count(),
			1,
		);
	});
	await test("comments preserve configured background, middle and foreground stacking", async () => {
		const stacking = await page.evaluate(() =>
			Object.fromEntries(
				["background", "middle", "foreground", "string-a"].map((id) => {
					const n = document.querySelector(
						`.react-flow__node[data-id="${id}"]`,
					);
					return [id, Number(getComputedStyle(n).zIndex)];
				}),
			),
		);
		assert.equal(stacking.background, 1);
		assert.equal(stacking.middle, 19);
		assert.equal(stacking.foreground, 25);
		assert.ok(stacking["string-a"] > stacking.background);
		await page.screenshot({ path: `${output}/stacking-egui.png` });
		const canvas = await page
			.locator('[data-testid="egui-board-canvas"]')
			.boundingBox();
		assert.ok(canvas.width > 1000 && canvas.height > 500);
	});
	await test("native node hover exposes the production toolbar without changing position", async () => {
		const before = await projectedBounds("native");
		assert.ok(before);
		await page.mouse.move(before.x + before.width / 2, before.y + 8);
		await settle();
		assert.ok(
			await node("native").getByText("Native node", { exact: true }).count(),
		);
		const after = await node("native").boundingBox();
		assert.equal(after.x, before.x);
		assert.equal(after.y, before.y);
		assert.ok(Math.abs(after.width - before.width) <= 1);
		assert.ok(Math.abs(after.height - before.height) <= 1);
		await node("native").focus();
		await api("select", "native");
		await settle();
		assert.equal(
			await node("native").evaluate(
				(element) => element === document.activeElement,
			),
			true,
			"Selection elevation retains the focused native editor wrapper",
		);
		await api("select", "none");
		await page.locator("[data-board-canvas-input]").focus();
		await page.mouse.move(1380, 800);
		await settle();
	});
	await test("production pin dropdown saves its value without starting a wire", async () => {
		const before = (await api("snapshot")).commands.length;
		await hover("string-c");
		await node("string-c").getByRole("combobox").click();
		await api("select", "string-c");
		await settle();
		await page.getByRole("option", { name: "second", exact: true }).click();
		await settle();
		const state = await api("snapshot");
		assert.equal(
			new TextDecoder().decode(
				Uint8Array.from(
					state.board.nodes["string-c"].pins["string-c.in"].default_value,
				),
			),
			JSON.stringify("second"),
		);
		assert.equal(state.commands.length, before + 1);
		assert.equal(state.commands.at(-1).command_type, "UpdateNode");
		await api("select", "none");
	});
	await test("dragging actual production pin handles commits a validated connection", async () => {
		await hover("string-c");
		const target = await node("string-c")
			.locator('[data-handleid="string-c.in"]')
			.boundingBox();
		const source = await node("string-b")
			.locator('[data-handleid="string-b.out"]')
			.boundingBox();
		assert.ok(source && target);
		await page.mouse.move(
			source.x + source.width / 2,
			source.y + source.height / 2,
		);
		await page.mouse.down();
		await page.mouse.move(
			target.x + target.width / 2,
			target.y + target.height / 2,
			{ steps: 15 },
		);
		await page.mouse.up();
		await settle();
		assert.ok(
			(await api("snapshot")).board.nodes["string-b"].pins[
				"string-b.out"
			].connected_to.includes("string-c.in"),
		);
	});
	await test("collapsing and expanding connected pins keeps native wires resolved at the visible handles", async () => {
		const initial = (await api("snapshot")).board.nodes.native;
		await page.evaluate(async () => {
			const module = await import("/board-canvas/board_canvas.js");
			const original = module.Handle.prototype.set_scene;
			const scenes = new Map();
			module.Handle.prototype.set_scene = function (json) {
				const result = original.call(this, json);
				scenes.set(this, JSON.parse(json));
				return result;
			};
			window.__pinGeometryProbe = () =>
				[...scenes].map(([handle, scene]) => ({
					scene,
					stats: JSON.parse(handle.stats()),
				}));
			window.__restorePinGeometryProbe = () => {
				module.Handle.prototype.set_scene = original;
			};
		});
		try {
			const first = initial.pins["native.in"];
			await api("mutate", "native", {
				pins_collapsed: false,
				pins: {
					...initial.pins,
					"native.in": { ...first, index: 2 },
					"native.spare": {
						...first,
						id: "native.spare",
						name: "spare",
						friendly_name: "Unconnected value",
						index: 1,
						depends_on: [],
						connected_to: [],
					},
				},
			});
			await api("select", "native");
			await page.waitForTimeout(400);
			const inspect = async () => {
				const current = await page.evaluate(() =>
					window
						.__pinGeometryProbe()
						.find((entry) => entry.stats.paintLayer === "edges"),
				);
				assert.ok(
					current,
					"Native edge scene was updated after the pin layout changed",
				);
				assert.equal(current.stats.unresolvedEdges, 0);
				if (current.scene.edges.some((edge) => !edge.dom))
					assert.ok(
						current.stats.renderedEdges > 0,
						"Visible native edges are painted",
					);
				const projected = current.scene.nodes.find(
					(item) => item.id === "native",
				);
				assert.ok(projected);
				const viewport = (await api("snapshot")).viewport;
				const root = await page.locator(".react-flow").boundingBox();
				for (const id of ["native.in", "native.out"]) {
					const pin = projected.pins.find((item) => item.id === id);
					const handle = await node("native")
						.locator(`[data-handleid="${id}"]`)
						.boundingBox();
					assert.ok(
						pin && handle && root,
						`Connected handle ${id} remains visible`,
					);
					assert.ok(
						Math.abs(
							root.x +
								viewport.x +
								(projected.x + pin.x) * viewport.zoom -
								(handle.x + handle.width / 2),
						) < 1.1,
						`${id} native wire follows its visible horizontal position`,
					);
					assert.ok(
						Math.abs(
							root.y +
								viewport.y +
								(projected.y + pin.y) * viewport.zoom -
								(handle.y + handle.height / 2),
						) < 1.1,
						`${id} native wire follows its visible vertical position`,
					);
				}
				for (const edge of current.scene.edges.filter(
					(edge) =>
						edge.dom &&
						(edge.source.nodeId === "native" ||
							edge.target.nodeId === "native"),
				)) {
					const fromNative = edge.source.nodeId === "native";
					const endpoint = fromNative ? edge.source : edge.target;
					const handle = await node("native")
						.locator(`[data-handleid="${endpoint.pinId}"]`)
						.boundingBox();
					const point = await page
						.locator(`.react-flow__edge[data-id="${edge.id}"] path`)
						.first()
						.evaluate((path, fromNative) => {
							const point = path.getPointAtLength(
								fromNative ? 0 : path.getTotalLength(),
							);
							return { x: point.x, y: point.y };
						}, fromNative);
					assert.ok(handle && root);
					assert.ok(
						Math.abs(
							root.x +
								viewport.x +
								point.x * viewport.zoom -
								(handle.x + handle.width / 2),
						) < 1.1,
						"Retained animated SVG wire keeps its visible horizontal endpoint",
					);
					assert.ok(
						Math.abs(
							root.y +
								viewport.y +
								point.y * viewport.zoom -
								(handle.y + handle.height / 2),
						) < 1.1,
						`Retained animated SVG wire ${edge.id} keeps its visible vertical endpoint: ${JSON.stringify({ point, handle, viewport, root })}`,
					);
				}
				return {
					y: projected.pins.find((pin) => pin.id === "native.in").y,
					edges: current.scene.edges
						.filter(
							(edge) =>
								edge.source.nodeId === "native" ||
								edge.target.nodeId === "native",
						)
						.map((edge) => edge.id)
						.sort(),
				};
			};
			const expanded = await inspect();
			await node("native")
				.getByRole("button", { name: "Hide unconnected pins", exact: true })
				.click();
			await page.waitForFunction(
				() =>
					window.__boardParity.snapshot().board.nodes.native.pins_collapsed ===
					true,
			);
			await page.waitForTimeout(400);
			assert.equal(
				await node("native").locator('[data-handleid="native.spare"]').count(),
				0,
			);
			const collapsed = await inspect();
			assert.deepEqual(collapsed.edges, expanded.edges);
			assert.ok(
				collapsed.y < expanded.y,
				"Collapse repacks the connected handle",
			);
			await page.screenshot({ path: `${output}/collapsed-connected-pins.png` });
			await node("native")
				.getByRole("button", { name: /Show .* hidden pins?/ })
				.click();
			await page.waitForFunction(
				() =>
					window.__boardParity.snapshot().board.nodes.native.pins_collapsed ===
					false,
			);
			await page.waitForTimeout(400);
			const restored = await inspect();
			assert.deepEqual(restored, expanded);
		} finally {
			await page.evaluate(() => window.__restorePinGeometryProbe());
			await api("mutate", "native", {
				pins: initial.pins,
				pins_collapsed: initial.pins_collapsed,
			});
			await api("select", "none");
			await settle();
		}
	});
	await test("production comment editor saves text and history restores it", async () => {
		const before = (await api("snapshot")).board.comments.background.content;
		await api("select", "background");
		await settle();
		await node("background")
			.getByRole("button", { name: "Edit", exact: true })
			.click();
		const editor = page.getByRole("textbox", { name: "Comment text" });
		await editor.click();
		await page.keyboard.press("ControlOrMeta+A");
		await page.keyboard.insertText("Saved from the real comment editor");
		const draft = await editor.innerText();
		assert.match(draft, /Saved from the real comment editor/);
		const viewport = (await api("snapshot")).viewport;
		await api("select", "native");
		await api(
			"viewport",
			viewport.x + 12,
			viewport.y + 8,
			viewport.zoom * 0.85,
		);
		await settle();
		assert.equal(await editor.innerText(), draft);
		assert.equal(
			await editor.evaluate((element) => element === document.activeElement),
			true,
		);
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await api("viewport", viewport.x, viewport.y, viewport.zoom);
		await settle();
		assert.match(
			(await api("snapshot")).board.comments.background.content,
			/Saved from the real comment editor/,
		);
		await page.getByRole("button", { name: "Undo", exact: true }).click();
		await settle();
		assert.equal(
			(await api("snapshot")).board.comments.background.content,
			before,
		);
	});
	await test("play and stop controls use the existing execution callbacks", async () => {
		await node("exec-a").getByTitle("Execute locally").click();
		await settle();
		assert.ok((await api("snapshot")).runs.includes("fixture-run"));
		await node("exec-a").locator("button:has(svg.lucide-circle-stop)").click();
		await settle();
		assert.equal((await api("snapshot")).runs.length, 0);
	});
	await test("run, log, heatmap and lint state is painted without mounting editors", async () => {
		await api("select", "none");
		await page.mouse.move(1380, 800);
		await api("runtime", true);
		await api("marks", true);
		await settle();
		for (const id of ["integer-a", "integer-b", "exec-b"])
			assert.equal(
				await node(id).count(),
				0,
				`${id} stays on the canvas while the board runs`,
			);
		const running = await canvasStatus();
		assert.ok(running.statusNodes >= 3, "Run, log and lint marks are painted");
		assert.ok(running.statusAnimating && running.cometsDrawn >= 1);
		await page.screenshot({ path: `${output}/runtime-egui.png` });
		// The editor that hovering mounts leaves the marks to the canvas.
		await hover("integer-a");
		assert.equal(
			await node("integer-a").getByText("36×", { exact: true }).count(),
			0,
		);
		assert.ok((await canvasStatus()).statusNodes >= 3);
		await page.mouse.move(1380, 800);
		await api("runtime", false);
		await api("marks", false);
		await settle();
		const idle = await canvasStatus();
		assert.equal(idle.statusNodes, 0);
		assert.equal(idle.statusAnimating, false);
	});
	await test("peer selection appears on an unselected native node", async () => {
		await api("presence", true);
		await settle();
		assert.equal(
			await node("native").getByTitle("Test peer", { exact: true }).count(),
			1,
		);
		await api("presence", false);
		await settle();
		assert.equal(
			await node("native").getByTitle("Test peer", { exact: true }).count(),
			0,
		);
	});
	await test("remote editing and execution remain visible and withdraw without selecting a node", async () => {
		await api("select", "none");
		await page.mouse.move(1380, 800);
		await api("presence", true, "editor");
		await settle();
		assert.equal(
			await node("native")
				.getByTitle("Being edited by Test peer", { exact: true })
				.count(),
			1,
		);
		assert.equal((await api("snapshot")).selected.includes("native"), false);
		await api("presence", true, "execution");
		await settle();
		assert.equal(
			await node("native")
				.getByTitle("Being edited by Test peer", { exact: true })
				.count(),
			0,
		);
		// A collaborator's run glows on the canvas like a local one.
		assert.equal(await node("native").count(), 0);
		const remote = await canvasStatus();
		assert.ok(remote.statusNodes >= 1 && remote.cometsDrawn >= 1);
		await api("presence", false);
		await settle();
		assert.equal((await canvasStatus()).statusNodes, 0);
	});
	await test("native nodes retain live activity, completion and log-summary controls", async () => {
		await api("runtimeNode", "running", 40000);
		await settle();
		assert.equal(await node("native").count(), 0, "Running stays native");
		await hover("native");
		assert.equal(
			await node("native")
				.getByTitle(/^Last activity: 4\ds ago$/)
				.count(),
			1,
		);
		await api("runtimeNode", "done");
		await settle();
		// The canvas draws the completion badge, over the hovered editor too.
		assert.equal(
			await node("native").locator("svg.lucide-square-check").count(),
			0,
		);
		assert.ok((await canvasStatus()).statusNodes >= 1);
		await api("runtimeNode", "none");
		await api("logState", "summary");
		await settle();
		await hover("native");
		const summary = node("native").getByRole("button", {
			name: "Show 6 logs from Native node",
			exact: true,
		});
		await summary.click();
		await page
			.locator(".parity-bar")
			.getByText("Logs filtered for native", { exact: true })
			.waitFor();
		await api("logState", "visited");
		await settle();
		await hover("native");
		const pendingSummary = node("native").getByRole("button", {
			name: "Show logs from Native node",
			exact: true,
		});
		await pendingSummary.focus();
		await page.keyboard.press("Enter");
		assert.ok(
			(await page.locator(".parity-bar").innerText()).includes(
				"Logs filtered for native",
			),
		);
		await api("logState", "none");
		await page.mouse.move(1380, 800);
		await settle();
		assert.equal(
			await node("native")
				.getByRole("button", { name: /Show .*logs/ })
				.count(),
			0,
		);
	});
	await test("pans, camera animations and edge auto-pan move the canvases between frames", async () => {
		await api("select", "none");
		await api("viewport", 20, 20, 1);
		// egui's frame callback runs first in every frame, so a camera or node move written
		// inside a later frame callback is painted a frame after the DOM shows it.
		await page.evaluate(async () => {
			const module = await import("/board-canvas/board_canvas.js");
			let depth = 0;
			const request = window.requestAnimationFrame;
			window.__geometryAudit = {
				request,
				methods: {},
				writes: 0,
				inFrame: [],
			};
			window.requestAnimationFrame = (callback) =>
				request.call(window, (time) => {
					depth++;
					try {
						callback(time);
					} finally {
						depth--;
					}
				});
			for (const method of ["set_viewport", "set_positions"]) {
				const original = module.Handle.prototype[method];
				window.__geometryAudit.methods[method] = original;
				module.Handle.prototype[method] = function (...args) {
					window.__geometryAudit.writes++;
					if (depth)
						window.__geometryAudit.inFrame.push(
							`${method}: ${new Error().stack}`,
						);
					return original.apply(this, args);
				};
			}
		});
		const writes = () => page.evaluate(() => window.__geometryAudit.writes);
		const panned = (await api("snapshot")).viewport;
		await page.mouse.move(650, 520);
		await page.mouse.down();
		for (let step = 1; step <= 12; step++) {
			await page.mouse.move(650 + step * 20, 520 - step * 6);
			await page.waitForTimeout(30);
		}
		await page.mouse.up();
		assert.ok(
			(await api("snapshot")).viewport.x - panned.x > 200,
			"The empty-pane drag panned the camera",
		);
		const beforeAnimation = await writes();
		assert.ok(beforeAnimation >= 12, "The pan moved the canvases");
		await page.evaluate(() =>
			window.__boardParity.animateViewport(-340, 40, 1, 500),
		);
		assert.deepEqual((await api("snapshot")).viewport, {
			x: -340,
			y: 40,
			zoom: 1,
		});
		assert.ok(
			(await writes()) - beforeAnimation > 15,
			"The camera animation moved the canvases in steps",
		);
		const coordinates = (await api("snapshot")).board.nodes.native.coordinates;
		const node = await projectedBounds("native");
		const container = await page.locator(".react-flow").boundingBox();
		assert.ok(container);
		const grab = { x: node.x + node.width / 2, y: node.y + 12 };
		await page.mouse.move(grab.x, grab.y);
		await page.mouse.down();
		await page.mouse.move(grab.x + 24, grab.y, { steps: 3 });
		await page.mouse.move(container.x + container.width - 12, grab.y, {
			steps: 8,
		});
		const beforeAutoPan = {
			writes: await writes(),
			viewport: (await api("snapshot")).viewport,
		};
		await page.waitForTimeout(500);
		const autoPanned = (await api("snapshot")).viewport;
		await page.mouse.up();
		await api("mutate", "native", { coordinates });
		await api("select", "none");
		assert.ok(
			beforeAutoPan.viewport.x - autoPanned.x > 80,
			"Holding a node drag at the right edge auto-panned the camera",
		);
		assert.ok(
			(await writes()) - beforeAutoPan.writes > 15,
			"Edge auto-pan moved the canvases in steps",
		);
		const inFrame = await page.evaluate(async () => {
			const module = await import("/board-canvas/board_canvas.js");
			const audit = window.__geometryAudit;
			window.requestAnimationFrame = audit.request;
			Object.assign(module.Handle.prototype, audit.methods);
			Reflect.deleteProperty(window, "__geometryAudit");
			return audit.inFrame;
		});
		assert.deepEqual(
			inFrame,
			[],
			"No camera or node move is written inside a frame callback",
		);
		await api("viewport", 20, 20, 1);
	});
	await test("collaboration cursors, drag ghosts and pings track the native viewport and layer", async () => {
		await api("viewport", 20, 20, 1);
		await api("peerSignals", "onscreen");
		await settle();
		const cursors = page.getByTestId("peer-cursors");
		const drags = page.getByTestId("peer-drags");
		const pings = page.getByTestId("peer-pings");
		for (const overlay of [cursors, drags, pings])
			assert.equal(
				await overlay.getByText("Test peer", { exact: true }).count(),
				1,
			);
		const cursorPosition = () =>
			cursors
				.locator("[style*='translate3d']")
				.evaluate((element) => getComputedStyle(element).transform);
		assert.equal(await cursorPosition(), "matrix(1, 0, 0, 1, 470, 240)");
		await api("viewport", 50, 30, 1.5);
		await settle();
		assert.equal(await cursorPosition(), "matrix(1, 0, 0, 1, 725, 360)");
		const ghost = await drags.locator(".border-dashed").boundingBox();
		const original = await projectedBounds("native");
		assert.ok(ghost && original);
		assert.ok(Math.abs(ghost.width - original.width) < 1);
		assert.ok(Math.abs(ghost.height - original.height) < 1);
		await api("peerSignals", "offscreen");
		await settle();
		assert.equal(await cursors.locator("svg.lucide-arrow-right").count(), 1);
		assert.equal(await pings.locator("svg.lucide-locate-fixed").count(), 1);
		await api("peerSignals", "other-layer");
		await settle();
		for (const overlay of [cursors, drags, pings])
			assert.equal(
				await overlay.getByText("Test peer", { exact: true }).count(),
				0,
			);
		await api("peerSignals", "none");
		await api("viewport", 20, 20, 1);
		await settle();
	});
	await test("dynamic pin edits preserve authoritative snapshots when commands are rejected", async () => {
		const initial = (await api("snapshot")).board.nodes.native;
		const first = initial.pins["native.in"];
		const pins = {
			...structuredClone(initial.pins),
			"dynamic-second": {
				...first,
				id: "dynamic-second",
				index: 2,
				depends_on: [],
				connected_to: [],
			},
			"dynamic-third": {
				...first,
				id: "dynamic-third",
				index: 3,
				depends_on: [],
				connected_to: [],
			},
		};
		await api("mutate", "native", { pins });
		await api("select", "native");
		await settle();
		const controls = page.locator(
			'[data-egui-pin-controls][data-board-node-id="native"]',
		);
		const add = controls.getByRole("button", { name: /add pin/i });
		assert.equal(await add.count(), 1);
		await api("failNextCommand", "Fixture rejected adding a pin");
		await add.click();
		await page
			.getByText("Fixture rejected adding a pin", { exact: true })
			.waitFor();
		assert.deepEqual((await api("snapshot")).board.nodes.native.pins, pins);
		await add.click();
		await page.waitForFunction(
			() =>
				Object.keys(window.__boardParity.snapshot().board.nodes.native.pins)
					.length === 5,
		);
		const added = (await api("snapshot")).board.nodes.native.pins;
		await controls.locator("[data-egui-pin-manage]").click();
		const remove = page
			.locator('[data-egui-pin-manager][data-board-node-id="native"]')
			.getByRole("button", { name: "Delete Pin" })
			.first();
		await api("failNextCommand", "Fixture rejected deleting a pin");
		await remove.click();
		await page
			.getByText("Fixture rejected deleting a pin", { exact: true })
			.waitFor();
		assert.deepEqual((await api("snapshot")).board.nodes.native.pins, added);
		await remove.click();
		await page.waitForFunction(
			() =>
				Object.keys(window.__boardParity.snapshot().board.nodes.native.pins)
					.length === 4,
		);
		await api("setReadOnly", true);
		await settle();
		assert.equal(
			await page
				.locator(
					'[data-egui-pin-controls][data-board-node-id="native"], [data-egui-pin-manager][data-board-node-id="native"]',
				)
				.count(),
			0,
		);
		await api("setReadOnly", false);
		await api("mutate", "native", { pins: initial.pins });
		await api("select", "none");
		await settle();
	});
	await test("production toolbar actions keep command errors visible without changing pins", async () => {
		await api("select", "native");
		await settle();
		await node("native")
			.getByRole("button", { name: "Info", exact: true })
			.click();
		assert.match(await page.locator(".parity-bar").innerText(), /Info: native/);
		await node("native")
			.getByRole("button", { name: "Explain with FlowPilot", exact: true })
			.click();
		assert.match(
			await page.locator(".parity-bar").innerText(),
			/Explain: native/,
		);
		await api("select", "exec-b");
		await settle();
		await node("exec-b")
			.getByRole("button", { name: "Handle Errors", exact: true })
			.click();
		await page.waitForFunction(() =>
			Object.values(
				window.__boardParity.snapshot().board.nodes["exec-b"].pins,
			).some((pin) => pin.name === "auto_handle_error"),
		);
		const pins = (await api("snapshot")).board.nodes["exec-b"].pins;
		await api("failNextCommand", "Fixture rejected removing error handling");
		await node("exec-b")
			.getByRole("button", { name: "Remove Error Handling", exact: true })
			.click();
		await page
			.getByText("Fixture rejected removing error handling", { exact: true })
			.waitFor();
		assert.deepEqual((await api("snapshot")).board.nodes["exec-b"].pins, pins);
		await node("exec-b")
			.getByRole("button", { name: "Remove Error Handling", exact: true })
			.click();
		await page.waitForFunction(
			() =>
				!Object.values(
					window.__boardParity.snapshot().board.nodes["exec-b"].pins,
				).some((pin) => pin.name.startsWith("auto_handle_error")),
		);
		await api("select", "none");
		await settle();
	});
	await test("node comments preserve existing text, current pins and rejected drafts", async () => {
		await api("select", "string-a");
		await settle();
		const initial = (await api("snapshot")).board.nodes["string-a"];
		await node("string-a")
			.getByRole("button", { name: "Comment", exact: true })
			.click();
		const editor = page.getByRole("textbox", {
			name: "Node comment",
			exact: true,
		});
		assert.equal(await editor.inputValue(), initial.comment);
		await editor.fill("A retained node comment draft");
		const pins = structuredClone(initial.pins);
		pins["string-a.in"].description = "Pin metadata changed by a peer";
		await api("mutate", "string-a", { pins });
		await api("failNextCommand", "Fixture rejected this save");
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await page
			.getByRole("alert")
			.filter({ hasText: "Fixture rejected this save" })
			.waitFor();
		assert.equal(await editor.inputValue(), "A retained node comment draft");
		assert.equal(
			(await api("snapshot")).board.nodes["string-a"].comment,
			initial.comment,
		);
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await page.getByRole("dialog").waitFor({ state: "hidden" });
		const saved = (await api("snapshot")).board.nodes["string-a"];
		assert.equal(saved.comment, "A retained node comment draft");
		assert.equal(
			saved.pins["string-a.in"].description,
			"Pin metadata changed by a peer",
		);
		await node("string-a")
			.getByRole("button", { name: "Comment", exact: true })
			.click();
		await editor.fill("A conflicting local draft");
		await api("mutate", "string-a", { comment: "A peer changed this comment" });
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await page
			.getByRole("alert")
			.filter({ hasText: "This comment changed" })
			.waitFor();
		assert.equal(await editor.inputValue(), "A conflicting local draft");
		assert.equal(
			(await api("snapshot")).board.nodes["string-a"].comment,
			"A peer changed this comment",
		);
		await page.getByRole("button", { name: "Cancel", exact: true }).click();
		await api("mutate", "string-a", {
			comment: initial.comment,
			pins: initial.pins,
		});
		await api("select", "none");
	});
	await test("CJK and emoji labels retain browser font fallback without missing glyphs", async () => {
		await api("mutate", "native", { friendly_name: "日本語のワークフロー 😀" });
		await settle();
		assert.equal(
			await node("native")
				.getByText("日本語のワークフロー 😀", { exact: true })
				.count(),
			1,
		);
		const original = (await api("snapshot")).board.nodes.native;
		const pins = structuredClone(original.pins);
		pins["native.in"].friendly_name = "入力データ";
		await api("mutate", "native", { friendly_name: "Native node", pins });
		await settle();
		assert.equal(
			await node("native").getByText("入力データ", { exact: true }).count(),
			1,
		);
		await api("mutate", "native", { pins: original.pins });
	});
	await test("same-hash errors appear on unselected native nodes and clearing restores the canvas", async () => {
		await api("select", "none");
		await page.mouse.click(1380, 800);
		const original = (await api("snapshot")).board.nodes.native;
		await api("mutate", "native", { hash: 1729, error: undefined });
		await settle();
		assert.equal(await node("native").count(), 0);
		await api("mutate", "native", { error: "A new backend validation error" });
		await node("native")
			.getByText("A new backend validation error", { exact: true })
			.waitFor();
		assert.equal((await api("snapshot")).board.nodes.native.hash, 1729);
		assert.equal((await api("snapshot")).selected.includes("native"), false);
		await api("mutate", "native", { error: undefined });
		await node("native").waitFor({ state: "detached" });
		assert.equal((await api("snapshot")).board.nodes.native.hash, 1729);
		await api("mutate", "native", {
			hash: original.hash,
			error: original.error,
		});
	});
	await test("global rem sizing and a changed UI font preserve native and DOM text geometry", async () => {
		await api("select", "none");
		await page.mouse.click(1380, 800);
		await settle();
		const original = await api("nodeBounds", "native");
		const rootFontSize = await page.evaluate(
			() => document.documentElement.style.fontSize,
		);
		await page.evaluate(() => {
			document.documentElement.style.fontSize = "20px";
		});
		await page.waitForFunction(
			(height) => window.__boardParity.nodeBounds("native").height > height,
			original.height,
		);
		const resized = await projectedBounds("native");
		assert.ok(Math.abs(resized.height - original.height - 7) < 1);
		await api("select", "native");
		await settle();
		const dom = await node("native").boundingBox();
		assert.ok(Math.abs(dom.height - resized.height) < 1);
		assert.equal(
			await node("native")
				.getByTitle("String", { exact: true })
				.first()
				.evaluate((element) => getComputedStyle(element).fontSize),
			"12px",
		);
		await api("select", "none");
		await page.mouse.click(1380, 800);
		await page.evaluate((value) => {
			document.documentElement.style.fontSize = value;
			document.querySelector(".react-flow").style.fontFamily = "serif";
		}, rootFontSize);
		await node("native").getByText("Native node", { exact: true }).waitFor();
		assert.equal(
			await node("native").evaluate(
				(element) => getComputedStyle(element).fontFamily,
			),
			"serif",
		);
		await page.evaluate(() => {
			document.querySelector(".react-flow").style.fontFamily = "";
		});
		await node("native").waitFor({ state: "detached" });
	});
	await test("node rename preserves current metadata and retains rejected drafts", async () => {
		const original = (await api("snapshot")).board.nodes["exec-a"];
		await api("select", "exec-a");
		await settle();
		await node("exec-a")
			.getByRole("button", { name: "Rename", exact: true })
			.click();
		const name = page.getByRole("textbox", { name: "Node name" });
		assert.equal(await name.inputValue(), original.friendly_name);
		await name.fill("Renamed workflow");
		await api("mutate", "exec-a", { comment: "Concurrent annotation" });
		await api("failNextCommand", "Fixture rejected renaming");
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await page
			.getByRole("alert")
			.filter({ hasText: "Fixture rejected renaming" })
			.waitFor();
		assert.equal(await name.inputValue(), "Renamed workflow");
		await page.getByRole("button", { name: "Save", exact: true }).click();
		await page.getByRole("dialog").waitFor({ state: "hidden" });
		const saved = (await api("snapshot")).board.nodes["exec-a"];
		assert.equal(saved.friendly_name, "Renamed workflow");
		assert.equal(saved.comment, "Concurrent annotation");
		await api("mutate", "exec-a", {
			friendly_name: original.friendly_name,
			comment: original.comment,
		});
		await api("select", "none");
		await settle();
	});
	await test("media resize persists its real dimensions", async () => {
		await api("select", "media");
		await settle();
		const before = (await api("snapshot")).board.comments.media;
		const corner = await node("media")
			.locator(".react-flow__resize-control.bottom.right")
			.last()
			.boundingBox();
		assert.ok(corner);
		await page.mouse.move(
			corner.x + corner.width / 2,
			corner.y + corner.height / 2,
		);
		await page.mouse.down();
		await page.mouse.move(
			corner.x + corner.width / 2 + 40,
			corner.y + corner.height / 2 + 30,
			{ steps: 10 },
		);
		await page.mouse.up();
		await settle();
		const after = (await api("snapshot")).board.comments.media;
		assert.equal(after.width, before.width + 40);
		assert.equal(after.height, before.height + 30);
	});
	await test("media loads and historical versions hide media mutations", async () => {
		const image = node("media").locator("img");
		await image.waitFor();
		assert.ok(await image.evaluate((e) => e.complete && e.naturalWidth > 0));
		await api("select", "media");
		await settle();
		assert.ok(
			await node("media").locator(".react-flow__resize-control").count(),
		);
		await api("setReadOnly", true);
		await settle();
		assert.equal(
			await node("media").locator(".react-flow__resize-control").count(),
			0,
		);
		assert.equal(await node("media").getByTitle("Lock media").count(), 0);
		await api("setReadOnly", false);
		await settle();
	});
	await test("comment lock and depth controls persist through production commands", async () => {
		await api("select", "background");
		await settle();
		await node("background")
			.getByRole("button", { name: "Move Up", exact: true })
			.click();
		await settle();
		assert.equal((await api("snapshot")).board.comments.background.z_index, 2);
		await node("background").getByTitle("Lock comment").click();
		await settle();
		assert.equal(
			(await api("snapshot")).board.comments.background.is_locked,
			true,
		);
		await node("background").getByTitle("Unlock comment").click();
		await settle();
		assert.equal(
			(await api("snapshot")).board.comments.background.is_locked,
			false,
		);
	});
	await test("switching to a historical version closes an already open comment editor", async () => {
		await api("select", "background");
		await settle();
		await node("background")
			.getByRole("button", { name: "Edit", exact: true })
			.click();
		assert.equal(await page.getByRole("dialog").count(), 1);
		await api("setReadOnly", true);
		await page.getByRole("dialog").waitFor({ state: "hidden" });
		assert.equal(await page.getByRole("dialog").count(), 0);
		await api("setReadOnly", false);
		await settle();
	});
	await test("historical comments cannot open edit or resize controls", async () => {
		await api("select", "background");
		await api("setReadOnly", true);
		await settle();
		assert.equal(
			await node("background").locator(".react-flow__resize-control").count(),
			0,
		);
		await node("background").dblclick({ position: { x: 15, y: 120 } });
		await settle();
		assert.equal(await page.getByRole("dialog").count(), 0);
		await api("setReadOnly", false);
		await settle();
	});
	await test("layer navigation uses the original production layer component", async () => {
		await api("select", "group");
		await settle();
		await node("group").dblclick();
		await settle();
		assert.equal((await api("snapshot")).layer, "group");
		assert.ok((await api("nodeBounds", "layer-child")).width > 0);
		await api("logState", "summary", "layer-child");
		await settle();
		await hover("layer-child");
		await node("layer-child")
			.getByRole("button", { name: "Show 6 logs from Inside layer" })
			.click();
		await page.waitForFunction(() =>
			document
				.querySelector(".parity-bar")
				?.textContent.includes("Logs filtered for layer-child"),
		);
		assert.match(
			await page.locator(".parity-bar").innerText(),
			/Logs filtered for layer-child/,
		);
		await api("logState", "none");
		await page.getByRole("button", { name: "Root", exact: true }).click();
		await settle();
		assert.equal((await api("snapshot")).layer, undefined);
	});
	await test("box selection includes mixed native nodes and media", async () => {
		await api("viewport", 20, 20, 1);
		const bounds = await page.locator(".react-flow").boundingBox();
		assert.ok(bounds);
		const start = { x: bounds.x + 20 + 1040, y: bounds.y + 20 + 90 };
		const finish = { x: bounds.x + 20 + 1320, y: bounds.y + 20 + 450 };
		await api("select", "none");
		await page.keyboard.down("Shift");
		await page.mouse.move(start.x, start.y);
		await page.mouse.down();
		await page.mouse.move(finish.x, finish.y, { steps: 10 });
		await page.mouse.up();
		await page.keyboard.up("Shift");
		await settle();
		assert.deepEqual((await api("snapshot")).selected.sort(), [
			"media",
			"native",
		]);
	});
	await test("renderer switch preserves the same rich board state", async () => {
		const initial = await api("snapshot");
		await setFixtureRenderer(page, "react-flow");
		await settle();
		assert.equal(
			await node("string-a").getByText("Required schema is missing").count(),
			1,
		);
		await page.screenshot({ path: `${output}/react-flow.png` });
		await setFixtureRenderer(page, "egui");
		await page.waitForFunction(
			() =>
				document
					.querySelector("[data-board-renderer]")
					?.getAttribute("data-board-renderer") === "egui",
		);
		assert.deepEqual((await api("snapshot")).board, initial.board);
	});
	assert.deepEqual(errors, [], "No uncaught production component errors");
	// React Flow's own observer can report this in WebKit while resizing. Verify
	// the unchanged renderer reproduces it before classifying it as shared.
	const baselineResizeDiagnostics = resizeDiagnostics.some(
		(item) => item.phase === "media resize persists its real dimensions",
	)
		? await verifyResizeBaseline()
		: [];
	const baselineTypographyDiagnostics = resizeDiagnostics.some(
		(item) =>
			item.phase ===
			"global rem sizing and a changed UI font preserve native and DOM text geometry",
	)
		? await verifyTypographyBaseline()
		: [];
	await writeFile(
		`${output}/results.json`,
		JSON.stringify(
			{
				kind,
				results,
				errors,
				resizeDiagnostics,
				baselineResizeDiagnostics,
				baselineTypographyDiagnostics,
			},
			null,
			2,
		),
	);
	console.log(`Verified ${results.length} production parity cases in ${kind}`);
} catch (error) {
	await page.screenshot({ path: `${output}/failure.png` });
	await writeFile(
		`${output}/failure.json`,
		JSON.stringify(
			{
				phase,
				error: String(error),
				errors,
				resizeDiagnostics,
				snapshot: await api("snapshot").catch(() => undefined),
				nativeDom: await node("native")
					.evaluateAll((elements) =>
						elements.map((element) => element.outerHTML),
					)
					.catch(() => []),
			},
			null,
			2,
		),
	);
	console.error(errors);
	throw error;
} finally {
	await browser.close();
}
