import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import puppeteer from "puppeteer";
import sharp from "sharp";

interface CraftEvent {
	type: string;
	requestId?: string;
	bytes?: ArrayBuffer;
	revision?: number;
	readOnly?: boolean;
	error?: string;
	dirty?: boolean;
	warnings?: string[];
	interactive?: boolean;
	completed?: number;
	total?: number;
	response?: Record<string, unknown> | null;
}

declare global {
	interface Window {
		events: CraftEvent[];
		runtimeSession: string;
		craftBridge: {
			mount: () => void;
			setReadOnly: (readOnly: boolean) => void;
			invoke: (
				command: string,
				args: Record<string, unknown>,
			) => Promise<unknown>;
		};
	}
}

const directory = await mkdtemp(join(tmpdir(), "flow-like-craft-solid-"));
const assets = resolve(import.meta.dir, "../../apps/web/public/craft");
const upstream = await Bun.file(
	resolve(
		import.meta.dir,
		"../../packages/craft-viewers/solidcraft/upstream.json",
	),
).json();
const source =
	process.env.CRAFT_SOURCE_SOLIDCRAFT ||
	resolve(
		import.meta.dir,
		"../../target/craft-viewers/solidcraft/source",
		upstream.revision,
	);
const bridgeEntry = join(directory, "mcp-bridge.ts");
await writeFile(
	bridgeEntry,
	`
import { CraftMcpSession } from ${JSON.stringify(resolve(import.meta.dir, "../../packages/ui/components/storage/craft-mcp-session.ts"))};
import { executeDeviceCommand } from ${JSON.stringify(resolve(import.meta.dir, "../../packages/ui/lib/device-bridge.ts"))};
let session;
let editing = true;
const post = message => document.querySelector("iframe")?.contentWindow?.postMessage({ ...message, channel: "flow-like-craft", session: window.runtimeSession }, location.origin);
addEventListener("message", event => session?.receive(event));
window.craftBridge = {
  mount() {
    session?.dispose();
    session = new CraftMcpSession({
      target: { appId: "mcp-app", surfaceId: "model", componentId: "preview" },
      sessionId: window.runtimeSession, editor: "solidcraft", channel: "flow-like-craft", origin: location.origin,
      frame: () => document.querySelector("iframe")?.contentWindow ?? null,
      post, state: () => ({ ready: true, editable: true, editing, busy: false })
    });
  },
  setReadOnly(readOnly) { editing = !readOnly; post({ type: "set-read-only", readOnly }); },
  invoke(command, args) { return executeDeviceCommand(command, args, { appId: "mcp-app", deadline: Date.now() + 30000 }); }
};
`,
);
const bridgeBuild = await Bun.build({
	entrypoints: [bridgeEntry],
	target: "browser",
});
assert.ok(
	bridgeBuild.success,
	`Could not bundle the live MCP registry: ${bridgeBuild.logs}`,
);
const bridgeScript = await bridgeBuild.outputs[0].text();
const encode = (value: string) => new TextEncoder().encode(value);
const stl = encode(`solid tetrahedron
facet normal 0 0 -1
outer loop
vertex 0 0 0
vertex 0 10 0
vertex 10 0 0
endloop
endfacet
facet normal 0 -1 0
outer loop
vertex 0 0 0
vertex 10 0 0
vertex 0 0 10
endloop
endfacet
facet normal -1 0 0
outer loop
vertex 0 0 0
vertex 0 0 10
vertex 0 10 0
endloop
endfacet
facet normal 1 1 1
outer loop
vertex 10 0 0
vertex 0 10 0
vertex 0 0 10
endloop
endfacet
endsolid tetrahedron
`);
const sketch = {
	format: "solidcraft/1",
	name: "Rectangle",
	units: "mm",
	params: [],
	next_id: 2,
	features: [
		{
			id: 1,
			name: "Rectangle sketch",
			suppressed: false,
			type: "sketch",
			plane: { type: "origin", name: "XY" },
			sketch: {
				points: [
					{ id: "origin", pos: [0, 0], fixed: true },
					{ id: "l1.start", pos: [0, 0], fixed: false },
					{ id: "l1.end", pos: [20, 0], fixed: false },
					{ id: "l2.end", pos: [20, 10], fixed: false },
					{ id: "l3.end", pos: [0, 10], fixed: false },
				],
				curves: [
					{ id: "l1", type: "line", a: 1, b: 2, construction: false },
					{ id: "l2", type: "line", a: 2, b: 3, construction: false },
					{ id: "l3", type: "line", a: 3, b: 4, construction: false },
					{ id: "l4", type: "line", a: 4, b: 1, construction: false },
				],
				constraints: [],
				counters: {},
			},
		},
	],
};
const triangleGeometry = Buffer.alloc(44);
for (const [index, value] of [0, 0, 0, 0.01, 0, 0, 0, 0.01, 0].entries())
	triangleGeometry.writeFloatLE(value, index * 4);
for (let index = 0; index < 3; index++)
	triangleGeometry.writeUInt16LE(index, 36 + index * 2);
const triangleGltf = {
	asset: { version: "2.0" },
	buffers: [{ byteLength: 42 }],
	bufferViews: [
		{ buffer: 0, byteOffset: 0, byteLength: 36, target: 34962 },
		{ buffer: 0, byteOffset: 36, byteLength: 6, target: 34963 },
	],
	accessors: [
		{
			bufferView: 0,
			componentType: 5126,
			count: 3,
			type: "VEC3",
			min: [0, 0, 0],
			max: [0.01, 0.01, 0],
		},
		{ bufferView: 1, componentType: 5123, count: 3, type: "SCALAR" },
	],
	meshes: [{ primitives: [{ attributes: { POSITION: 0 }, indices: 1 }] }],
	nodes: [{ mesh: 0 }],
	scenes: [{ nodes: [0] }],
	scene: 0,
};
const embeddedGltf = encode(
	JSON.stringify({
		...triangleGltf,
		buffers: [
			{
				byteLength: 42,
				uri: `data:application/octet-stream;base64,${triangleGeometry.subarray(0, 42).toString("base64")}`,
			},
		],
	}),
);
const triangleJson = JSON.stringify(triangleGltf);
const paddedJson = Buffer.from(
	triangleJson.padEnd(Math.ceil(triangleJson.length / 4) * 4, " "),
);
const embeddedGlb = Buffer.alloc(
	12 + 8 + paddedJson.length + 8 + triangleGeometry.length,
);
embeddedGlb.write("glTF");
embeddedGlb.writeUInt32LE(2, 4);
embeddedGlb.writeUInt32LE(embeddedGlb.length, 8);
embeddedGlb.writeUInt32LE(paddedJson.length, 12);
embeddedGlb.write("JSON", 16);
paddedJson.copy(embeddedGlb, 20);
const binaryChunk = 20 + paddedJson.length;
embeddedGlb.writeUInt32LE(triangleGeometry.length, binaryChunk);
embeddedGlb.write("BIN\0", binaryChunk + 4);
triangleGeometry.copy(embeddedGlb, binaryChunk + 8);
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	async fetch(request) {
		const path = new URL(request.url).pathname;
		if (path === "/mcp-bridge.js")
			return new Response(bridgeScript, {
				headers: { "Content-Type": "text/javascript" },
			});
		if (path === "/")
			return new Response(
				`<!doctype html><html><link rel="icon" href="data:,"><style>html,body,iframe{width:100%;height:100%;margin:0;border:0}</style><script>window.events=[];window.runtimeSession='solid-smoke';addEventListener('message',e=>{if(e.origin===location.origin&&e.source===document.querySelector('iframe')?.contentWindow&&e.data.channel==='flow-like-craft'&&e.data.session===window.runtimeSession)events.push(e.data)});</script><script type="module" src="/mcp-bridge.js"></script><body></body></html>`,
				{ headers: { "Content-Type": "text/html" } },
			);
		const file = Bun.file(join(assets, path));
		return (await file.exists())
			? new Response(file)
			: new Response("Not found", { status: 404 });
	},
});
const browser = await puppeteer.launch({
	headless: true,
	args: ["--enable-unsafe-swiftshader"],
	...(process.env.CHROME_PATH
		? { executablePath: process.env.CHROME_PATH }
		: {}),
});
const page = await browser.newPage();
const errors: string[] = [];
let failed = false;
page.on("pageerror", (error) => errors.push(String(error)));
page.on("console", (message) => {
	if (message.type() === "error") console.error("solidcraft:", message.text());
});
await page.setViewport({ width: 1280, height: 900 });
await page.setRequestInterception(true);
page.on("request", (request) => {
	if (
		request.url().startsWith(server.url.origin) ||
		/^(blob|data):/.test(request.url())
	)
		void request.continue();
	else {
		errors.push(`External request: ${request.url()}`);
		void request.abort();
	}
});

const send = (data: Record<string, unknown>) =>
	page.evaluate((data) => {
		document
			.querySelector("iframe")
			?.contentWindow?.postMessage(
				{ ...data, channel: "flow-like-craft", session: window.runtimeSession },
				location.origin,
			);
	}, data);
const cursor = () => page.evaluate(() => window.events.length);
const waitEvent = async (type: string, requestId?: string, after = 0) => {
	console.log(
		`solidcraft: waiting for ${type}${requestId ? ` (${requestId})` : ""}`,
	);
	await page.waitForFunction(
		(type, requestId, after) =>
			window.events
				.slice(after)
				.some(
					(event) =>
						(event.type === type &&
							(!requestId || event.requestId === requestId)) ||
						(event.type === "error" &&
							type !== "error" &&
							(!event.requestId || event.requestId === requestId)),
				),
		{ timeout: 180_000, polling: 100 },
		type,
		requestId,
		after,
	);
	return page.evaluate(
		(type, requestId, after) => {
			const event = window.events
				.slice(after)
				.find(
					(event) =>
						(event.type === type &&
							(!requestId || event.requestId === requestId)) ||
						(event.type === "error" &&
							type !== "error" &&
							(!event.requestId || event.requestId === requestId)),
				);
			if (!event) throw new Error(`Missing ${type} event`);
			if (event.type === "error" && type !== "error")
				throw new Error(event.error);
			return {
				...event,
				bytes: event.bytes ? [...new Uint8Array(event.bytes)] : undefined,
			};
		},
		type,
		requestId,
		after,
	);
};
const settle = () =>
	page.evaluate(
		() =>
			new Promise<void>((resolve) =>
				requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
			),
	);
const load = async (sessionId = "solid-smoke") => {
	await page.goto(server.url.origin);
	await page.evaluate((sessionId) => {
		window.runtimeSession = sessionId;
		const iframe = document.createElement("iframe");
		iframe.src = `/solidcraft/index.html?session=${encodeURIComponent(sessionId)}`;
		document.body.append(iframe);
	}, sessionId);
	await waitEvent("ready");
};
const open = async (name: string, bytes: Uint8Array, readOnly = true) => {
	const after = await cursor();
	await page.evaluate(
		({ name, bytes, readOnly }) => {
			const buffer = Uint8Array.from(bytes).buffer;
			document.querySelector("iframe")?.contentWindow?.postMessage(
				{
					channel: "flow-like-craft",
					session: window.runtimeSession,
					type: "open",
					name,
					bytes: buffer,
					readOnly,
				},
				location.origin,
				[buffer],
			);
		},
		{ name, bytes: [...bytes], readOnly },
	);
	return after;
};
const exportDesign = async (requestId: string, purpose?: "download") => {
	const after = await cursor();
	await send({ type: "export", requestId, format: "solidcraft", purpose });
	const exported = await waitEvent("exported", requestId, after);
	assert.ok(exported.bytes && exported.bytes.length > 100);
	assert.ok(Number.isSafeInteger(exported.revision));
	const bytes = Uint8Array.from(exported.bytes);
	assert.deepEqual([...bytes.subarray(0, 4)], [0x50, 0x4b, 3, 4]);
	const path = join(directory, `${requestId}.solidcraft`);
	await writeFile(path, bytes);
	const entry = (name: string) => execFileSync("unzip", ["-p", path, name]);
	assert.equal(
		JSON.parse(entry("container.json").toString()).format,
		"solidcraft-container/1",
	);
	const document = JSON.parse(entry("document.json").toString());
	assert.equal(document.format, "solidcraft/47");
	return {
		bytes,
		revision: exported.revision,
		document,
		entry,
	};
};
const assertLoading = async (after: number) => {
	const events = await page.evaluate(
		(after) => window.events.slice(after),
		after,
	);
	const opened = events.findIndex((event) => event.type === "opened");
	const progress = events.filter((event) => event.type === "loading");
	assert.ok(progress.length > 0, "The staged loader reports progress");
	assert.ok(opened > events.findIndex((event) => event.type === "loading"));
	for (const event of progress) {
		assert.equal(event.interactive, true);
		assert.ok(Number.isSafeInteger(event.completed));
		assert.ok(Number.isSafeInteger(event.total));
		assert.ok((event.completed ?? -1) >= 0);
		assert.ok((event.total ?? -1) >= (event.completed ?? 0));
	}
};
const command = async (text: string) => {
	// The current editor accepts command JSON in the ribbon's search box.
	await page.mouse.click(1190, 56);
	await settle();
	await page.keyboard.type(text);
	await page.keyboard.press("Enter");
	await settle();
	// Let the search box finish closing before the next command opens it again.
	await new Promise((resolve) => setTimeout(resolve, 160));
};
const viewportPixels = async () =>
	sharp(
		await page.screenshot({
			clip: { x: 250, y: 150, width: 800, height: 600 },
		}),
	)
		.removeAlpha()
		.raw()
		.toBuffer();
const assertViewChanged = (before: Buffer, after: Buffer, action: string) => {
	assert.equal(before.length, after.length);
	let changed = 0;
	for (let index = 0; index < before.length; index++)
		if (Math.abs(before[index] - after[index]) > 15) changed++;
	assert.ok(changed > 1_000, `${action} must change the rendered model view`);
};

try {
	await load();
	assert.equal(
		(await waitEvent("opened", undefined, await open("tetrahedron.stl", stl)))
			.readOnly,
		true,
	);
	await page.mouse.move(1100, 800);
	await settle();
	await page.screenshot({ path: join(directory, "solidcraft-preview.png") });
	const beforeNavigation = await cursor();
	const initialView = await viewportPixels();
	await page.mouse.move(650, 450);
	await page.mouse.down({ button: "right" });
	for (let step = 1; step <= 5; step++) {
		await page.mouse.move(650 + step * 20, 450 + step * 8);
		await settle();
	}
	await page.mouse.up({ button: "right" });
	await page.mouse.move(1100, 800);
	await settle();
	const orbitedView = await viewportPixels();
	assertViewChanged(initialView, orbitedView, "Readonly orbit");
	await page.mouse.wheel({ deltaY: -220 });
	await settle();
	const zoomedView = await viewportPixels();
	assertViewChanged(orbitedView, zoomedView, "Readonly zoom");
	await page.mouse.click(623, 878);
	await new Promise((resolve) => setTimeout(resolve, 700));
	assertViewChanged(zoomedView, await viewportPixels(), "Native preview Fit");
	await page.screenshot({
		path: join(directory, "solidcraft-preview-fit.png"),
	});
	assert.equal(
		await page.evaluate(
			(after) =>
				window.events
					.slice(after)
					.some((event) => event.type === "dirty" && event.dirty),
			beforeNavigation,
		),
		false,
		"Preview navigation must not dirty the model",
	);
	await send({ type: "export", requestId: "readonly", format: "solidcraft" });
	assert.match((await waitEvent("error", "readonly")).error ?? "", /editing/i);
	await send({ type: "set-read-only", readOnly: false });
	await settle();
	const imported = await exportDesign("imported-mesh");
	const meshFeature = imported.document.features.find(
		(feature: { type: string }) => feature.type === "mesh_import",
	);
	assert.ok(meshFeature, "STL import must retain an editable mesh feature");
	assert.equal(meshFeature.meshes.length, 1);
	const mesh = imported.entry(meshFeature.meshes[0].mesh_blob);
	assert.equal(mesh.toString("ascii", 0, 8), "SCMESH01");
	assert.equal(
		Number(mesh.readBigUInt64LE(16)),
		12,
		"The tetrahedron keeps four triangles",
	);
	const positionCount = Number(mesh.readBigUInt64LE(8));
	const positions = Array.from({ length: positionCount }, (_, index) =>
		mesh.readDoubleLE(24 + index * 8),
	);
	assert.equal(Math.min(...positions), 0);
	assert.equal(Math.max(...positions), 10);
	await command("FILE.NEW {}");
	const afterNew = await exportDesign("blocked-new");
	assert.deepEqual(
		afterNew.document.features,
		imported.document.features,
		"The command palette cannot replace the host's document",
	);
	assert.equal(afterNew.revision, imported.revision);

	const beforeEdit = await cursor();
	await command(
		'solid.box {"length":12,"width":8,"height":4,"corner":[30,0,0]}',
	);
	assert.equal((await waitEvent("dirty", undefined, beforeEdit)).dirty, true);
	const edited = await exportDesign("edited-mesh");
	const box = edited.document.features.find(
		(feature: { type: string }) => feature.type === "box",
	);
	assert.ok(
		box,
		"The command field adds a solid feature to the imported model",
	);
	assert.deepEqual([box.length, box.width, box.height], ["12", "8", "4"]);
	assert.deepEqual(box.corner, [30, 0, 0]);
	assert.ok((edited.revision ?? 0) > (imported.revision ?? 0));
	await command(
		'solid.box {"length":3,"width":4,"height":5,"corner":[50,0,0]}',
	);
	const beforeSaveRequest = await cursor();
	await command("FILE.SAVE {}");
	await waitEvent("save-request", undefined, beforeSaveRequest);
	const beforeSaved = await cursor();
	await send({ type: "saved", revision: edited.revision });
	await settle();
	const current = await exportDesign("current-mesh");
	await page.keyboard.press("F6");
	await new Promise((resolve) => setTimeout(resolve, 700));
	await page.screenshot({ path: join(directory, "solidcraft-editing.png") });
	assert.ok((current.revision ?? 0) > (edited.revision ?? 0));
	assert.equal(
		current.document.features.filter(
			(feature: { type: string }) => feature.type === "box",
		).length,
		2,
	);
	await send({ type: "saved", revision: (current.revision ?? 0) + 1 });
	await settle();
	assert.equal(
		await page.evaluate(
			(after) =>
				window.events
					.slice(after)
					.some((event) => event.type === "dirty" && event.dirty === false),
			beforeEdit,
		),
		false,
		"Palette Save, export and stale upload acknowledgements must retain newer edits",
	);
	await send({ type: "saved", revision: current.revision });
	assert.equal((await waitEvent("dirty", undefined, beforeSaved)).dirty, false);
	await load();
	assert.equal(
		(
			await waitEvent(
				"opened",
				undefined,
				await open("edited.solidcraft", current.bytes),
			)
		).readOnly,
		true,
	);
	await send({ type: "set-read-only", readOnly: false });
	const reopened = await exportDesign("reopened-mesh");
	assert.deepEqual(reopened.document.features, current.document.features);
	assert.deepEqual(reopened.entry(meshFeature.meshes[0].mesh_blob), mesh);
	console.log(
		"solidcraft: STL geometry, readonly export rejection, UI edit, saved revision, native save and reopen verified",
	);

	await load();
	await waitEvent(
		"opened",
		undefined,
		await open("rectangle.solidcraft", encode(JSON.stringify(sketch)), false),
	);
	const drawing = await exportDesign("rectangle");
	assert.equal(drawing.document.features.length, 1);
	assert.deepEqual(
		drawing.document.features[0].plane,
		sketch.features[0].plane,
	);
	assert.deepEqual(
		drawing.document.features[0].sketch,
		sketch.features[0].sketch,
	);
	await load();
	await waitEvent(
		"opened",
		undefined,
		await open("rectangle.solidcraft", drawing.bytes),
	);
	await settle();
	await page.screenshot({ path: join(directory, "solidcraft-sketch.png") });
	console.log("solidcraft: native 2D sketch geometry survives save and reopen");
	await load("new-model");
	const beforeCreate = await cursor();
	await send({ type: "create", name: "New model.solidcraft", readOnly: false });
	assert.equal(
		(await waitEvent("opened", undefined, beforeCreate)).readOnly,
		false,
	);
	assert.equal((await waitEvent("dirty", undefined, beforeCreate)).dirty, true);
	await settle();
	await page.screenshot({
		path: join(directory, "solidcraft-blank-editor.png"),
	});
	const blankDownload = await exportDesign("blank-download", "download");
	assert.equal(blankDownload.document.name, "New model");
	assert.deepEqual(blankDownload.document.features, []);
	const beforeDownloadAck = await cursor();
	await send({ type: "saved", revision: blankDownload.revision });
	await settle();
	assert.equal(
		await page.evaluate(
			(after) =>
				window.events
					.slice(after)
					.some((event) => event.type === "dirty" && event.dirty === false),
			beforeDownloadAck,
		),
		false,
		"A download cannot acknowledge a new model as saved",
	);
	// The pinned native Create Sketch button opens its plane-selection dialog.
	await page.mouse.click(134, 82);
	await settle();
	const beforeSketchDialog = await cursor();
	await send({
		type: "export",
		requestId: "active-sketch-dialog",
		format: "solidcraft",
	});
	assert.match(
		(await waitEvent("error", "active-sketch-dialog", beforeSketchDialog))
			.error ?? "",
		/finish or cancel/i,
	);
	await page.keyboard.press("Escape");
	await settle();
	await command('solid.box {"length":20,"width":12,"height":8}');
	const freshBox = await exportDesign("created-box");
	assert.equal(freshBox.document.features.length, 1);
	assert.equal(freshBox.document.features[0].type, "box");
	// Native app-bar Undo and Redo operate on the newly created live document.
	await page.mouse.click(239, 17);
	await settle();
	assert.deepEqual((await exportDesign("created-undo")).document.features, []);
	await page.mouse.click(272, 17);
	await settle();
	const freshRedo = await exportDesign("created-redo");
	assert.deepEqual(freshRedo.document.features, freshBox.document.features);
	await page.keyboard.press("F6");
	await new Promise((resolve) => setTimeout(resolve, 700));
	await page.screenshot({
		path: join(directory, "solidcraft-created-editing.png"),
	});
	await send({ type: "set-read-only", readOnly: true });
	await settle();
	await page.screenshot({
		path: join(directory, "solidcraft-created-preview.png"),
	});
	const freshPreview = await exportDesign(
		"created-preview-download",
		"download",
	);
	assert.deepEqual(freshPreview.document.features, freshRedo.document.features);
	assert.equal(freshPreview.revision, freshRedo.revision);
	const beforePreviewAck = await cursor();
	await send({ type: "saved", revision: freshRedo.revision });
	assert.equal(
		(await waitEvent("dirty", undefined, beforePreviewAck)).dirty,
		false,
	);
	await send({ type: "set-read-only", readOnly: false });
	await settle();
	// Change theme through the native Preferences dialog, then inspect the preview.
	await page.mouse.click(1054, 17);
	await settle();
	await page.mouse.click(620, 254);
	await settle();
	await page.mouse.click(889, 615);
	await settle();
	await page.screenshot({
		path: join(directory, "solidcraft-editing-light.png"),
	});
	await send({ type: "set-read-only", readOnly: true });
	await settle();
	await page.mouse.click(623, 878);
	await new Promise((resolve) => setTimeout(resolve, 700));
	await page.screenshot({
		path: join(directory, "solidcraft-preview-light.png"),
	});
	const lightPreview = await exportDesign("light-preview-download", "download");
	assert.deepEqual(lightPreview.document.features, freshRedo.document.features);
	assert.equal(lightPreview.revision, freshRedo.revision);
	console.log(
		"solidcraft: blank creation, native sketch dialog/undo/redo, download and save acknowledgement verified",
	);
	await load();
	const beforeNativeReopen = await open(
		"created.solidcraft",
		freshRedo.bytes,
		false,
	);
	await waitEvent("opened", undefined, beforeNativeReopen);
	await assertLoading(beforeNativeReopen);
	assert.deepEqual(
		(await exportDesign("created-reopened")).document.features,
		freshRedo.document.features,
		"A new format 47 model must reopen with its feature history",
	);
	for (const extension of ["FCStd", "3dm"]) {
		const bytes = new Uint8Array(
			await Bun.file(
				join(source, "crates/io/src/native_cad/fixtures", `block.${extension}`),
			).arrayBuffer(),
		);
		await load();
		const opened = await waitEvent(
			"opened",
			undefined,
			await open(`block.${extension}`, bytes, false),
		);
		assert.ok(
			opened.warnings?.some((warning) =>
				/original design history/i.test(warning),
			),
			`${extension} import must explain that it preserves geometry rather than the original design history`,
		);
		const imported = await exportDesign(`native-${extension}`);
		const feature = imported.document.features.find(
			(feature: { type: string }) => feature.type === "import",
		);
		assert.ok(
			feature?.step_blob,
			`${extension} must preserve imported STEP geometry`,
		);
		const step = imported.entry(feature.step_blob);
		assert.match(step.toString(), /MANIFOLD_SOLID_BREP/);
		await load();
		await waitEvent(
			"opened",
			undefined,
			await open(`imported-${extension}.solidcraft`, imported.bytes, false),
		);
		const reopened = await exportDesign(`native-${extension}-reopened`);
		assert.deepEqual(reopened.document.features, imported.document.features);
		assert.deepEqual(reopened.entry(feature.step_blob), step);
		console.log(
			`solidcraft: ${extension} geometry import, warning, format 47 save and reopen verified`,
		);
	}
	await load();
	const stepBytes = new Uint8Array(
		await Bun.file(
			join(source, "tests/step-conformance/cylinder_seam.step"),
		).arrayBuffer(),
	);
	const beforeStep = await open("cylinder.step", stepBytes, false);
	await waitEvent("opened", undefined, beforeStep);
	await assertLoading(beforeStep);
	const stepDesign = await exportDesign("staged-step");
	assert.ok(
		stepDesign.document.features.some(
			(feature: { type: string }) => feature.type === "import",
		),
	);
	await load();
	const manyFeatures = {
		...sketch,
		name: "Cancellation fixture",
		next_id: 6001,
		features: Array.from({ length: 6000 }, (_, index) => ({
			...sketch.features[0],
			id: index + 1,
			name: `Sketch ${index + 1}`,
		})),
	};
	const beforeCancelOpen = await open(
		"cancel.solidcraft",
		encode(JSON.stringify(manyFeatures)),
	);
	await waitEvent("loading", undefined, beforeCancelOpen);
	const beforePendingExport = await cursor();
	await send({
		type: "export",
		requestId: "pending-load",
		format: "solidcraft",
		purpose: "download",
	});
	assert.match(
		(await waitEvent("error", "pending-load", beforePendingExport)).error ?? "",
		/enable editing/i,
	);
	const beforeCancel = await cursor();
	await page.mouse.click(1100, 800);
	await page.keyboard.press("Escape");
	assert.match(
		(await waitEvent("error", undefined, beforeCancel)).error ?? "",
		/cancelled/i,
	);
	const cancelledEvents = await page.evaluate(
		(after) => window.events.slice(after),
		beforeCancelOpen,
	);
	assert.equal(
		cancelledEvents.some((event) => event.type === "opened"),
		false,
	);
	assert.equal(
		cancelledEvents.some((event) => event.type === "dirty" && event.dirty),
		false,
	);
	const beforeRetry = await cursor();
	await send({
		type: "create",
		name: "After cancel.solidcraft",
		readOnly: false,
	});
	await waitEvent("opened", undefined, beforeRetry);
	assert.deepEqual((await exportDesign("after-cancel")).document.features, []);
	console.log(
		"solidcraft: native design and STEP progress, cancellation, blocked export and clean retry verified",
	);
	for (const [format, bytes] of [
		["gltf", embeddedGltf],
		["glb", embeddedGlb],
	] as const) {
		await load();
		await waitEvent(
			"opened",
			undefined,
			await open(`triangle.${format}`, bytes, false),
		);
		const imported = await exportDesign(`embedded-${format}`);
		const feature = imported.document.features.find(
			(feature: { type: string }) => feature.type === "mesh_import",
		);
		assert.ok(feature, `${format} import must contain the embedded mesh`);
		assert.equal(feature.meshes.length, 1);
		const mesh = imported.entry(feature.meshes[0].mesh_blob);
		assert.equal(Number(mesh.readBigUInt64LE(8)), 9);
		assert.equal(Number(mesh.readBigUInt64LE(16)), 3);
		console.log(`solidcraft: embedded ${format} triangle imported and saved`);
	}

	await load("mcp-original");
	await waitEvent(
		"opened",
		undefined,
		await open("agent-model.stl", stl, false),
	);
	await page.waitForFunction(() => !!window.craftBridge);
	await page.evaluate(() => window.craftBridge.mount());
	const beforeMcp = await exportDesign("before-mcp");
	const foregroundPage = await browser.newPage();
	await foregroundPage.bringToFront();
	assert.equal(
		await page.evaluate(() => document.visibilityState),
		"hidden",
		"The editor must be hidden while an external MCP client works",
	);
	const elementRef = "model/preview";
	const pin = (await page.evaluate(
		(elementRef) => window.craftBridge.invoke("craft.session", { elementRef }),
		elementRef,
	)) as { sessionId: string; editor: string };
	assert.deepEqual(pin, { sessionId: "mcp-original", editor: "solidcraft" });
	let rpcId = 0;
	const rpc = async (method: string, params: Record<string, unknown> = {}) => {
		const response = (await page.evaluate(
			({ elementRef, pin, message }) =>
				window.craftBridge.invoke("craft.mcp", { elementRef, ...pin, message }),
			{
				elementRef,
				pin,
				message: { jsonrpc: "2.0", id: ++rpcId, method, params },
			},
		)) as { result?: Record<string, unknown>; error?: unknown };
		assert.equal(response.error, undefined, JSON.stringify(response));
		assert.ok(response.result, JSON.stringify(response));
		return response.result;
	};
	await rpc("initialize", {
		protocolVersion: "2024-11-05",
		capabilities: {},
		clientInfo: { name: "flow-like-browser-test", version: "1" },
	});
	await page.evaluate(
		({ elementRef, pin }) =>
			window.craftBridge.invoke("craft.mcp", {
				elementRef,
				...pin,
				message: { jsonrpc: "2.0", method: "notifications/initialized" },
			}),
		{ elementRef, pin },
	);
	const tools = await rpc("tools/list");
	assert.ok(Array.isArray(tools.tools));
	assert.ok(
		tools.tools.some(
			(tool) =>
				tool.name === "inspect_design" &&
				tool.annotations?.readOnlyHint === true,
		),
	);
	assert.ok(tools.tools.some((tool) => tool.name === "execute"));
	const tool = async (name: string, args: Record<string, unknown> = {}) => {
		const result = await rpc("tools/call", { name, arguments: args });
		assert.notEqual(result.isError, true, JSON.stringify(result));
		return result;
	};
	const initialInspection = await tool("inspect_design");
	assert.ok(Array.isArray(initialInspection.content));
	const initialInspectedText = initialInspection.content.find(
		(content) => content.type === "text",
	)?.text;
	assert.equal(typeof initialInspectedText, "string");
	assert.equal(JSON.parse(initialInspectedText).revision, beforeMcp.revision);
	const beforeMcpEdit = await cursor();
	await tool("execute", {
		command: "solid.box",
		params: { length: 13, width: 7, height: 5, corner: [30, 0, 0] },
	});
	assert.equal(
		(await waitEvent("dirty", undefined, beforeMcpEdit)).dirty,
		true,
	);
	const inspected = await tool("inspect_design");
	assert.ok(Array.isArray(inspected.content));
	const inspectedText = inspected.content.find(
		(content) => content.type === "text",
	)?.text;
	assert.equal(typeof inspectedText, "string");
	const inspectedDesign = JSON.parse(inspectedText);
	assert.ok(inspectedDesign.revision > (beforeMcp.revision ?? 0));
	assert.ok(
		inspectedDesign.timeline.some(
			(feature: { type: string }) => feature.type === "BoxFeature",
		),
	);
	assert.equal(
		await page.evaluate(() => document.visibilityState),
		"hidden",
		"MCP discovery, inspection, and mutation must keep the editor hidden",
	);
	await page.bringToFront();
	await foregroundPage.close();
	assert.equal(await page.evaluate(() => document.visibilityState), "visible");
	const afterMcp = await exportDesign("after-mcp");
	const agentBox = afterMcp.document.features.find(
		(feature: { type: string }) => feature.type === "box",
	);
	assert.ok(
		agentBox,
		"An MCP tool edits the document displayed by this iframe",
	);
	assert.deepEqual(
		[agentBox.length, agentBox.width, agentBox.height],
		["13", "7", "5"],
	);
	assert.deepEqual(agentBox.corner, [30, 0, 0]);
	assert.ok((afterMcp.revision ?? 0) > (beforeMcp.revision ?? 0));
	assert.equal(inspectedDesign.revision, afterMcp.revision);
	await tool("undo");
	const afterUndo = await exportDesign("after-mcp-undo");
	assert.deepEqual(
		afterUndo.document.features,
		beforeMcp.document.features,
		"MCP undo uses the visible document's history",
	);
	assert.equal(
		(
			await rpc("tools/call", {
				name: "batch",
				arguments: {
					commands: [
						{
							command: "solid.box",
							params: { length: 2, width: 2, height: 2 },
						},
						{ command: "FILE.NEW", params: {} },
					],
				},
			})
		).isError,
		true,
		"A batch cannot replace the host document",
	);
	assert.equal(
		(
			await rpc("tools/call", {
				name: "execute",
				arguments: { command: "FILE.NEW", params: {} },
			})
		).isError,
		true,
		"Command aliases cannot replace the host document",
	);
	const afterBlockedBatch = await exportDesign("after-blocked-mcp-batch");
	assert.deepEqual(
		afterBlockedBatch.document.features,
		afterUndo.document.features,
	);
	assert.equal(
		afterBlockedBatch.revision,
		afterUndo.revision,
		"The whole batch is rejected before its first valid command can run",
	);
	await page.evaluate(() => window.craftBridge.setReadOnly(true));
	await tool("inspect_design");
	await assert.rejects(
		() =>
			tool("execute", {
				command: "solid.box",
				params: { length: 1, width: 1, height: 1 },
			}),
		/Enable editing/,
	);
	const readonlyRequest = "mcp-readonly-host";
	const beforeReadonlyCall = await cursor();
	await send({
		type: "mcp-request",
		requestId: readonlyRequest,
		deadlineAtMs: Date.now() + 30_000,
		message: {
			jsonrpc: "2.0",
			id: 5000,
			method: "tools/call",
			params: {
				name: "execute",
				arguments: {
					command: "solid.box",
					params: { length: 1, width: 1, height: 1 },
				},
			},
		},
	});
	const readonlyReply = (
		await waitEvent("mcp-response", readonlyRequest, beforeReadonlyCall)
	).response;
	assert.ok(
		readonlyReply?.error ||
			(readonlyReply?.result as { isError?: boolean } | undefined)?.isError,
		"The WASM host independently refuses read-only mutations",
	);
	await page.evaluate(() => window.craftBridge.setReadOnly(false));
	const afterReadonly = await exportDesign("after-mcp-readonly");
	assert.deepEqual(
		afterReadonly.document.features,
		beforeMcp.document.features,
	);
	assert.equal(afterReadonly.revision, afterUndo.revision);
	await load("mcp-replacement");
	await waitEvent(
		"opened",
		undefined,
		await open("replacement.stl", stl, false),
	);
	await page.evaluate(() => window.craftBridge.mount());
	await assert.rejects(() => rpc("tools/list"), /document changed/i);
	console.log(
		"solidcraft: background MCP discovery, inspection and edit, same-document undo, read-only checks, and stale-session rejection verified",
	);

	for (const [name, contents, expected] of [
		["broken.solidcraft", "{}", /invalid|missing|design|document/i],
		[
			"broken.FCStd",
			"PK\\x03\\x04junk",
			/invalid|unrecogn|could not|zip|archive/i,
		],
		["broken.3dm", "not a model", /invalid|unrecogn|could not|rhino|header/i],
		[
			"external.gltf",
			JSON.stringify({
				asset: { version: "2.0" },
				buffers: [{ uri: "geometry.bin", byteLength: 36 }],
			}),
			/external|buffer|embedded|self.contained|missing/i,
		],
	] as const) {
		await load();
		const after = await open(name, encode(contents));
		assert.match(
			(await waitEvent("error", undefined, after)).error ?? "",
			expected,
		);
		assert.equal(
			await page.evaluate(
				(after) =>
					window.events.slice(after).some((event) => event.type === "opened"),
				after,
			),
			false,
			`${name} must fail without opening a partial document`,
		);
	}
	await load();
	const beforeOversized = await cursor();
	await page.evaluate(() => {
		const bytes = new ArrayBuffer(100 * 1024 * 1024 + 1);
		document.querySelector("iframe")?.contentWindow?.postMessage(
			{
				channel: "flow-like-craft",
				session: "solid-smoke",
				type: "open",
				name: "large.stl",
				bytes,
			},
			location.origin,
			[bytes],
		);
	});
	assert.match(
		(await waitEvent("error", undefined, beforeOversized)).error ?? "",
		/100 MiB/,
	);
	assert.deepEqual(errors, []);
	console.log(
		"solidcraft: malformed native file, external glTF buffer and oversized file rejected without external requests",
	);
	if (process.env.CRAFT_KEEP_SCREENSHOTS)
		console.log(`Browser screenshots: ${directory}`);
} catch (error) {
	failed = true;
	console.error(error);
	console.error(
		await page.evaluate(() =>
			JSON.stringify(window.events.map(({ bytes, ...event }) => event)),
		),
	);
	await page.screenshot({ path: join(directory, "solidcraft-failure.png") });
	console.error(`Browser failure artifacts: ${directory}`);
	throw error;
} finally {
	let closeTimeout: ReturnType<typeof setTimeout> | undefined;
	try {
		await Promise.race([
			browser.close(),
			new Promise<void>((resolve) => {
				closeTimeout = setTimeout(() => {
					browser.disconnect();
					browser.process()?.kill("SIGKILL");
					resolve();
				}, 10000);
			}),
		]);
	} finally {
		clearTimeout(closeTimeout);
	}
	server.stop(true);
	if (!failed && !process.env.CRAFT_KEEP_SCREENSHOTS)
		await rm(directory, { recursive: true, force: true });
}

// The standalone harness is complete after assertions and browser cleanup.
process.exit(0);
