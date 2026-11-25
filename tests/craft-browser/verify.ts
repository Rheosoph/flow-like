import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import tailwindcss from "@tailwindcss/postcss";
import postcss from "postcss";
import puppeteer from "puppeteer";
const directory = await mkdtemp(join(tmpdir(), "flow-like-craft-browser-"));
const build = await Bun.build({
	entrypoints: [join(import.meta.dir, "main.tsx")],
	outdir: directory,
	target: "browser",
	format: "esm",
	splitting: true,
	define: { "process.env.NODE_ENV": JSON.stringify("production") },
});
assert.ok(build.success, build.logs.join("\n"));
const stylePath = resolve(import.meta.dir, "../../packages/ui/global.css");
const styles = await postcss([tailwindcss()]).process(
	await readFile(stylePath, "utf8"),
	{ from: stylePath },
);
const wav = new Uint8Array(44 + 4800 * 2);
const view = new DataView(wav.buffer);
const text = (offset: number, value: string) =>
	wav.set(new TextEncoder().encode(value), offset);
text(0, "RIFF");
view.setUint32(4, wav.length - 8, true);
text(8, "WAVEfmt ");
view.setUint32(16, 16, true);
view.setUint16(20, 1, true);
view.setUint16(22, 1, true);
view.setUint32(24, 48000, true);
view.setUint32(28, 96000, true);
view.setUint16(32, 2, true);
view.setUint16(34, 16, true);
text(36, "data");
view.setUint32(40, wav.length - 44, true);
let stored: Uint8Array = wav;
let saves = 0;
const requests: string[] = [];
const server = Bun.serve({
	hostname: "127.0.0.1",
	port: 0,
	async fetch(request) {
		const path = new URL(request.url).pathname;
		requests.push(path);
		if (path === "/")
			return new Response(
				'<!doctype html><html class="dark"><link rel="icon" href="data:,"><link rel="stylesheet" href="/style.css"><div id="app"></div><script type="module" src="/main.js"></script></html>',
				{ headers: { "Content-Type": "text/html" } },
			);
		if (path === "/style.css")
			return new Response(styles.css, {
				headers: { "Content-Type": "text/css" },
			});
		if (path === "/sample.pdf")
			return new Response(
				"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Count 0/Kids[]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF",
				{ headers: { "Content-Type": "application/pdf" } },
			);
		if (path === "/sample.wav") {
			if (request.method === "PUT") {
				stored = new Uint8Array(await request.arrayBuffer());
				saves++;
				return new Response(null, { status: 204 });
			}
			return new Response(stored, {
				headers: {
					"Content-Type": "audio/wav",
					"Cache-Control": "max-age=3600",
				},
			});
		}
		const file = Bun.file(
			path.startsWith("/craft/") || path.startsWith("/wordcraft/")
				? resolve(import.meta.dir, "../../apps/web/public", `.${path}`)
				: join(directory, path),
		);
		return (await file.exists())
			? new Response(file)
			: new Response("Missing", { status: 404 });
	},
});
const browser = await puppeteer.launch({
	headless: true,
	args: ["--enable-unsafe-swiftshader"],
	...(process.env.CHROME_PATH
		? { executablePath: process.env.CHROME_PATH }
		: {}),
});
try {
	const page = await browser.newPage();
	await page.setViewport({ width: 1280, height: 900 });
	const errors: string[] = [];
	page.on("pageerror", (e) => errors.push(String(e)));
	await page.setRequestInterception(true);
	page.on("request", (request) => {
		if (
			request.url().startsWith(server.url.origin) ||
			/^(blob|data):/.test(request.url()) ||
			// Chromium's built-in PDF reader loads shared browser resources.
			request
				.url()
				.startsWith("chrome://resources/") ||
			request
				.url()
				.startsWith("chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/")
		)
			void request.continue();
		else {
			errors.push(`Unexpected request ${request.url()}`);
			void request.abort();
		}
	});
	await page.goto(server.url.origin);
	assert.equal(
		requests.some((path) => path.startsWith("/craft/")),
		false,
	);
	await page.click("#pdf");
	await page.waitForSelector('iframe[title="PDF Preview: sample.pdf"]');
	assert.equal(
		requests.some((path) => path.startsWith("/craft/")),
		false,
		"PDF reading must not load an editor",
	);
	assert.ok(
		await page.$eval("iframe", (el) => el.src.endsWith("#toolbar=1&view=FitH")),
	);
	await page.click('[data-slot="dialog-close"]');
	await page.click("#audio");
	await page.waitForSelector("audio");
	assert.equal(
		requests.some((path) => path.startsWith("/craft/")),
		false,
	);
	await page.click('[aria-label="Edit file"]');
	await page.waitForFunction(
		() => {
			const save = document.querySelector<HTMLButtonElement>(
				'[aria-label="Save"]',
			);
			return save && !save.disabled;
		},
		{ timeout: 180000 },
	);
	assert.ok(
		requests.some((path) => /soundcraft\/craft\.wasm\.gz\./.test(path)),
	);
	const editorFrame = page
		.frames()
		.find((frame) => frame.url().includes("/craft/soundcraft/index.html"));
	assert.ok(
		editorFrame,
		"The audio editor must remain open while viewing licenses",
	);
	const editorDocument = await editorFrame.evaluateHandle(() => document);
	assert.equal(
		requests.some((path) => path.startsWith("/craft/soundcraft/licenses/")),
		false,
		"License assets load when About opens",
	);
	await page.click('[aria-label="About Flows Audio"]');
	const licenseElement = await page.waitForSelector(
		'iframe[title="Flows Audio third-party licenses and source"]',
	);
	assert.ok(licenseElement);
	const licenseFrame = await licenseElement.contentFrame();
	assert.ok(licenseFrame);
	await licenseFrame.waitForSelector("h1");
	assert.equal(
		await licenseFrame.$eval("h1", (el) => el.textContent),
		"Flows Audio: third-party licenses and source",
	);
	const licenseBounds = await licenseElement.boundingBox();
	assert.ok(
		licenseBounds && licenseBounds.height > 300 && licenseBounds.width > 500,
		"The license page needs enough space to read and use its links",
	);
	const audioInventory = await fetch(
		new URL("/craft/soundcraft/licenses/manifest.json", server.url),
	);
	assert.ok(audioInventory.ok);
	assert.equal(
		(await audioInventory.json()).packages.some((pkg: { name: string }) =>
			pkg.name.startsWith("symphonia"),
		),
		false,
		"Audio imports use platform decoding and must not bundle Symphonia",
	);
	// Word still contains MPL-covered code after Audio switches to platform codecs.
	const sourcePage = await browser.newPage();
	await sourcePage.goto(
		new URL("/wordcraft/licenses/index.html", server.url).href,
	);
	await sourcePage.waitForSelector('a[download][href^="sources/"]');
	const sourceUrl = await sourcePage.$eval(
		'a[download][href^="sources/"]',
		(el) => (el as HTMLAnchorElement).href,
	);
	assert.equal(new URL(sourceUrl).origin, server.url.origin);
	const manifestResponse = await fetch(
		new URL("/wordcraft/licenses/manifest.json", server.url),
	);
	assert.ok(
		manifestResponse.ok,
		"The runtime must serve its license inventory",
	);
	const manifest = (await manifestResponse.json()) as {
		packages: {
			selectedLicense: string;
			sourceArchive?: string;
			sourceArchiveSha256?: string;
			sourceArchiveByteLength?: number;
			sourceArchiveParts?: {
				path: string;
				sha256: string;
				byteLength: number;
			}[];
			checksum?: string;
		}[];
	};
	const sourcePackage = manifest.packages.find(
		(pkg) =>
			pkg.sourceArchive &&
			(new URL(pkg.sourceArchive, sourcePage.url()).href === sourceUrl ||
				pkg.sourceArchiveParts?.some(
					(part) => new URL(part.path, sourcePage.url()).href === sourceUrl,
				)),
	);
	assert.ok(sourcePackage?.selectedLicense.split(" AND ").includes("MPL-2.0"));
	assert.match(
		sourcePackage.sourceArchive ?? "",
		/^sources\/[\w.+-]+\.(?:crate|tar\.gz)$/,
	);
	const part = sourcePackage.sourceArchiveParts?.find(
		(entry) => new URL(entry.path, sourcePage.url()).href === sourceUrl,
	);
	if (sourcePackage.sourceArchiveParts) assert.ok(part);
	if (part) {
		assert.ok(part.path.startsWith(`${sourcePackage.sourceArchive}.part`));
		assert.match(
			part.path.slice((sourcePackage.sourceArchive?.length ?? 0) + 5),
			/^\d{3,}$/,
		);
	}
	if (sourcePackage.sourceArchiveSha256 === undefined) {
		assert.equal(sourcePackage.sourceArchiveByteLength, undefined);
		assert.equal(sourcePackage.sourceArchiveParts, undefined);
		assert.ok(sourcePackage.sourceArchive?.endsWith(".crate"));
	} else {
		assert.match(sourcePackage.sourceArchiveSha256, /^[a-f0-9]{64}$/);
		assert.ok(
			Number.isSafeInteger(sourcePackage.sourceArchiveByteLength) &&
				(sourcePackage.sourceArchiveByteLength ?? 0) > 0,
		);
	}
	const sourceChecksum =
		part?.sha256 ?? sourcePackage.sourceArchiveSha256 ?? sourcePackage.checksum;
	const sourceByteLength =
		part?.byteLength ?? sourcePackage.sourceArchiveByteLength;
	assert.match(sourceChecksum ?? "", /^[a-f0-9]{64}$/);
	const sourceResponse = await fetch(sourceUrl);
	assert.ok(
		sourceResponse.ok,
		"The MPL source download must be available locally",
	);
	const sourceBytes = Buffer.from(await sourceResponse.arrayBuffer());
	if (sourceByteLength !== undefined) {
		assert.ok(Number.isSafeInteger(sourceByteLength) && sourceByteLength > 0);
		assert.equal(sourceBytes.byteLength, sourceByteLength);
	}
	assert.equal(
		createHash("sha256").update(sourceBytes).digest("hex"),
		sourceChecksum,
		"The downloadable source must match the runtime's pinned source checksum",
	);
	await sourcePage.close();
	await page.screenshot({ path: join(directory, "license-dialog.png") });
	await page.click(
		'[role="dialog"]:has(> iframe[title="Flows Audio third-party licenses and source"]) > [data-slot="dialog-close"]',
	);
	await page.waitForSelector(
		'iframe[title="Flows Audio third-party licenses and source"]',
		{ hidden: true },
	);
	assert.ok(page.frames().includes(editorFrame));
	assert.ok(
		await editorFrame.evaluate(
			(original) => original === document,
			editorDocument,
		),
		"Closing About must preserve the loaded editor document",
	);
	await editorDocument.dispose();
	await page.click('[aria-label="Save"]');
	await page.waitForFunction(
		() => document.body.textContent?.includes("Saved"),
		{ timeout: 60000 },
	);
	assert.equal(saves, 1);
	assert.equal(new TextDecoder().decode(stored.subarray(0, 4)), "RIFF");
	await page.screenshot({ path: join(directory, "audio-editor.png") });
	await page.click('[data-slot="dialog-close"]');
	await page.click("#audio");
	await page.waitForSelector("audio");
	const reopened = await page.$eval("audio", async (el) =>
		Array.from(new Uint8Array(await (await fetch(el.src)).arrayBuffer())),
	);
	assert.deepEqual(reopened, Array.from(stored));
	assert.deepEqual(errors, []);
	console.log(
		"Browser PDF + audio previews stay lazy; About serves readable licenses and checksum-verified MPL source; Flows Audio saves to the original URL and reopens with current bytes.",
	);
} finally {
	await browser.close();
	server.stop(true);
	await rm(directory, { recursive: true, force: true });
}
