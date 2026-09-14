import puppeteer from "puppeteer";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
const output = path.resolve("output/pricing-ui-screenshots");
await mkdir(output, { recursive: true });
const profile = await mkdtemp(
	path.join(tmpdir(), "flow-pricing-admin-chrome-"),
);
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	userDataDir: profile,
	args: ["--no-sandbox", "--disable-gpu"],
	protocolTimeout: 60000,
});
const errors = [];
const captures = [];
try {
	const page = await browser.newPage();
	page.on("pageerror", (error) => errors.push(error.message));
	page.on("console", (message) => {
		if (message.type() === "error") console.log("browser:", message.text());
	});
	await page.setRequestInterception(true);
	page.on("request", (request) => {
		if (/^(http:\/\/127\.0\.0\.1:4333\/|data:|blob:)/.test(request.url()))
			request.continue();
		else request.abort();
	});
	await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 2 });
	await page.goto("http://127.0.0.1:4333/", {
		waitUntil: "networkidle0",
		timeout: 60000,
	});
	await page.waitForFunction(
		() => document.body.innerText.includes("Maya Chen"),
		{ timeout: 60000 },
	);
	const clickText = async (selector, text) => {
		const elements = await page.$$(selector);
		for (const el of elements) {
			if ((await el.evaluate((e) => e.textContent.trim())) === text) {
				await el.click();
				return;
			}
		}
		throw new Error(`Missing ${selector}: ${text}`);
	};
	const capture = async (name, title) => {
		await new Promise((resolve) => setTimeout(resolve, 250));
		await page.screenshot({ path: path.join(output, name), fullPage: false });
		captures.push({
			file: name,
			title,
			width: (await page.viewport()).width,
			height: (await page.viewport()).height,
		});
	};
	await capture(
		"admin-users-dark.png",
		"Admin users: all plans, including Max",
	);
	await clickText('button[role="combobox"]', "Max");
	await page.waitForSelector('[role="listbox"]', { visible: true });
	await capture("admin-tier-edit-max.png", "Admin user tier selector with Max");
	await page.keyboard.press("Escape");
	await clickText('button[role="combobox"]', "All tiers");
	await page.waitForSelector('[role="listbox"]', { visible: true });
	await capture("admin-tier-filter-max.png", "Admin tier filter with Max");
	await clickText('[role="option"]', "Max");
	await page.waitForFunction(
		() => document.querySelectorAll("tbody tr").length === 2,
	);
	await capture("admin-max-filtered.png", "Admin users filtered to Max");
	await page.goto("http://127.0.0.1:4333/?light", {
		waitUntil: "networkidle0",
	});
	await page.waitForFunction(() =>
		document.body.innerText.includes("Maya Chen"),
	);
	await capture("admin-users-light.png", "Admin users in light theme");
	await page.setViewport({ width: 390, height: 1000, deviceScaleFactor: 2 });
	await page.goto("http://127.0.0.1:4333/?light", {
		waitUntil: "networkidle0",
	});
	await page.waitForFunction(() =>
		document.body.innerText.includes("Maya Chen"),
	);
	await clickText('button[role="combobox"]', "All tiers");
	await page.waitForSelector('[role="listbox"]', { visible: true });
	await capture(
		"admin-tier-filter-mobile.png",
		"Mobile admin tier filter with Max",
	);
	await writeFile(
		path.join(output, "admin-captures.json"),
		JSON.stringify({ fixture: true, errors, captures }, null, 2),
	);
	console.log(JSON.stringify({ errors, captures }, null, 2));
	if (errors.length) process.exitCode = 1;
} finally {
	await browser.close();
	await rm(profile, { recursive: true, force: true });
}
