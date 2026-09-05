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
			[...document.querySelectorAll(`${selector} svg`)].some((element) => {
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
	await chart("bar")
		.getByText("No data matches these filters.", { exact: true })
		.waitFor();
	await chart("stat")
		.getByText("No data matches these filters.", { exact: true })
		.waitFor();
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
