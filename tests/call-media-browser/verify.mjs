import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";

const root = fileURLToPath(new URL("../..", import.meta.url));
const port = Number(process.env.CALL_MEDIA_TEST_PORT ?? 4389);
const origin = `http://127.0.0.1:${port}`;
const output = resolve(
	process.env.CALL_MEDIA_TEST_OUTPUT ??
		`${tmpdir()}/flow-like-call-media-browser`,
);
await mkdir(output, { recursive: true });
const server = spawn(
	process.execPath,
	[
		"node_modules/vite/bin/vite.js",
		"--config",
		"tests/call-media-browser/vite.config.ts",
		"--port",
		String(port),
	],
	{ cwd: root, stdio: ["ignore", "pipe", "pipe"] },
);
let log = "";
server.stdout.on("data", (data) => {
	log += data;
});
server.stderr.on("data", (data) => {
	log += data;
});
let browser;
try {
	const deadline = Date.now() + 20_000;
	while (true) {
		if (server.exitCode !== null) throw new Error(log);
		try {
			if ((await fetch(origin)).ok) break;
		} catch {}
		if (Date.now() > deadline) throw new Error(`Preview did not start: ${log}`);
		await new Promise((done) => setTimeout(done, 100));
	}
	browser = await puppeteer.launch({
		headless: true,
		executablePath:
			process.env.CHROME_PATH ??
			"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
		args: [
			"--use-fake-device-for-media-stream",
			"--use-fake-ui-for-media-stream",
			"--autoplay-policy=no-user-gesture-required",
		],
	});
	const page = await browser.newPage();
	await page.setViewport({ width: 1280, height: 800 });
	const errors = [];
	page.on("pageerror", (error) => errors.push(String(error)));
	await page.goto(origin);
	await page.waitForFunction(() => Boolean(window.callMediaTest));
	const result = await page.evaluate(() => window.callMediaTest.run());
	assert.equal(result.streams, 3);
	assert.equal(result.videos.length, 2);
	assert.ok(result.videos.every((video) => video.muted && video.width > 0));
	assert.deepEqual(errors, []);
	await page.screenshot({
		path: resolve(output, "camera-and-screen.png"),
		fullPage: true,
	});
	await page.evaluate(() => window.callMediaTest.cleanup());
	console.log(
		JSON.stringify(
			{ ...result, screenshot: resolve(output, "camera-and-screen.png") },
			null,
			2,
		),
	);
} finally {
	await browser?.close();
	server.kill();
}
