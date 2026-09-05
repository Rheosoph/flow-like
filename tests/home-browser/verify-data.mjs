import assert from "node:assert/strict";
import { writeFile } from "node:fs/promises";
import { chromium } from "playwright-core";

const executablePath =
	process.env.CHROME_EXECUTABLE_PATH ??
	(process.platform === "darwin"
		? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
		: undefined);
const browser = await chromium.launch({
	executablePath,
	headless: true,
	args: ["--disable-dev-shm-usage", "--no-sandbox"],
});
const page = await browser.newPage({ viewport: { width: 1480, height: 1100 } });
const errors = [];
const warnings = [];
const blocked = [];
const passed = [];
page.on("pageerror", (error) => {
	errors.push(error.message);
	console.log("PAGE ERROR", error.stack);
});
page.on("console", (message) => {
	if (message.type() === "error") {
		errors.push(message.text());
		console.log("CONSOLE ERROR", message.text());
	} else if (message.type() === "warning") warnings.push(message.text());
});
await page.route("**/*", (route) => {
	const url = new URL(route.request().url());
	if (url.hostname === "127.0.0.1" || ["data:", "blob:"].includes(url.protocol))
		return route.continue();
	blocked.push(url.origin);
	return route.abort();
});
const chart = (id) => page.getByTestId(`data-${id}`);
const waitVisual = (id) =>
	page.waitForFunction(
		(selector) =>
			[
				...document.querySelectorAll(
					`${selector} svg, ${selector} [data-home-data-calendar]`,
				),
			].some((element) => {
				const box = element.getBoundingClientRect();
				return box.width > 100 && box.height > 80;
			}),
		`[data-testid="data-${id}"]`,
		{ timeout: 60_000 },
	);
const waitQuery = async (test) => {
	await page.waitForFunction(
		({ expression }) => {
			const text =
				document.querySelector('[data-testid="data-last-query"]')
					?.textContent ?? "";
			return (
				text.includes(expression) ||
				(text && JSON.parse(text).sql.includes(expression))
			);
		},
		{ expression: test },
	);
	return JSON.parse(await page.getByTestId("data-last-query").innerText());
};
try {
	await page.goto("http://127.0.0.1:4318/data-fixture", {
		waitUntil: "domcontentloaded",
	});
	await page
		.getByRole("heading", { name: "Data widgets · local fixture" })
		.waitFor({ timeout: 60_000 });
	await chart("stat")
		.getByText("840", { exact: true })
		.waitFor({ timeout: 60_000 });
	for (const id of [
		"bar",
		"stacked",
		"donut",
		"calendar",
		"sankey",
		"boxplot",
		"percentstacked",
	])
		await waitVisual(id);
	await chart("pivot").getByRole("table").waitFor();
	assert.equal(await chart("pivot").getByRole("row").count(), 4);
	await chart("records").getByText("September 2026", { exact: true }).waitFor();
	assert.ok(
		(await chart("percentstacked").innerText()).includes(
			"Limited to 2 results",
		),
	);
	assert.equal(
		await page.getByText("Data is unavailable", { exact: true }).count(),
		0,
	);
	passed.push(
		"Desktop renders real stat, bar, stacked, donut, pivot, calendar, Sankey, boxplot, percent-stacked and record calendar; truncation shown",
	);
	await page.screenshot({
		path: "/private/tmp/home-data-qa-desktop.png",
		fullPage: true,
	});
	await page.setViewportSize({ width: 390, height: 844 });
	await page.waitForFunction(
		() => document.documentElement.scrollWidth <= innerWidth + 1,
	);
	for (const id of [
		"bar",
		"stacked",
		"donut",
		"calendar",
		"sankey",
		"boxplot",
		"percentstacked",
	])
		await waitVisual(id);
	const dimensions = await page
		.locator('[data-testid^="data-"]')
		.evaluateAll((elements) =>
			elements
				.filter((element) => element.tagName === "SECTION")
				.map((element) => ({
					id: element.getAttribute("data-testid"),
					width: element.getBoundingClientRect().width,
					viewport: innerWidth,
				})),
		);
	assert.ok(dimensions.every((item) => item.width <= item.viewport));
	passed.push(
		"390px viewport fits every data card without page overflow and charts retain measurable area",
	);
	await page.screenshot({
		path: "/private/tmp/home-data-qa-mobile.png",
		fullPage: true,
	});
	await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
	await page.screenshot({
		path: "/private/tmp/home-data-qa-mobile-top-viewport.png",
	});
	await chart("sankey").evaluate((element) =>
		element.scrollIntoView({ block: "start", behavior: "instant" }),
	);
	await page.waitForFunction(() => {
		const top = document
			.querySelector('[data-testid="data-sankey"]')
			?.getBoundingClientRect().top;
		return top !== undefined && Math.abs(top) < 2;
	});
	await page.screenshot({
		path: "/private/tmp/home-data-qa-mobile-sankey-boxplot-viewport.png",
	});
	await page
		.getByTestId("data-settings")
		.evaluate((element) =>
			element.scrollIntoView({ block: "start", behavior: "instant" }),
		);
	await page.waitForFunction(() => {
		const top = document
			.querySelector('[data-testid="data-settings"]')
			?.getBoundingClientRect().top;
		return top !== undefined && Math.abs(top) < 2;
	});
	await page.screenshot({
		path: "/private/tmp/home-data-qa-mobile-settings-viewport.png",
	});
	await page.setViewportSize({ width: 1480, height: 1100 });
	const settings = page.getByTestId("data-settings");
	await settings.getByLabel("Group by", { exact: true }).selectOption("status");
	await waitQuery('GROUP BY "status"');
	await settings
		.getByLabel("Database", { exact: true })
		.selectOption("personal");
	await settings.getByLabel("Table", { exact: true }).selectOption("orders");
	await settings.getByRole("button", { name: "Filter", exact: true }).click();
	await settings
		.getByLabel("Field", { exact: true })
		.last()
		.selectOption("owner");
	await settings
		.getByLabel("Compare with", { exact: true })
		.selectOption("viewer");
	const scoped = await waitQuery('"__home_filter_0": "fixture-user"');
	assert.equal(scoped.personal, true);
	passed.push(
		"Production settings generate grouped SQL and bind actual viewer ID in personal database scope",
	);
	await settings.getByLabel("Source", { exact: true }).selectOption("ontology");
	await settings
		.getByLabel("Ontology", { exact: true })
		.selectOption("qa-ontology");
	await settings
		.getByLabel("Object type", { exact: true })
		.selectOption("Order");
	const ontology = await waitQuery('"overlay_id": "qa-ontology"');
	assert.ok(ontology.sql.includes('COUNT(DISTINCT "order_id")'));
	passed.push(
		"Ontology picker issues object-identity count on the selected ontology surface",
	);
	await settings.getByLabel("Source", { exact: true }).selectOption("query");
	await settings
		.getByLabel("Saved query or view", { exact: true })
		.selectOption("qa-query");
	await settings.getByLabel("Owner", { exact: true }).selectOption("viewer");
	const query = await waitQuery('"owner": "fixture-user"');
	assert.equal(query.personal, true);
	assert.ok(query.sql.includes("WHERE owner = $owner"));
	passed.push(
		"Saved-query picker binds the viewer parameter while preserving the saved SQL",
	);
	await page.getByLabel("Scenario", { exact: true }).selectOption("empty");
	await chart("bar").getByText("No records yet", { exact: true }).waitFor();
	await chart("stat").getByText("No records yet", { exact: true }).waitFor();
	await page.getByLabel("Scenario", { exact: true }).selectOption("error");
	await chart("bar")
		.getByText("Data is unavailable", { exact: true })
		.waitFor();
	await chart("bar")
		.getByRole("button", { name: "Try again", exact: true })
		.click();
	await chart("bar")
		.getByText("Data is unavailable", { exact: true })
		.waitFor();
	await page.getByLabel("Scenario", { exact: true }).selectOption("populated");
	await chart("stat").getByText("840", { exact: true }).waitFor();
	await waitVisual("sankey");
	passed.push(
		"Empty state, permission error, retry and successful recovery remain contained in each widget",
	);
	await page.goto("http://127.0.0.1:4318/data-fixture?all=1", {
		waitUntil: "domcontentloaded",
	});
	await page.waitForFunction(
		() => {
			const cards = [
				...document.querySelectorAll('section[data-testid^="data-"]'),
			];
			return (
				cards.length === 32 &&
				cards.every((card) =>
					card.querySelector('[data-home-data-state="ready"]'),
				)
			);
		},
		undefined,
		{ timeout: 60_000 },
	);
	for (const id of ["sankey", "heatmap", "treemap", "funnel", "graph"])
		await waitVisual(id);
	await page.waitForFunction(
		() =>
			![...document.querySelectorAll("output")].some((element) =>
				element.textContent?.includes("Loading visualization"),
			),
	);
	assert.equal(
		await page.getByText("Nothing to display", { exact: true }).count(),
		0,
	);
	assert.equal(
		await page.getByText("No numeric values to plot", { exact: true }).count(),
		0,
	);
	await page.screenshot({
		path: "/private/tmp/home-data-polished-all32-desktop.png",
		fullPage: true,
	});
	for (const width of [390, 768]) {
		await page.setViewportSize({ width, height: 844 });
		await page.waitForFunction(
			() => document.documentElement.scrollWidth <= innerWidth + 1,
		);
		const overflow = await page
			.locator('section[data-testid^="data-"]')
			.evaluateAll((cards) =>
				cards
					.filter((card) => card.getBoundingClientRect().width > innerWidth)
					.map((card) => card.getAttribute("data-testid")),
			);
		assert.deepEqual(overflow, []);
	}
	await page.setViewportSize({ width: 390, height: 844 });
	for (const id of [
		"stat",
		"calendar",
		"sankey",
		"metricstrip",
		"gauge",
		"heatmap",
		"table",
		"timeline",
	]) {
		await chart(id).evaluate((element) =>
			element.scrollIntoView({ block: "start", behavior: "instant" }),
		);
		await page.screenshot({
			path: `/private/tmp/home-data-polished-${id}-390.png`,
		});
	}
	await page.getByLabel("Scenario", { exact: true }).selectOption("empty");
	await page.waitForFunction(() =>
		[...document.querySelectorAll('section[data-testid^="data-"]')].every(
			(card) => card.querySelector('[data-home-data-state="empty"]'),
		),
	);
	const emptyHeights = await page
		.locator('section[data-testid^="data-"]')
		.evaluateAll((cards) =>
			cards.map((card) => card.getBoundingClientRect().height),
		);
	assert.ok(
		emptyHeights.every((height) => height < 220),
		`Empty cards must be compact: ${emptyHeights.join(", ")}`,
	);
	await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
	await page.screenshot({
		path: "/private/tmp/home-data-polished-empty-390.png",
	});
	passed.push(
		"All 32 data presentations render at desktop, tablet, and narrow widths; empty cards remain below 220px",
	);
	await page.goto("http://127.0.0.1:4318/data-fixture?editor=1", {
		waitUntil: "domcontentloaded",
	});
	await page.waitForFunction(
		() =>
			document.querySelectorAll(
				'[data-home-widget] [data-home-data-state="ready"]',
			).length === 32,
		undefined,
		{ timeout: 60_000 },
	);
	for (const width of [1480, 768, 390]) {
		await page.setViewportSize({ width, height: 844 });
		await page.waitForFunction(
			() => document.documentElement.scrollWidth <= innerWidth + 1,
		);
		await page.waitForFunction(() =>
			[...document.querySelectorAll("[data-home-widget]")].every((card) => {
				const root = card.querySelector("[data-home-data-state]");
				return (
					root &&
					card.getBoundingClientRect().bottom >=
						root.getBoundingClientRect().bottom - 1
				);
			}),
		);
		const height = await page
			.locator('[data-home-widget="bar"]')
			.evaluate((element) => element.getBoundingClientRect().height);
		assert.ok(
			height > 280 && height < 440,
			`Production chart frame height ${height}`,
		);
		await page.screenshot({
			path: `/private/tmp/home-data-production-${width}.png`,
			fullPage: width === 1480,
		});
	}
	await page.getByLabel("Scenario", { exact: true }).selectOption("empty");
	await page.waitForFunction(
		() =>
			document.querySelectorAll(
				'[data-home-widget] [data-home-data-state="empty"]',
			).length === 32,
	);
	await page.waitForFunction(() =>
		[...document.querySelectorAll("[data-home-widget]")].every(
			(card) => card.getBoundingClientRect().height < 230,
		),
	);
	await page.screenshot({
		path: "/private/tmp/home-data-production-empty-390.png",
	});
	await page.getByLabel("Scenario", { exact: true }).selectOption("populated");
	await page.waitForFunction(
		() =>
			document.querySelector(
				'[data-home-widget="bar"] [data-home-data-state="ready"]',
			) &&
			document.querySelector('[data-home-widget="bar"]').getBoundingClientRect()
				.height > 280,
	);
	passed.push(
		"Production HomeEditor automatically sizes all 32 real data widgets at three widths; empty frames contract and populated charts regain their plot area without clipped content",
	);
	for (const id of ["calendar", "metricstrip", "sankey"]) {
		await page
			.locator(`[data-home-widget="${id}"]`)
			.evaluate((element) =>
				element.scrollIntoView({ block: "start", behavior: "instant" }),
			);
		await page.screenshot({
			path: `/private/tmp/home-data-production-${id}-390.png`,
		});
	}
	await page
		.getByLabel("Scenario", { exact: true })
		.selectOption("unconfigured");
	await page.waitForFunction(
		() =>
			document.querySelectorAll(
				'[data-home-widget] [data-home-data-state="unconfigured"]',
			).length === 32,
	);
	await page.waitForFunction(() =>
		[...document.querySelectorAll("[data-home-widget]")].every(
			(card) => card.getBoundingClientRect().height < 230,
		),
	);
	assert.equal(
		await page.getByText("Connect your data", { exact: true }).count(),
		32,
	);
	await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
	await page.screenshot({
		path: "/private/tmp/home-data-production-unconfigured-390.png",
	});
	passed.push(
		"All 32 unconfigured production widgets show a compact source-selection hint",
	);
	assert.deepEqual(errors, []);
	assert.deepEqual(blocked, []);
	passed.push(
		"No browser page errors, console errors or attempted remote requests",
	);
} catch (error) {
	await page
		.screenshot({
			path: "/private/tmp/home-data-qa-failure.png",
			fullPage: true,
		})
		.catch(() => {});
	console.log((await page.locator("body").innerText()).slice(0, 5000));
	throw error;
} finally {
	await writeFile(
		"/private/tmp/home-data-qa-report.json",
		JSON.stringify({ passed, errors, warnings, blocked }, null, 2),
	);
	console.log(JSON.stringify({ passed, errors, warnings, blocked }, null, 2));
	await browser.close();
}
