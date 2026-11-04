import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";

const root = fileURLToPath(new URL("../..", import.meta.url));
const port = Number(process.env.MEETINGS_TEST_PORT ?? 4390);
const origin = `http://127.0.0.1:${port}`;
const output = resolve(
	process.env.MEETINGS_TEST_OUTPUT ?? `${tmpdir()}/flow-like-meetings-browser`,
);
await mkdir(output, { recursive: true });
const server = spawn(
	process.execPath,
	[
		"node_modules/vite/bin/vite.js",
		"--config",
		"tests/meetings-browser/vite.config.ts",
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
const errors = [];
async function click(page, text) {
	const clicked = await page.evaluate((value) => {
		const button = [...document.querySelectorAll("button")].find(
			(node) => node.textContent?.trim() === value,
		);
		button?.click();
		return !!button;
	}, text);
	assert.ok(clicked, `Missing button: ${text}`);
}
async function hasText(page, text) {
	await page.bringToFront();
	await page.waitForFunction(
		(value) => document.body.textContent?.includes(value),
		{ polling: 100 },
		text,
	);
}
async function screenshot(page, name) {
	await page.screenshot({ path: resolve(output, name), fullPage: true });
}
try {
	const deadline = Date.now() + 30_000;
	while (true) {
		if (server.exitCode !== null) throw new Error(log);
		try {
			if ((await fetch(origin)).ok) break;
		} catch {}
		if (Date.now() > deadline) throw new Error(log);
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
			"--disable-background-timer-throttling",
			"--disable-renderer-backgrounding",
		],
	});
	const host = await browser.newPage();
	host.on("pageerror", (error) => errors.push(String(error)));
	await host.setViewport({ width: 1280, height: 900 });
	await host.goto(`${origin}/?view=host`);
	await hasText(host, "Ready to join?");
	await host.evaluate(() => localStorage.removeItem("meeting-browser-check"));
	await click(host, "Join with my account");
	await hasText(host, "Meeting chat");
	const guest = await browser.newPage();
	guest.on("pageerror", (error) => errors.push(String(error)));
	await guest.setViewport({
		width: 390,
		height: 844,
		isMobile: true,
		deviceScaleFactor: 1,
	});
	await guest.goto(`${origin}/?view=guest`);
	await hasText(guest, "Ready to join?");
	await screenshot(guest, "guest-join-mobile.png");
	await click(guest, "Ask to join as a guest");
	await hasText(guest, "Waiting for the host");
	await screenshot(guest, "guest-lobby-mobile.png");
	await host.bringToFront();
	await hasText(host, "Alex Rivera");
	await screenshot(host, "host-admission-desktop.png");
	await click(host, "Admit");
	await guest.bringToFront();
	await hasText(guest, "Meeting chat");
	await screenshot(guest, "guest-meeting-mobile.png");
	assert.equal(
		await guest.evaluate(
			() => document.documentElement.scrollWidth > innerWidth,
		),
		false,
		"Mobile page overflows horizontally",
	);
	const scroll = await guest.evaluate(() => {
		const scroller = document.querySelector("[data-fixture-scroller]");
		scroller.scrollTop = scroller.scrollHeight;
		return {
			scrollTop: scroller.scrollTop,
			scrollHeight: scroller.scrollHeight,
			clientHeight: scroller.clientHeight,
		};
	});
	assert.ok(
		scroll.scrollTop > 0,
		"Tall meeting content cannot scroll inside the app shell",
	);
	await guest.type("textarea", "Thanks, I can see the meeting chat.");
	await click(guest, "Send");
	await hasText(host, "Thanks, I can see the meeting chat.");
	await click(host, "Join audio and video");
	await host.waitForSelector('button[aria-label="Turn camera on"]');
	await host.click('button[aria-label="Turn camera on"]');
	await host.waitForFunction(() =>
		[...document.querySelectorAll("video")].some(
			(video) => video.videoWidth > 0,
		),
	);
	await screenshot(host, "host-camera-desktop.png");
	await click(host, "End meeting for everyone");
	await hasText(guest, "Meeting ended");
	await host.waitForFunction(
		() => document.querySelectorAll("video").length === 0,
	);
	const schedule = await browser.newPage();
	schedule.on("pageerror", (error) => errors.push(String(error)));
	await schedule.setViewport({
		width: 390,
		height: 844,
		isMobile: true,
		deviceScaleFactor: 1,
	});
	await schedule.goto(`${origin}/?view=schedule`);
	await schedule.waitForSelector('button[aria-label="Meetings"]');
	await schedule.click('button[aria-label="Meetings"]');
	await click(schedule, "Schedule meeting");
	await schedule.waitForSelector('form[aria-label="Schedule meeting"]');
	await schedule.type("form input:not([type])", "Customer review");
	await screenshot(schedule, "schedule-mobile.png");
	await click(schedule, "Schedule meeting");
	await hasText(schedule, "Customer review");
	await hasText(schedule, "Meeting link");
	await screenshot(schedule, "scheduled-mobile.png");
	assert.deepEqual(errors, []);
	console.log(
		JSON.stringify(
			{
				guestAdmission: true,
				isolatedChat: true,
				cameraPreview: true,
				endTeardown: true,
				scheduleCreated: true,
				mobileScroll: scroll,
				screenshots: output,
			},
			null,
			2,
		),
	);
} catch (error) {
	console.error(log);
	console.error(errors);
	for (const [index, page] of ((await browser?.pages()) ?? []).entries()) {
		console.error(
			page.url(),
			await page.evaluate(() => document.body.textContent).catch(() => ""),
		);
		await screenshot(page, `failure-${index}.png`).catch(() => {});
	}
	throw error;
} finally {
	await browser?.close();
	server.kill();
}
