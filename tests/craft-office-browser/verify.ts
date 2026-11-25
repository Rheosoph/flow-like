import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { strFromU8, unzipSync } from "fflate";
import { PDFDocument } from "pdf-lib";
import puppeteer from "puppeteer";
import { pdf, slides, workbook } from "./fixtures";

interface HostEvent {
	type: string;
	requestId?: string;
	readOnly?: boolean;
	dirty?: boolean;
	revision?: number;
	error?: string;
	bytes?: ArrayBuffer;
}
declare global {
	interface Window {
		events: HostEvent[];
	}
}

const directory = await mkdtemp(join(tmpdir(), "flow-like-craft-office-"));
const assets = resolve(import.meta.dir, "../../apps/web/public/craft");
const selected = process.argv.slice(2);
const runtimes = selected.length
	? selected
	: ["gridcraft", "deckcraft", "pdfcraft"];
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	async fetch(request) {
		const path = new URL(request.url).pathname;
		if (path === "/")
			return new Response(
				`<!doctype html><html><link rel="icon" href="data:,"><style>html,body,iframe{width:100%;height:100%;margin:0;border:0}</style><script>window.events=[];addEventListener('message',e=>{if(e.origin===location.origin&&e.data.channel==='flow-like-craft')events.push(e.data)});</script><body></body></html>`,
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
let failed = false;
try {
	for (const runtime of runtimes) {
		assert.ok(["gridcraft", "deckcraft", "pdfcraft"].includes(runtime));
		const format =
			runtime === "gridcraft"
				? "xlsx"
				: runtime === "deckcraft"
					? "pptx"
					: "pdf";
		const sample =
			runtime === "gridcraft"
				? workbook
				: runtime === "deckcraft"
					? slides
					: pdf();
		const page = await browser.newPage();
		await page.setViewport({ width: 1280, height: 900 });
		const errors: string[] = [];
		page.on("pageerror", (error) => errors.push(String(error)));
		page.on("response", (response) => {
			if (response.status() >= 400)
				errors.push(`HTTP ${response.status()}: ${response.url()}`);
		});
		page.on("console", (message) => {
			if (message.type() === "error") console.error(runtime, message.text());
		});
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
		const waitEvent = async (type: string, requestId?: string) => {
			await page.waitForFunction(
				(type, requestId) =>
					window.events.some(
						(event) =>
							event.type === type &&
							(!requestId || event.requestId === requestId),
					),
				{ timeout: type === "ready" ? 180_000 : 30_000 },
				type,
				requestId,
			);
			return page.evaluate(
				(type, requestId) => {
					const event = window.events.find(
						(event) =>
							event.type === type &&
							(!requestId || event.requestId === requestId),
					);
					if (!event) throw new Error("The runtime event is missing.");
					return {
						...event,
						bytes: event.bytes
							? Array.from(new Uint8Array(event.bytes))
							: undefined,
					};
				},
				type,
				requestId,
			);
		};
		const send = (data: Record<string, unknown>) =>
			page.evaluate((data) => {
				const frame = document.querySelector("iframe")?.contentWindow;
				if (!frame) throw new Error("The runtime frame is missing.");
				frame.postMessage(
					{ ...data, channel: "flow-like-craft", session: "smoke" },
					location.origin,
				);
			}, data);
		const settle = () =>
			page.evaluate(
				() =>
					new Promise<void>((resolve) =>
						requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
					),
			);
		const focusCanvas = async () => {
			const iframe = await page.$("iframe");
			assert.ok(iframe);
			const frame = await iframe.contentFrame();
			assert.ok(frame);
			await frame.evaluate(() => {
				const canvas = document.querySelector("canvas");
				if (!canvas) throw new Error("The runtime canvas is missing.");
				canvas.focus();
			});
		};
		const open = async (bytes: Uint8Array, readOnly: boolean) => {
			await page.goto(server.url.origin);
			await page.evaluate((runtime) => {
				const iframe = document.createElement("iframe");
				iframe.src = `/${runtime}/index.html?session=smoke`;
				document.body.append(iframe);
			}, runtime);
			await waitEvent("ready");
			await page.evaluate(
				({ bytes, format, readOnly }) => {
					const frame = document.querySelector("iframe")?.contentWindow;
					if (!frame) throw new Error("The runtime frame is missing.");
					frame.postMessage(
						{
							channel: "flow-like-craft",
							session: "smoke",
							type: "open",
							name: `sample.${format}`,
							bytes: Uint8Array.from(bytes).buffer,
							readOnly,
						},
						location.origin,
					);
				},
				{ bytes: [...bytes], format, readOnly },
			);
			assert.equal((await waitEvent("opened")).readOnly, readOnly);
			await settle();
		};
		const exportDocument = async (requestId: string) => {
			await send({ type: "export", requestId, format });
			const event = await waitEvent("exported", requestId);
			assert.ok(Number.isSafeInteger(event.revision));
			assert.ok(event.bytes);
			return { bytes: Uint8Array.from(event.bytes), revision: event.revision };
		};
		try {
			await open(sample, true);
			await focusCanvas();
			await page.keyboard.type("readonly must not modify document");
			await settle();
			assert.equal(
				await page.evaluate(() =>
					window.events.some((event) => event.type === "dirty" && event.dirty),
				),
				false,
			);
			await send({ type: "export", requestId: "readonly", format });
			assert.match(
				(await waitEvent("error", "readonly")).error ?? "",
				/editing/i,
			);
			await page.screenshot({
				path: join(directory, `${runtime}-preview.png`),
			});
			await send({ type: "set-read-only", readOnly: false });
			await settle();
			await focusCanvas();
			const modifier = await page.evaluate(() =>
				navigator.platform.includes("Mac") ? "Meta" : "Control",
			);
			if (runtime === "gridcraft") {
				await page.keyboard.press("Home");
				await settle();
				await page.keyboard.type("Edited in Flows Sheets", { delay: 20 });
				await settle();
				assert.equal((await waitEvent("dirty")).dirty, true);
				// Export commits the active cell draft, as the storage Save button does.
			} else {
				await page.keyboard.down(modifier);
				if (runtime === "deckcraft") await page.keyboard.down("Shift");
				await page.keyboard.press(runtime === "deckcraft" ? "KeyP" : "KeyK");
				if (runtime === "deckcraft") await page.keyboard.up("Shift");
				await page.keyboard.up(modifier);
				await settle();
				if (runtime === "pdfcraft") {
					await page.keyboard.sendCharacter("Rotate pages clockwise");
					await settle();
				} else {
					await page.keyboard.type("Duplicate Slide", { delay: 20 });
				}
				await settle();
				if (runtime === "pdfcraft") {
					await page.screenshot({
						path: join(directory, "pdfcraft-palette-selected.png"),
					});
				}
				await page.keyboard.press("Enter");
				await settle();
				assert.equal((await waitEvent("dirty")).dirty, true);
			}
			if (runtime === "pdfcraft") {
				const shortcut = async (key: string, shift = false) => {
					await page.keyboard.down(modifier);
					if (shift) await page.keyboard.down("Shift");
					await page.keyboard.press(key);
					if (shift) await page.keyboard.up("Shift");
					await page.keyboard.up(modifier);
					await settle();
				};
				const rotation = async (requestId: string) => {
					const { bytes } = await exportDocument(requestId);
					return (await PDFDocument.load(bytes)).getPage(0).getRotation().angle;
				};
				assert.equal(await rotation("before-palette-shortcut"), 90);
				await shortcut("KeyK");
				await shortcut("KeyA");
				await page.keyboard.press("Backspace");
				await settle();
				await page.screenshot({
					path: join(directory, "pdfcraft-palette-shortcut.png"),
				});
				await shortcut("KeyZ");
				assert.equal(
					await rotation("palette-shortcut"),
					90,
					"Undo must not reach the document behind the palette.",
				);
				await page.keyboard.press("Escape");
				await settle();
				await page.keyboard.press("Escape");
				await settle();
				await focusCanvas();
				await shortcut("KeyZ");
				assert.equal(await rotation("undo-shortcut"), 0);
				await shortcut("KeyZ", true);
				assert.equal(await rotation("redo-shortcut"), 90);
			}
			await page.screenshot({ path: join(directory, `${runtime}-edited.png`) });
			let exported = await exportDocument("save");
			assert.ok(exported.bytes.length > 100);
			await writeFile(join(directory, `edited.${format}`), exported.bytes);
			if (runtime !== "pdfcraft") {
				const files = unzipSync(exported.bytes);
				if (runtime === "gridcraft") {
					assert.match(
						Object.values(files)
							.map((bytes) => strFromU8(bytes))
							.join("\n"),
						/Edited in Flows Sheets/,
					);
					assert.match(strFromU8(files["xl/worksheets/sheet1.xml"]), /B2\*2/);
				} else {
					assert.equal(
						Object.keys(files).filter((name) =>
							/^ppt\/slides\/slide\d+\.xml$/.test(name),
						).length,
						2,
					);
					assert.match(
						strFromU8(files["ppt/slides/slide1.xml"]),
						/Flows Slides smoke/,
					);
				}
			} else {
				assert.equal(strFromU8(exported.bytes.subarray(0, 5)), "%PDF-");
				const document = await PDFDocument.load(exported.bytes);
				assert.equal(document.getPageCount(), 1);
				assert.equal(document.getPage(0).getRotation().angle, 90);
			}
			assert.equal(
				await page.evaluate(
					() =>
						window.events.filter((event) => event.type === "dirty").at(-1)
							?.dirty,
				),
				true,
			);
			if (runtime === "gridcraft") {
				await focusCanvas();
				await page.keyboard.press("ArrowDown");
				await settle();
				await page.keyboard.type("Newer cell draft", { delay: 20 });
				await settle();
				await send({ type: "saved", revision: exported.revision });
				await settle();
				assert.equal(
					await page.evaluate(
						() =>
							window.events.filter((event) => event.type === "dirty").at(-1)
								?.dirty,
					),
					true,
					"An acknowledgement for the previous export must preserve a newer cell draft.",
				);
				exported = await exportDocument("save-newer");
				assert.match(
					Object.values(unzipSync(exported.bytes))
						.map((bytes) => strFromU8(bytes))
						.join("\n"),
					/Newer cell draft/,
				);
			}
			await send({ type: "saved", revision: exported.revision });
			await page.waitForFunction(
				() =>
					window.events.filter((event) => event.type === "dirty").at(-1)
						?.dirty === false,
			);
			await open(exported.bytes, false);
			const reopened = await exportDocument("reopen");
			assert.ok(reopened.bytes.length > 100);
			assert.deepEqual(errors, []);
			console.log(
				`${runtime}: read-only protected, real UI edit, valid ${format} export (${exported.bytes.length} bytes), acknowledged clean, reopened`,
			);
		} catch (error) {
			failed = true;
			console.error(
				runtime,
				await page.evaluate(() =>
					window.events.map(({ bytes, ...event }) => event),
				),
			);
			await page.screenshot({
				path: join(directory, `${runtime}-failure.png`),
			});
			console.error(`Smoke artifacts: ${directory}`);
			throw error;
		} finally {
			await page.close();
		}
	}
} finally {
	await browser.close();
	server.stop(true);
	if (!failed && !process.env.KEEP_CRAFT_SMOKE)
		await rm(directory, { recursive: true, force: true });
	else console.log(`Smoke artifacts: ${directory}`);
}
