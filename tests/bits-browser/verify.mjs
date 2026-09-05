import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import puppeteer from "puppeteer";
const require = createRequire(import.meta.url);
const output = "/tmp/flow-like-bits-verification";
await mkdir(output, { recursive: true });
const browser = await puppeteer.launch({
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	headless: true,
	args: ["--no-sandbox", "--disable-gpu"],
	userDataDir: `${output}/verify-${process.pid}`,
});
const page = await browser.newPage();
const errors = [];
const checks = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.setRequestInterception(true);
page.on("request", (request) =>
	/^(http:\/\/127\.0\.0\.1:4322\/|data:|blob:)/.test(request.url())
		? request.continue()
		: request.abort(),
);
const button = (text) => page.locator(`::-p-aria(${text}[role="button"])`);
async function field(label) {
	const handle = await page.evaluateHandle((label) => {
		const labels = [...document.querySelectorAll("label")];
		const found = labels.find(
			(node) =>
				node.textContent.replace("*", "").trim() === label &&
				node.getClientRects().length,
		);
		return found ? document.getElementById(found.htmlFor) : null;
	}, label);
	assert(await handle.asElement(), `Missing field ${label}`);
	return handle.asElement();
}
async function fill(label, value) {
	const input = await field(label);
	await input.click({ clickCount: 3 });
	await page.keyboard.press("Backspace");
	if (value) await input.type(value);
}
async function section(name) {
	await page.locator(`nav button::-p-text(${name})`).click();
}
async function saved() {
	await page.waitForFunction(() =>
		[...document.querySelectorAll("output")].some((el) =>
			el.textContent.includes("All changes saved"),
		),
	);
}
async function visit(query = "") {
	await page.goto(`http://127.0.0.1:4322/${query}`, {
		waitUntil: "domcontentloaded",
	});
}
async function audit(name) {
	await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
	const violations = await page.evaluate(async () =>
		(
			await window.axe.run(
				document.querySelector('[role="dialog"]') ||
					document.querySelector("main"),
				{ runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag21aa"] } },
			)
		).violations.map((v) => ({
			id: v.id,
			impact: v.impact,
			targets: v.nodes.map((n) => n.target),
		})),
	);
	await writeFile(
		`${output}/${name}-a11y.json`,
		JSON.stringify(violations, null, 2),
	);
	assert.deepEqual(violations, [], name);
	checks.push(`${name} accessibility`);
}
try {
	await page.setViewport({ width: 1440, height: 1000 });
	await visit("?custom");
	await page.waitForSelector('[role="dialog"]');
	await fill("Display name", "My research model");
	await page.select("#metadata-language", "de");
	await fill("Description", "Neue Beschreibung");
	await section("Parameters");
	await fill("Context length", "64000");
	await button("JSON").click();
	await fill("Parameters JSON", "{invalid");
	await button("Apply JSON").click();
	assert(await page.$('[role="alert"]'));
	await section("Details");
	assert(
		await page.$eval("button::-p-aria(Save changes)", (el) => el.disabled),
	);
	await section("Parameters");
	await button("Discard JSON edits").click();
	await button("Fields").click();
	await page.screenshot({ path: `${output}/parameters.png` });
	await audit("parameters");
	await button("Save changes").click();
	await saved();
	const custom = await page.evaluate(() => window.bitQa.saved);
	assert.equal(custom.meta.en.name, "My research model");
	assert.equal(custom.meta.de.description, "Neue Beschreibung");
	assert.equal(custom.parameters.context_length, 64000);
	assert.equal(custom.parameters.custom_options.retained, "do not drop");
	assert.equal(custom.parameters.provider.api_surface, null);
	checks.push(
		"custom fields, locales, unknown parameters and null API defaults round-trip",
	);
	await section("Images");
	await fill("Icon URL", "https://example.invalid/logo.png");
	await button("Remove icon").click();
	const imagePath = `${output}/upload.png`;
	await writeFile(
		imagePath,
		Buffer.from(
			"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jN1kAAAAASUVORK5CYII=",
			"base64",
		),
	);
	await (await page.$('input[aria-label="Upload icon file"]')).uploadFile(
		imagePath,
	);
	await page.waitForFunction(() =>
		document
			.querySelector('img[alt="Icon preview"]')
			?.getAttribute("src")
			?.startsWith("data:image/"),
	);
	await button("Save changes").click();
	await saved();
	assert(
		(await page.evaluate(() => window.bitQa.saved.meta.de.icon)).startsWith(
			"data:image/",
		),
	);
	checks.push("image file preparation and save");
	await page.screenshot({ path: `${output}/images.png` });
	await audit("images");
	await section("Details");
	await fill("Display name", "Unsaved name");
	await button("Close bit editor").click();
	await page.waitForSelector('[role="alertdialog"]');
	await button("Keep editing").click();
	assert.equal(
		await (await field("Display name")).evaluate((el) => el.value),
		"Unsaved name",
	);
	await button("Close bit editor").click();
	await button("Discard changes").click();
	await page.waitForFunction(() => !document.querySelector('[role="dialog"]'));
	checks.push("closing prompts and keeps or discards draft");
	await visit("?custom&fail");
	await page.waitForSelector('[role="dialog"]');
	await fill("Display name", "Retry me");
	await button("Save changes").click();
	await page.waitForSelector('[role="alert"]');
	assert.equal(
		await (await field("Display name")).evaluate((el) => el.value),
		"Retry me",
	);
	await page.evaluate(() => {
		window.bitQa.fail = false;
	});
	await button("Save changes").click();
	await saved();
	checks.push("failed custom save retains draft and retries");
	await visit();
	await page.locator("button::-p-text(Research assistant)").click();
	await page.waitForSelector('[role="dialog"]');
	await fill("Description", "Updated registry description");
	await button("Save changes").click();
	await saved();
	assert.deepEqual(
		await page.evaluate(() => window.bitQa.calls.map((c) => c.kind)),
		["metadata"],
	);
	checks.push("admin metadata saves without artifact upsert");
	await section("Parameters");
	await fill("Context length", "32000");
	await section("Details");
	await fill("Display name", "Retry registry");
	await page.evaluate(() => {
		window.bitQa.fail = true;
	});
	await button("Save changes").click();
	await page.waitForSelector('[role="alert"]');
	await page.evaluate(() => {
		window.bitQa.fail = false;
	});
	await button("Save changes").click();
	await saved();
	assert.equal(
		await page.evaluate(
			() => window.bitQa.calls.filter((c) => c.kind === "core").length,
		),
		1,
	);
	checks.push("admin retry skips completed core update");
	await button("Close bit editor").click();
	await page.waitForFunction(() => !document.querySelector('[role="dialog"]'));
	await page.screenshot({ path: `${output}/admin-library.png` });
	await audit("admin-library");
	await visit("?custom");
	await page.setViewport({ width: 390, height: 844 });
	await page.waitForSelector('[role="dialog"]');
	await section("Images");
	await page.screenshot({ path: `${output}/mobile-images.png` });
	assert(
		await page.evaluate(
			() =>
				document.querySelector('[role="dialog"]').getBoundingClientRect()
					.right <= innerWidth,
		),
	);
	assert(
		await page.evaluate(() => {
			const box = document
				.querySelector('[role="dialog"]')
				.getBoundingClientRect();
			const save = [...document.querySelectorAll("button")]
				.find((b) => b.textContent.trim() === "Save changes")
				.getBoundingClientRect();
			return save.bottom <= box.bottom;
		}),
	);
	await audit("mobile-images");
	checks.push("mobile dialog and persistent save controls");
	assert.deepEqual(errors, []);
	await writeFile(
		`${output}/results.json`,
		JSON.stringify({ checks, errors }, null, 2),
	);
	console.log(JSON.stringify({ checks, errors }, null, 2));
} finally {
	await browser.close();
}
