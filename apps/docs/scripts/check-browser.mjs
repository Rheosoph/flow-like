import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import puppeteer from "puppeteer";

const origin = process.env.DOCS_PREVIEW_URL ?? "http://127.0.0.1:4339";
const output = process.env.DOCS_SCREENSHOT_DIR ?? "/tmp/flow-like-docs-preview";
await mkdir(output, { recursive: true });
const browser = await puppeteer.launch({
	headless: true,
	executablePath: process.env.DOCS_BROWSER_EXECUTABLE,
	args: ["--no-sandbox"],
});
try {
	const page = await browser.newPage();
	const errors = [];
	const measurements = { searches: {} };
	page.on("pageerror", (error) => errors.push(error.message));
	await page.evaluateOnNewDocument(() => {
		const observer = new MutationObserver(() => {
			const catalog = document.querySelector(
				"astro-island[component-export='NodeCatalogOverview']",
			);
			if (catalog && !catalog.hasAttribute("ssr")) {
				window.__docsCatalogHydratedAt = Math.round(performance.now());
				observer.disconnect();
			}
		});
		observer.observe(document, {
			subtree: true,
			childList: true,
			attributes: true,
			attributeFilter: ["ssr"],
		});
	});
	await page.setViewport({ width: 1440, height: 1100, deviceScaleFactor: 1 });
	await page.goto(`${origin}/nodes/overview/`, { waitUntil: "networkidle0" });
	measurements.catalog = await page.evaluate(() => {
		const navigation = performance.getEntriesByType("navigation")[0];
		return {
			htmlBytes: navigation.decodedBodySize,
			responseMs: Math.round(navigation.responseEnd),
			hydrationMs: window.__docsCatalogHydratedAt,
			domContentLoadedMs: Math.round(navigation.domContentLoadedEventEnd),
			categoryColumns: getComputedStyle(
				document.querySelector(".node-category-grid"),
			).gridTemplateColumns,
		};
	});
	assert.equal(await page.$$eval(".node-card", (cards) => cards.length), 48);
	assert.ok(
		await page.$$eval(".node-category-card", (cards) => cards.length > 20),
	);
	assert.ok(
		await page.$$eval(
			"#starlight__sidebar a[href^='/nodes/']",
			(links) => links.length < 50,
		),
	);
	await page.screenshot({ path: resolve(output, "node-overview-desktop.png") });
	await page.locator(".node-directory").scroll();
	await page.screenshot({ path: resolve(output, "node-cards-desktop.png") });
	await page.click(".node-load-more");
	await page.waitForFunction(
		() => document.querySelectorAll(".node-card").length === 96,
	);
	assert.equal(await page.$$eval(".node-card", (cards) => cards.length), 96);
	const searchStarted = performance.now();
	await page.type(".node-toolbar input", "tcp server");
	await page.waitForFunction(() =>
		document
			.querySelector(".node-card-grid")
			?.textContent.includes("TCP Server"),
	);
	measurements.catalog.filterMs = Math.round(performance.now() - searchStarted);
	assert.ok(await page.$$eval(".node-card", (cards) => cards.length < 48));
	await page.goto(`${origin}/start/first-use/`, { waitUntil: "networkidle0" });
	assert.ok(
		await page.$$eval(
			".sl-markdown-content img[srcset]",
			(images) => images.length > 0,
		),
		"Screenshots need responsive sources",
	);
	assert.ok(
		await page.$$eval(".docs-image-link", (links) => links.length > 0),
		"Screenshots need a full-size link",
	);
	await page.click("button[data-open-modal]");
	await page.waitForSelector("#starlight__search input");
	await page.type("#starlight__search input", "offline access");
	await page.waitForSelector(".pagefind-ui__result-link");
	assert.ok(
		await page.$eval(
			"#starlight__search",
			(search) =>
				search.textContent.includes("Guides") &&
				search.textContent.includes("Nodes"),
		),
		"Search needs Guides/Nodes scopes",
	);
	const guideFilter = await page.$("input[type='checkbox'][value='Guides']");
	assert.ok(guideFilter, "Guides filter must be selectable");
	await guideFilter.click();
	for (const query of [
		"create an app",
		"publish a page",
		"offline access",
		"API token",
	]) {
		await page.$eval(
			".pagefind-ui__search-input",
			(input, value) => {
				input.value = value;
				input.dispatchEvent(new Event("input", { bubbles: true }));
			},
			query,
		);
		await page.waitForFunction(
			(value) => {
				const message =
					document.querySelector(".pagefind-ui__message")?.textContent ?? "";
				return (
					/^(?:\d+ results?|No results)/.test(message.trim()) &&
					message.includes(value)
				);
			},
			{},
			query,
		);
		assert.ok(
			!(await page.$eval(".pagefind-ui__message", (message) =>
				message.textContent.trim().startsWith("No results"),
			)),
			`No guide matches for ${query}`,
		);
		await page.waitForSelector(".pagefind-ui__result-link");
		const results = await page.$$eval(".pagefind-ui__result-link", (links) =>
			links.map((link) => ({
				title: link.textContent,
				path: new URL(link.href).pathname,
			})),
		);
		assert.ok(results.length > 0, `No guide results for ${query}`);
		assert.ok(
			results.every((result) => !result.path.startsWith("/nodes/")),
			`Node results leaked into Guides for ${query}`,
		);
		measurements.searches[query] = results.slice(0, 3);
	}
	await page.screenshot({ path: resolve(output, "search-guides.png") });
	await page.keyboard.press("Escape");
	await page.goto(`${origin}/topics/datascience/visualization/`, {
		waitUntil: "networkidle0",
	});
	await page.locator(".docs-chart-preview").scroll();
	await page.waitForSelector(".docs-chart-preview svg");
	assert.ok(
		await page.$$eval(".docs-chart-preview svg rect", (bars) =>
			bars.some(
				(bar) =>
					getComputedStyle(bar).fill !== "rgb(0, 0, 0)" &&
					Number(bar.getAttribute("height")) > 10,
			),
		),
		"Chart colors must be visible",
	);
	await page.screenshot({ path: resolve(output, "chart-example.png") });
	await page.goto(`${origin}/topics/genai/prompt-templates/`, {
		waitUntil: "networkidle0",
	});
	const previews = await page.$$(".docs-chart-preview");
	assert.equal(previews.length, 5);
	for (let i = 0; i < previews.length; i++) {
		await previews[i].scrollIntoView();
		await page.waitForFunction(
			(index) =>
				document
					.querySelectorAll(".docs-chart-preview")
					[index]?.querySelector("svg, canvas"),
			{},
			i,
		);
	}
	assert.equal(
		await page.$$eval(
			"h3",
			(headings) =>
				headings.filter((heading) =>
					heading.textContent.includes("Example output"),
				).length,
		),
		7,
	);
	const prompts = await page.$$eval("pre", (blocks) =>
		blocks
			.map((block) => block.textContent)
			.filter((text) => text.includes("```nivo") || text.includes("```plotly")),
	);
	assert.equal(prompts.length, 5);
	assert.ok(
		prompts.every((text) => (text.match(/```/g) ?? []).length === 2),
		"Copied prompts must retain both inner fences",
	);
	await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 });
	await page.goto(`${origin}/nodes/overview/`, { waitUntil: "networkidle0" });
	assert.ok(
		await page.evaluate(
			() => document.documentElement.scrollWidth <= innerWidth + 1,
		),
		"Catalog overflows mobile width",
	);
	await page.screenshot({ path: resolve(output, "node-overview-mobile.png") });
	await page.focus("starlight-menu-button button");
	await page.keyboard.press("Enter");
	assert.equal(
		await page.$eval("starlight-menu-button", (menu) =>
			menu.getAttribute("aria-expanded"),
		),
		"true",
	);
	assert.ok(
		await page.$eval(
			"#starlight__sidebar",
			(sidebar) => sidebar.getBoundingClientRect().width > 0,
		),
	);
	await page.screenshot({ path: resolve(output, "navigation-mobile.png") });
	await page.keyboard.press("Escape");
	assert.equal(
		await page.$eval("starlight-menu-button", (menu) =>
			menu.getAttribute("aria-expanded"),
		),
		"false",
	);
	await page.goto(`${origin}/404.html`, { waitUntil: "networkidle0" });
	assert.match(
		await page.$eval("h1", (heading) => heading.textContent),
		/Page not found/,
	);
	const noScript = await browser.newPage();
	await noScript.setJavaScriptEnabled(false);
	await noScript.goto(`${origin}/nodes/overview/`, {
		waitUntil: "networkidle0",
	});
	assert.ok(
		await noScript.$$eval("noscript li a", (links) => links.length > 1900),
		"Catalog must remain navigable without JavaScript",
	);
	assert.deepEqual(errors, [], "Unexpected browser errors");
	await writeFile(
		resolve(output, "measurements.json"),
		`${JSON.stringify(measurements, null, 2)}\n`,
	);
	console.log(`Docs browser checks passed. Screenshots: ${output}`);
} finally {
	await browser.close();
}
