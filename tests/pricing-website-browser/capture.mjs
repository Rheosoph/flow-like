import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import puppeteer from "puppeteer";

const output = resolve(
	process.env.SCREENSHOT_OUTPUT ?? "output/pricing-ui-screenshots",
);
await mkdir(output, { recursive: true });
const profile = await mkdtemp("/private/tmp/flow-like-pricing-website-chrome-");
const browser = await puppeteer.launch({
	headless: true,
	executablePath:
		"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	args: ["--no-sandbox", "--disable-gpu"],
	userDataDir: profile,
	protocolTimeout: 30000,
});
const page = await browser.newPage();
const errors = [];
const screenshots = [];
page.on("pageerror", (error) => errors.push(error.message));
await page.setRequestInterception(true);
page.on("request", (request) => {
	if (/^(http:\/\/127\.0\.0\.1:4332\/|data:|blob:)/.test(request.url()))
		request.continue();
	else request.abort();
});

async function show(view, width, dark = false) {
	await page.setViewport({ width, height: 1000, deviceScaleFactor: 1 });
	await page.goto("http://127.0.0.1:4332/", { waitUntil: "networkidle0" });
	await page.waitForSelector("#pricing article");
	await page.evaluate(
		({ view, dark }) => {
			document.documentElement.classList.toggle("dark", dark);
			document.getElementById(view === "faq" ? "pricing" : "faq").hidden = true;
			if (view === "faq") {
				for (const details of document.querySelectorAll("#faq details"))
					details.open = true;
			}
		},
		{ view, dark },
	);
	await page.evaluate(() => document.fonts.ready);
	await new Promise((resolve) => setTimeout(resolve, 300));
	assert.equal(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth,
		),
		true,
		`${view} ${width}px fits viewport`,
	);
}

async function compareLimits(name) {
	await page.click("#pricing details > summary");
	assert.equal(
		await page.$eval("#pricing details", (element) => element.open),
		true,
	);
	assert.equal(
		await page.$$eval("#pricing tbody tr", (elements) => elements.length),
		8,
	);
	await capture(name, "#pricing details");
}

async function capture(name, selector) {
	const path = resolve(output, name);
	if (selector) {
		const element = await page.$(selector);
		assert.ok(element, `Missing capture element ${selector}`);
		await element.screenshot({ path });
	} else await page.screenshot({ path, fullPage: true });
	screenshots.push(name);
	console.log(`Saved ${name}`);
}

try {
	await show("pricing", 1600);
	const text = await page.$eval("body", (element) => element.innerText);
	for (const value of [
		"Premium",
		"Pro",
		"Max",
		"€19",
		"€59",
		"€200",
		"€190",
		"€590",
		"€2,160",
		"25 GB",
		"100 GB",
		"500 GB",
	])
		assert.ok(text.includes(value), `Pricing includes ${value}`);
	assert.equal(
		await page.$$eval("#pricing article", (elements) => elements.length),
		4,
	);
	assert.deepEqual(
		await page.$$eval("#pricing article", (elements) =>
			elements.map((element) => element.querySelectorAll("dl > div").length),
		),
		[3, 3, 3, 3],
	);
	assert.equal(
		await page.$eval("#pricing details", (element) => element.open),
		false,
	);
	await capture("website-pricing-desktop.png");
	await capture(
		"website-pricing-cards-desktop.png",
		"#pricing section > div > div:first-child",
	);
	await capture("website-free-options-desktop.png", "#pricing details + div");
	await compareLimits("website-comparison-expanded-desktop.png");
	await show("faq", 1600);
	assert.equal(
		await page.$$eval("#faq details[open]", (elements) => elements.length),
		8,
	);
	await capture("website-faq-expanded-desktop.png");
	await show("pricing", 390);
	await capture("website-pricing-mobile.png");
	await capture("website-free-options-mobile.png", "#pricing details + div");
	await compareLimits("website-comparison-expanded-mobile.png");
	await page.$eval("#pricing details > div", (element) => {
		element.scrollLeft = element.scrollWidth;
	});
	await capture("website-comparison-max-mobile.png", "#pricing details");
	await show("faq", 390);
	await capture("website-faq-expanded-mobile.png");
	await show("pricing", 1600, true);
	await capture("website-pricing-dark-desktop.png");
	await capture(
		"website-free-options-dark-desktop.png",
		"#pricing details + div",
	);
	await compareLimits("website-comparison-expanded-dark-desktop.png");
	await show("faq", 1600, true);
	await capture("website-faq-dark-desktop.png");
	await show("pricing", 390, true);
	await capture("website-pricing-dark-mobile.png");
	await compareLimits("website-comparison-expanded-dark-mobile.png");
	await show("faq", 390, true);
	await capture("website-faq-dark-mobile.png");
	assert.deepEqual(errors, []);
	await writeFile(
		resolve(output, "website-capture-results.json"),
		JSON.stringify(
			{
				source:
					"Actual PricingTiers.astro and PricingFAQ.astro with production UI and website CSS; local isolated preview",
				pricing:
					"Website displays monthly prices and annual totals together; no interval toggle exists.",
				screenshots,
				errors,
			},
			null,
			2,
		),
	);
} finally {
	const forceClose = setTimeout(() => browser.process()?.kill("SIGKILL"), 5000);
	await browser.close().finally(() => clearTimeout(forceClose));
	await rm(profile, { recursive: true, force: true });
}
