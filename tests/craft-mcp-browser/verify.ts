import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { strToU8, zipSync } from "fflate";
import { PDFDocument } from "pdf-lib";
import puppeteer from "puppeteer";
import sharp from "sharp";
import { pdf, slides, workbook } from "../craft-office-browser/fixtures";

const directory = await mkdtemp(join(tmpdir(), "flow-like-craft-mcp-"));
const assets = resolve(import.meta.dir, "../../apps/web/public");
const formats: Record<string, string> = {
	wordcraft: "docx",
	gridcraft: "xlsx",
	deckcraft: "pptx",
	designcraft: "idml",
	vectorcraft: "svg",
	cadcraft: "dxf",
	soundcraft: "wav",
	photocraft: "png",
	filmcraft: "mp4",
	pdfcraft: "pdf",
};
const selected = process.argv.slice(2);
const runtimes = selected.length ? selected : Object.keys(formats);

async function fixture(runtime: string): Promise<Uint8Array> {
	switch (runtime) {
		case "gridcraft":
			return workbook;
		case "deckcraft":
			return slides;
		case "pdfcraft":
			return pdf();
		case "wordcraft":
			return zipSync({
				"[Content_Types].xml": strToU8(
					'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
				),
				"_rels/.rels": strToU8(
					'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
				),
				"word/document.xml": strToU8(
					'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Live MCP document</w:t></w:r></w:p><w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr></w:body></w:document>',
				),
			});
		case "designcraft":
			return zipSync({
				mimetype: [
					strToU8("application/vnd.adobe.indesign-idml-package"),
					{ level: 0 },
				],
				"designmap.xml": strToU8(
					'<Document xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging" Self="d"><idPkg:Spread src="Spreads/one.xml"/></Document>',
				),
				"Spreads/one.xml": strToU8(
					'<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging"><Spread Self="s"><Page Self="p" GeometricBounds="0 0 300 400" ItemTransform="1 0 0 1 0 0"/></Spread></idPkg:Spread>',
				),
			});
		case "vectorcraft":
			return strToU8(
				'<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><rect x="30" y="30" width="260" height="180" fill="#ff2200"/></svg>',
			);
		case "cadcraft":
			return strToU8(
				"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n30\n0\n11\n10\n21\n10\n31\n0\n0\nENDSEC\n0\nEOF\n",
			);
		case "photocraft":
			return sharp({
				create: { width: 320, height: 180, channels: 4, background: "#e66428" },
			})
				.png()
				.toBuffer();
		case "soundcraft":
		case "filmcraft": {
			const output = join(directory, `fixture.${formats[runtime]}`);
			const args =
				runtime === "soundcraft"
					? [
							"-f",
							"lavfi",
							"-i",
							"sine=frequency=440:sample_rate=48000",
							"-t",
							"1",
							"-c:a",
							"pcm_s16le",
						]
					: [
							"-f",
							"lavfi",
							"-i",
							"color=c=blue:s=320x180:r=10",
							"-t",
							"1",
							"-c:v",
							"libx264",
							"-pix_fmt",
							"yuv420p",
						];
			execFileSync("ffmpeg", ["-v", "error", ...args, "-y", output]);
			return readFile(output);
		}
		default:
			throw new Error(`Unknown runtime ${runtime}`);
	}
}

interface RpcResult {
	isError?: boolean;
	content?: { type: string; text?: string }[];
	[key: string]: unknown;
}
interface RpcResponse {
	id?: string;
	result?: RpcResult;
	error?: unknown;
}
interface HostMessage {
	type: string;
	requestId?: string;
	error?: string;
	readOnly?: boolean;
	dirty?: boolean;
	bytes?: ArrayBuffer;
	response?: RpcResponse;
}
declare global {
	interface Window {
		mcpSmokeEvents: HostMessage[];
	}
}

function resultJson(result: RpcResult): unknown {
	const text = result.content?.find((item) => item.type === "text")?.text;
	assert.ok(text, "The upstream tool must return its result");
	return JSON.parse(text);
}
function countClips(value: unknown): number {
	if (!value || typeof value !== "object") return 0;
	if (Array.isArray(value))
		return value.reduce((sum, item) => sum + countClips(item), 0);
	return Object.entries(value).reduce(
		(sum, [key, item]) =>
			sum + (key === "clip" && typeof item === "number" ? 1 : countClips(item)),
		0,
	);
}

const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	async fetch(request) {
		const path = new URL(request.url).pathname;
		if (path === "/")
			return new Response(
				'<!doctype html><html><link rel="icon" href="data:,"><style>html,body,iframe{width:100%;height:100%;margin:0;border:0}</style><script>window.mcpSmokeEvents=[];addEventListener("message",e=>{if(e.origin===location.origin&&e.source===document.querySelector("iframe")?.contentWindow&&e.data.session==="mcp-smoke")mcpSmokeEvents.push(e.data)});</script><body></body></html>',
				{ headers: { "Content-Type": "text/html" } },
			);
		const file = Bun.file(resolve(assets, `.${path}`));
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
let failed = false;
try {
	for (const runtime of runtimes) {
		assert.ok(formats[runtime], `Unsupported smoke runtime ${runtime}`);
		const page = await browser.newPage();
		await page.setViewport({ width: 1280, height: 900 });
		const errors: string[] = [];
		page.on("pageerror", (error) => errors.push(String(error)));
		await page.setRequestInterception(true);
		page.on("request", (request) => {
			if (
				request.url().startsWith(server.url.origin) ||
				/^(blob|data):/.test(request.url())
			)
				void request.continue();
			else {
				errors.push(`External request ${request.url()}`);
				void request.abort();
			}
		});
		const channel =
			runtime === "wordcraft" ? "flow-like-wordcraft" : "flow-like-craft";
		const send = (data: Record<string, unknown>) =>
			page.evaluate(
				({ data, channel }) => {
					document
						.querySelector("iframe")
						?.contentWindow?.postMessage(
							{ ...data, channel, session: "mcp-smoke" },
							location.origin,
						);
				},
				{ data, channel },
			);
		const wait = async (type: string, requestId?: string, start = 0) => {
			await page.waitForFunction(
				(type, requestId, start) =>
					window.mcpSmokeEvents
						.slice(start)
						.some(
							(item) =>
								item.type === "error" ||
								(item.type === type &&
									(!requestId || item.requestId === requestId)),
						),
				{ timeout: type === "ready" ? 180_000 : 30_000 },
				type,
				requestId,
				start,
			);
			const message = await page.evaluate(
				(type, requestId, start) => {
					const found = window.mcpSmokeEvents
						.slice(start)
						.find(
							(item) =>
								item.type === "error" ||
								(item.type === type &&
									(!requestId || item.requestId === requestId)),
						);
					if (!found) throw new Error("Missing host response");
					return {
						...found,
						bytes: found.bytes
							? Array.from(new Uint8Array(found.bytes))
							: undefined,
					};
				},
				type,
				requestId,
				start,
			);
			assert.notEqual(message.type, "error", `${runtime}: ${message.error}`);
			return message;
		};
		let nextId = 0;
		const rpc = async (
			method: string,
			params: Record<string, unknown> = {},
		) => {
			const id = `rpc-${++nextId}`;
			await send({
				type: "mcp-request",
				requestId: id,
				deadlineAtMs: Date.now() + 30_000,
				message: { jsonrpc: "2.0", id, method, params },
			});
			const response = (await wait("mcp-response", id)).response;
			assert.equal(response?.id, id);
			assert.equal(
				response.error,
				undefined,
				`${runtime}: ${JSON.stringify(response.error)}`,
			);
			assert.ok(response.result);
			return response.result;
		};
		const call = async (
			name: string,
			args: Record<string, unknown> = {},
			expectError = false,
		) => {
			const result = await rpc("tools/call", { name, arguments: args });
			assert.equal(
				Boolean(result.isError),
				expectError,
				`${runtime} ${name}: ${JSON.stringify(result)}`,
			);
			return result;
		};
		const exportBytes = async () => {
			const requestId = `export-${++nextId}`;
			await send({ type: "export", requestId, format: formats[runtime] });
			const result = await wait("exported", requestId);
			assert.ok(result.bytes?.length);
			return new Uint8Array(result.bytes);
		};
		try {
			await page.goto(server.url.origin);
			await page.evaluate(
				(path) => {
					const frame = document.createElement("iframe");
					frame.src = path;
					document.body.append(frame);
				},
				runtime === "wordcraft"
					? "/wordcraft/index.html?session=mcp-smoke"
					: `/craft/${runtime}/index.html?session=mcp-smoke`,
			);
			await wait("ready");
			const sample = await fixture(runtime);
			await page.evaluate(
				({ data, format, channel }) => {
					const bytes = new Uint8Array(data).buffer;
					document.querySelector("iframe")?.contentWindow?.postMessage(
						{
							channel,
							session: "mcp-smoke",
							type: "open",
							name: `fixture.${format}`,
							format,
							readOnly: false,
							bytes,
						},
						location.origin,
						[bytes],
					);
				},
				{ data: Array.from(sample), format: formats[runtime], channel },
			);
			await wait("opened");
			const initialized = await rpc("initialize", {
				protocolVersion: "2025-06-18",
				capabilities: {},
				clientInfo: { name: "browser-smoke", version: "1" },
			});
			assert.equal(initialized.protocolVersion, "2025-06-18");
			const listed = await rpc("tools/list");
			const tools = listed.tools as {
				name: string;
				inputSchema: Record<string, unknown>;
			}[];
			assert.ok(tools.length > 0);
			assert.ok(
				tools.every(
					(tool) =>
						typeof tool.name === "string" &&
						tool.inputSchema?.type === "object",
				),
			);
			await writeFile(
				join(directory, `${runtime}-tools.json`),
				JSON.stringify(tools, null, 2),
			);
			const names = new Set(tools.map((tool) => tool.name));
			if (runtime === "pdfcraft") {
				for (const excluded of [
					"doc_open",
					"doc_open_revision",
					"doc_create",
					"doc_close",
					"doc_save",
					"doc_combine",
					"printers",
					"printer_options",
				])
					assert.ok(
						!names.has(excluded),
						`${excluded} must not escape the live document`,
					);
			}
			const inspect = [
				"doc_inspect",
				"inspect_workbook",
				"inspect_presentation",
				"inspect_document",
				"document_inspect",
				"inspect_drawing",
				"drawing_inspect",
				"inspect_session",
				"list_commands",
				"command_list",
			].find((name) => names.has(name));
			assert.ok(
				inspect,
				`${runtime}: expected an upstream inspection tool, got ${[...names]}`,
			);
			const before = resultJson(await call(inspect));
			assert.ok(before && typeof before === "object");
			await writeFile(
				join(directory, `${runtime}-inspection.json`),
				JSON.stringify(before, null, 2),
			);

			let denied: { name: string; args: Record<string, unknown> } | undefined;
			if (runtime === "gridcraft") {
				const original = resultJson(await call("read_range", { range: "B2" }));
				await call("set_cell", { cell: "B2", input: "777" });
				const changed = resultJson(await call("read_range", { range: "B2" }));
				assert.notDeepEqual(changed, original);
				assert.match(JSON.stringify(changed), /777/);
				await call("undo");
				assert.deepEqual(
					resultJson(await call("read_range", { range: "B2" })),
					original,
				);
				denied = { name: "set_cell", args: { cell: "B2", input: "999" } };
			} else if (runtime === "photocraft") {
				const command = "image.imageRotation.90cw";
				await call("command_run", { id: command, params: {} });
				const rotated = await sharp(await exportBytes()).metadata();
				assert.deepEqual([rotated.width, rotated.height], [180, 320]);
				await call("command_run", { id: "edit.undo", params: {} });
				const restored = await sharp(await exportBytes()).metadata();
				assert.deepEqual([restored.width, restored.height], [320, 180]);
				denied = { name: "command_run", args: { id: command, params: {} } };
			} else if (runtime === "filmcraft") {
				const original = resultJson(await call("sequence_inspect"));
				const originalClips = countClips(original);
				assert.ok(originalClips > 0);
				const args = { id: "sequence.addEdit", params: { time: 127008000000 } };
				await call("command_run", args);
				assert.equal(
					countClips(resultJson(await call("sequence_inspect"))),
					originalClips + 1,
				);
				await call("command_run", { id: "edit.undo", params: {} });
				assert.equal(
					countClips(resultJson(await call("sequence_inspect"))),
					originalClips,
				);
				const beforeBin = resultJson(await call("project_inspect")) as {
					root: unknown;
				};
				await call("command_run", {
					id: "file.newBin",
					params: { name: "MCP test bin" },
				});
				assert.match(
					JSON.stringify(resultJson(await call("project_inspect"))),
					/MCP test bin/,
				);
				await call("command_run", { id: "edit.undo", params: {} });
				const afterBin = resultJson(await call("project_inspect")) as {
					root: unknown;
				};
				assert.deepEqual(afterBin.root, beforeBin.root);
				assert.doesNotMatch(JSON.stringify(afterBin), /MCP test bin/);
				denied = { name: "command_run", args };
			} else if (runtime === "pdfcraft") {
				const documents = resultJson(await call("doc_list")) as {
					documents: { doc: number }[];
				};
				assert.equal(documents.documents.length, 1);
				const doc = documents.documents[0].doc;
				assert.ok(Number.isSafeInteger(doc));
				for (const [name, args] of [
					["doc_open", { path: "second.pdf" }],
					["doc_create", {}],
					["doc_close", { doc }],
				] as const) {
					assert.match(
						JSON.stringify(await call(name, args, true)),
						/Use the document controls above the editor for file operations/,
					);
				}
				assert.deepEqual(resultJson(await call("doc_list")), documents);
				await call("page_rotate", { doc, degrees: 90 });
				assert.equal(
					(await PDFDocument.load(await exportBytes())).getPage(0).getRotation()
						.angle,
					90,
				);
				await call("edit_undo", { doc });
				assert.equal(
					(await PDFDocument.load(await exportBytes())).getPage(0).getRotation()
						.angle,
					0,
				);
				denied = { name: "page_rotate", args: { doc, degrees: 90 } };
			}
			if (denied) {
				const beforeReadOnly = resultJson(await call(inspect));
				await send({ type: "set-read-only", readOnly: true });
				await call(denied.name, denied.args, true);
				assert.deepEqual(resultJson(await call(inspect)), beforeReadOnly);
			}
			assert.deepEqual(errors, []);
			console.log(
				`${runtime}: ${tools.length} original tools discovered; live inspection${denied ? ", edit, undo and read-only rejection" : ""} verified`,
			);
		} catch (error) {
			await page.screenshot({
				path: join(directory, `${runtime}-failure.png`),
			});
			await writeFile(
				join(directory, `${runtime}-events.json`),
				JSON.stringify(
					await page.evaluate(() => window.mcpSmokeEvents),
					null,
					2,
				),
			);
			throw error;
		} finally {
			let timer: ReturnType<typeof setTimeout> | undefined;
			await Promise.race([
				page.close(),
				new Promise<void>((resolve) => {
					timer = setTimeout(resolve, 1000);
				}),
			]);
			clearTimeout(timer);
		}
	}
} catch (error) {
	failed = true;
	throw error;
} finally {
	// Bound teardown of the browser process owned by this test after its targets close.
	const cleanup = setTimeout(() => {
		browser.process()?.kill("SIGKILL");
		browser.disconnect();
	}, 10_000);
	try {
		await browser.close();
	} finally {
		clearTimeout(cleanup);
	}
	server.stop(true);
	if (failed || process.env.CRAFT_KEEP_SCREENSHOTS)
		console.log(`MCP browser artifacts: ${directory}`);
	else await rm(directory, { recursive: true, force: true });
}

process.exit(0);
